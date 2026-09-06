"""Playwright `navigator.serial` bridge onto a host serial device (pyserial)."""

from __future__ import annotations

import asyncio
import base64
import errno
import json
import os
import uuid

import serial
from playwright.async_api import Page


def _is_pty_device(device: str) -> bool:
    norm = device.replace("\\", "/")
    if "/dev/pts" in norm or norm.startswith("/dev/pts") or "/pts/" in norm:
        return True
    try:
        return os.stat(device).st_rdev != 0 and "pts" in os.path.realpath(device)
    except OSError:
        return False


def _swallow_modem_ioctl(fn) -> None:
    try:
        fn()
    except (OSError, serial.SerialException) as e:
        msg = str(e)
        err = getattr(e, "errno", None)
        if err in (errno.ENOTTY, errno.EINVAL) or "ENOTTY" in msg or "Inappropriate ioctl" in msg:
            return
        raise


class WebSerialBridge:
    def __init__(self, page: Page, auto_select_port: str | None = None, *, dtr: bool = False, rts: bool = False):
        self.page = page
        self.auto_select_port = auto_select_port
        self.dtr = dtr
        self.rts = rts
        self.ports: dict = {}
        self.read_tasks: dict = {}

    async def setup(self) -> None:
        await self.page.expose_binding("_ws_requestPort", self._handle_requestPort)
        await self.page.expose_binding("_ws_getPorts", self._handle_getPorts)
        await self.page.expose_binding("_ws_open", self._handle_open)
        await self.page.expose_binding("_ws_close", self._handle_close)
        await self.page.expose_binding("_ws_write", self._handle_write)
        await self.page.expose_binding("_ws_setSignals", self._handle_setSignals)

        mock_script = """
        (() => {
            class SerialPort {
                constructor(id, info) {
                    this._id = id;
                    this._info = info;
                    this.readable = null;
                    this.writable = null;
                    this._open = false;
                }
                getInfo() { return this._info; }
                async open(options) {
                    if (this._open) return;
                    await window._ws_open({ id: this._id, options });
                    this._open = true;
                    this._readController = null;
                    this.readable = new ReadableStream({
                        start: (controller) => {
                            this._readController = controller;
                            window._ws_ports[this._id] = this;
                        },
                        cancel: async () => {
                            if (this._readController) {
                                try { this._readController.close(); } catch (_) {}
                                this._readController = null;
                            }
                        }
                    });
                    this.writable = new WritableStream({
                        write: async (chunk) => {
                            const binary = Array.from(chunk).map(b => String.fromCharCode(b)).join('');
                            await window._ws_write({ id: this._id, data: btoa(binary) });
                        },
                        close: async () => {}
                    });
                }
                async close() {
                    this._open = false;
                    if (this._readController) {
                        try { this._readController.close(); } catch (_) {}
                        this._readController = null;
                    }
                    this.readable = null;
                    this.writable = null;
                    delete window._ws_ports[this._id];
                    await window._ws_close({ id: this._id });
                }
                async setSignals(signals) {
                    await window._ws_setSignals({ id: this._id, signals });
                }
            }
            window._ws_ports = {};
            window._ws_data_received = (id, base64) => {
                const port = window._ws_ports[id];
                if (port && port._readController) {
                    const binaryString = atob(base64);
                    const bytes = new Uint8Array(binaryString.length);
                    for (let i = 0; i < binaryString.length; i++) bytes[i] = binaryString.charCodeAt(i);
                    port._readController.enqueue(bytes);
                }
            };
            window._ws_stream_closed = (id, reason) => {
                const port = window._ws_ports[id];
                if (!port) return;
                port._open = false;
                if (port._readController) {
                    try { port._readController.close(); } catch (_) {}
                    port._readController = null;
                }
                port.readable = null;
                port.writable = null;
            };
            Object.defineProperty(navigator, 'serial', {
                value: {
                    requestPort: async (options) => {
                        const res = await window._ws_requestPort(options);
                        if (!res) throw new DOMException("No port selected by the user.", "NotFoundError");
                        return new SerialPort(res.id, res.info);
                    },
                    getPorts: async () => {
                        const ports = await window._ws_getPorts();
                        return ports.map(p => new SerialPort(p.id, p.info));
                    }
                },
                writable: true,
                configurable: true
            });
        })();
        """
        await self.page.add_init_script(mock_script)

    async def _handle_requestPort(self, source, options):
        if not self.auto_select_port:
            return None
        port_id = str(uuid.uuid4())
        self.ports[port_id] = {"device": self.auto_select_port, "serial": None}
        return {"id": port_id, "info": {"usbVendorId": 0x303A, "usbProductId": 0x1001}}

    async def _handle_getPorts(self, source):
        return [{"id": pid, "info": {"usbVendorId": 0x303A, "usbProductId": 0x1001}} for pid in self.ports]

    def _open_serial(self, device: str, baud_rate: int) -> serial.Serial:
        ser = serial.Serial()
        ser.port = device
        ser.baudrate = baud_rate
        ser.timeout = 0.1
        ser.write_timeout = 2.0
        ser.rtscts = False
        if _is_pty_device(device):
            # PTYs reject TIOCMSET (ENOTTY). dsrdtr=True skips pyserial DTR on open.
            ser.dsrdtr = True
            ser.open()
            return ser
        ser.dsrdtr = False
        ser.dtr = self.dtr
        ser.rts = self.rts
        ser.open()
        _swallow_modem_ioctl(lambda: setattr(ser, "dtr", self.dtr))
        _swallow_modem_ioctl(lambda: setattr(ser, "rts", self.rts))
        return ser

    async def _handle_open(self, source, args):
        port_id = args.get("id")
        options = args.get("options", {})
        baud_rate = options.get("baudRate", 115200)
        if port_id not in self.ports:
            raise Exception("Port not found")
        device = self.ports[port_id]["device"]
        print(f"WebSerialBridge: opening {device} baud={baud_rate} dtr={self.dtr} rts={self.rts}", flush=True)
        try:
            ser = await asyncio.to_thread(self._open_serial, device, baud_rate)
        except Exception as e:
            raise Exception(f"NotFoundError: Failed to connect: {e}") from e
        self.ports[port_id]["serial"] = ser
        self.read_tasks[port_id] = asyncio.create_task(self._read_loop(port_id))

    async def _notify_stream_closed(self, port_id: str, reason: str = "disconnect") -> None:
        try:
            reason_js = json.dumps(reason)
            await self.page.evaluate(
                f"window._ws_stream_closed && window._ws_stream_closed('{port_id}', {reason_js})"
            )
        except Exception:
            pass

    async def _read_loop(self, port_id: str):
        ser = self.ports[port_id].get("serial")
        try:
            while True:
                if ser is None or not ser.is_open:
                    break
                try:
                    data = await asyncio.to_thread(ser.read, 1024)
                    if data:
                        b64 = base64.b64encode(data).decode("ascii")
                        try:
                            await self.page.evaluate(
                                f"window._ws_data_received('{port_id}', '{b64}')"
                            )
                        except Exception:
                            break
                    await asyncio.sleep(0.01)
                except asyncio.CancelledError:
                    raise
                except (serial.SerialException, OSError) as e:
                    print(f"WebSerialBridge: disconnect: {e}", flush=True)
                    self.ports[port_id]["serial"] = None
                    await self._notify_stream_closed(port_id)
                    break
        except asyncio.CancelledError:
            pass

    async def _handle_close(self, source, args):
        port_id = args.get("id")
        if port_id not in self.ports:
            return
        if port_id in self.read_tasks:
            task = self.read_tasks.pop(port_id)
            task.cancel()
            try:
                await task
            except asyncio.CancelledError:
                pass
        ser = self.ports[port_id].get("serial")
        if ser and ser.is_open:
            await asyncio.to_thread(ser.close)
        self.ports[port_id]["serial"] = None

    async def _handle_write(self, source, args):
        port_id = args.get("id")
        ser = self.ports.get(port_id, {}).get("serial")
        if ser is None or not ser.is_open:
            raise Exception("NetworkError: Serial port is not open")
        data = base64.b64decode(args.get("data"))
        await asyncio.to_thread(ser.write, data)
        await asyncio.to_thread(ser.flush)

    async def _handle_setSignals(self, source, args):
        port_id = args.get("id")
        signals = args.get("signals", {})
        ser = self.ports.get(port_id, {}).get("serial")
        if not ser or not ser.is_open:
            return

        def apply_signals():
            if "dataTerminalReady" in signals:
                _swallow_modem_ioctl(lambda: setattr(ser, "dtr", signals["dataTerminalReady"]))
            if "requestToSend" in signals:
                _swallow_modem_ioctl(lambda: setattr(ser, "rts", signals["requestToSend"]))

        await asyncio.to_thread(apply_signals)
