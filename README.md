# OSSM-Rust — User Manual

[**English**](README.md) | [**🇨🇳 简体中文**](README.zh.md)

> [!TIP]
> 🇨🇳 **中文读者**：本文档提供完整中文说明，请直接参阅 [**简体中文版用户手册 (README.zh.md)**](README.zh.md)。

OSSM-Rust drives an Open Source Sex Machine built around a **57AIM30** servo motor. It gives you a web interface with speed, depth, and stroke-shape controls, plus custom motion curves and recorded sequences.

There are two ways to run it. Pick one and follow that section — you do not need the other.

Building on the code, or looking for the HTTP / WebSocket / BLE API? See the [Developer Guide](DEVELOPERS.md).

> [!CAUTION]
> This machine moves with real force. Before every session, make sure nothing and nobody is in the path of travel, and keep the power switch within reach. The motor always starts **paused** and always finds its own travel limits at startup — let it finish that first slow sweep before using it.

---

## 1. Choose your setup

| | **A — Computer or phone** | **B — ESP32 (standalone)** |
| --- | --- | --- |
| **What runs it** | A Linux PC or an Android phone running Termux | A small WiFi board wired to the motor |
| **Extra hardware** | USB-RS485 adapter | ESP32-C6 or ESP32-S3 board + MAX3485 module |
| **Needs a computer nearby?** | Yes, it *is* the computer | No — it runs on its own once configured |
| **Setup effort** | Low: plug in, run one command | Medium: flash firmware, enter WiFi details |
| **Control from** | `http://127.0.0.1:8080` | Any phone or PC on your WiFi |

**Setup B is the best choice for normal use** — it is the only one that keeps working when you close your laptop. Setup A is good for trying things out or if you would rather not solder or flash anything.

Both use the same motor, the same power supply, and the same plugs.

> [!TIP]
> **The two motor connectors**
> - **Front, 6-pin green (KF2EDGK-3.81 6P)** — 24 V power for the shaft and internal driver.
> - **Back, 10-pin white (PHB2.0 2×5P)** — 5 V power for the isolated RS-485 transceiver and the RS-485 data pair.
>
> **Never put 24 V into the back connector.** It will destroy the drive electronics.

The standard **`57AIM30`** is recommended — the `57AIM30H` uses the same communication protocol, but has different RPM and torque ratings.

---

## 2. Setup A — Computer or Android phone

No microcontroller, no firmware. A USB adapter connects your machine directly to the motor.

### What you need

| Item | Notes |
| --- | --- |
| Linux PC, or Android phone with [Termux](https://f-droid.org/packages/com.termux/) | Install Termux and Termux:API from **F-Droid** (do NOT use Google Play). Uses the `ossm-std-linux-arm64` download |
| USB-RS485 adapter | Must provide a 5 V / VCC pin (to power the motor's isolated RS-485 transceiver) and handle send/receive switching **automatically**. Cheap "loopback" adapters and ones needing manual direction control will not work. |
| 57AIM30 motor | Standard model (`57AIM30H` has different RPM/torque) |
| 24 V DC power supply | Sized for your mechanical load |
| Cables | DC 5.5×2.1/2.5 mm barrel jack, KF2EDGK-3.81 6P, PHB2.0 2×5P |

### Wiring

<div align="center">
  <img src="./assets/wiring_diagram_std.svg" alt="USB-RS485 wiring, no ESP32" width="100%"/>
</div>

| Motor | → | Other end |
| --- | --- | --- |
| Front **+V** / **GND** | → | 24 V supply **+** / **−** |
| Back pin **10 (5V)** / pin **6 (COM)** | → | USB-RS485 **5V** / **GND** |
| Back pin **2 (485A)** / pin **3 (485B)** | → | USB-RS485 **A** / **B** |

> [!CAUTION]
> Double-check 24 V polarity before switching on, and make sure it goes to the **front** connector only.

### Run it

Download `ossm-std-linux-x64` (or `-arm64`, `-armel`, `-macos-arm64`, `-win-x64.exe`) from **Releases**.

```bash
chmod +x ossm-std-linux-x64

# Find your adapter, usually /dev/ttyUSB0
ls /dev/ttyUSB*

./ossm-std-linux-x64 --serial /dev/ttyUSB0
```

Then open **http://127.0.0.1:8080**.

To try the interface without a motor connected, run `./ossm-std-linux-x64 --mock`.

Useful options:

| Option | Effect |
| --- | --- |
| `--serial <device>` | Which port the adapter is on |
| `--bind 0.0.0.0:8080` | Let other devices on your network open the interface (default is this machine only) |
| `--config <file>` | Where settings are saved (default `ossm-config.json` in the current folder) |
| `--mock` | Interface only, no motor |

### On Android (Termux)

> [!IMPORTANT]
> **Install from F-Droid, not Google Play.** The Google Play Store build of Termux is deprecated and unmaintained. Always install both **[Termux](https://f-droid.org/packages/com.termux/)** and the **[Termux:API](https://f-droid.org/packages/com.termux.api/)** companion app from **F-Droid** (or GitHub Releases). Note that installing the command-line package (`pkg install termux-api`) alone is not sufficient; the Android companion app is required to grant USB access.

Then:

```bash
pkg install termux-api
termux-usb -l                                   # lists e.g. /dev/bus/usb/001/002
# motor (firmware already in rs485, or a USB-RS485 adapter with automatic DE/RE):
./ossm-std-linux-arm64 --serial termux-usb:/dev/bus/usb/001/002 --bind 0.0.0.0:8080
# flash merged firmware (USB-Serial-JTAG), then UART CLI:
./ossm-std-linux-arm64 --mode flash --serial termux-usb:/dev/bus/usb/001/002 --image ossm-esp32c6.bin
./ossm-std-linux-arm64 --mode console --serial termux-usb:/dev/bus/usb/001/002 --send 'get shell'
./ossm-std-linux-arm64 --mode console --serial termux-usb:/dev/bus/usb/001/002 --exit-rs485
```

The `termux-usb:` prefix is what triggers a `termux-usb` permission re-exec (tap Allow). A bare `/dev/bus/usb/…` path is opened as usbfs and does **not** re-exec. USB-Serial-JTAG (the ESP32) is not an auto-DE/RE RS-485 adapter — use `--mode flash` / `--mode console` / `--exit-rs485` on that chip, and `--serial` motor mode only with an adapter that turns the bus around by itself. Binding to `0.0.0.0` lets you open the interface from a laptop on the same WiFi. From the PC: `./scripts/test_termux.py` (SSH host `termux`).

---

## 3. Setup B — ESP32 (standalone, WiFi and Bluetooth)

The board holds the firmware and joins your WiFi, so you can control the machine from any phone or computer on the network without a host running.

### What you need

| Item | Notes |
| --- | --- |
| ESP32-C6 or ESP32-S3 dev board with USB-C | Many boards have **two** USB-C ports — see the note below |
| MAX3485 TTL module | Converts the board's serial pins to RS-485 |
| 57AIM30 motor | Standard model (`57AIM30H` has different RPM/torque) |
| 24 V DC power supply | Motor shaft power |
| 5 V USB charger | Powers the ESP32 once it is set up |
| Cables | DC barrel jack, KF2EDGK-3.81 6P, PHB2.0 2×5P, jumper wires |

> [!IMPORTANT]
> **Which USB port?** If your board has two, one is labelled **USB / Native / JTAG** and the other **UART / COM**.
> Use **USB / Native / JTAG** for everything in this guide — flashing, configuration, and the command line. The UART/COM port only prints logs and cannot flash from a browser.

### Wiring

<div align="center">
  <img src="./assets/wiring_diagram.svg" alt="ESP32 and MAX3485 wiring" width="100%"/>
</div>

**Motor power (front 6-pin)**

| Motor | → | 24 V supply |
| --- | --- | --- |
| **+V** | → | **+** |
| **GND** | → | **−** |

**Data and RS-485 interface (back 10-pin, MAX3485, ESP32)**

| From | → | To | Why |
| --- | --- | --- | --- |
| Motor **485A** | → | MAX3485 **A** | Data pair |
| Motor **485B** | → | MAX3485 **B** | Data pair |
| ESP32 **5V** (or VBUS/VIN) | → | Motor **5V** | RS-485 transceiver power |
| ESP32 **GND** | → | Motor **COM / GND** | Shared ground |
| ESP32 **3.3V** | → | MAX3485 **VCC** | Module power |
| ESP32 **GND** | → | MAX3485 **GND** | Module ground |
| ESP32 **GPIO 18** | → | MAX3485 **DI** | Board sends |
| ESP32 **GPIO 19** | → | MAX3485 **RO** | Board receives |
| ESP32 **GPIO 20** | → | MAX3485 **DE** *and* **RE** (tie both together) | Direction control |

> [!WARNING]
> The serial connection is **crossed**: the board's TX goes to **DI**, its RX goes to **RO**.
> Some modules label these TXD and RXD instead, where **TXD is actually RO** and **RXD is actually DI**. Do not wire the board's TX to a pin marked TXD.

GPIO 18 / 19 / 20 are the factory defaults. If your board uses those pins for something else — on many ESP32-S3 boards 19 and 20 are the USB data lines — pick different pins and enter them during configuration in the next step.

### Flash the firmware

You do not need to install any tools. The release package includes a browser flasher.

1. Download and extract the latest release `.zip` from **Releases**.
2. Plug the board into your computer with a USB-C **data** cable, on the **USB / Native / JTAG** port.
3. Open `flasher.html` in **Chrome** or **Edge** — desktop only; Safari and Firefox cannot talk to USB devices. Double-clicking the file is fine.
4. Click **Connect** and pick your board's port.
   > If several ports are listed, unplug the board, click **Connect** again to see which one disappeared, plug it back in, and choose the one that returns.
5. Under **Flash Firmware**, drop in the image for your chip — `ossm-esp32c6.bin` or `ossm-esp32s3.bin`. Leave the address at `0x0`.
6. Click **Flash Firmware** and wait for **Flash complete** in the Console panel.

The board resets itself and stays connected, so you can go straight to configuration.

### Enter your settings

Stay on the same page. Fill in the **Device Configuration** panel on the right:

| Section | What to enter |
| --- | --- |
| **WiFi** | Tick **Enable WiFi**, then your network name and password. Leave it off if you only want Bluetooth or USB control. |
| **GPIO Pins** | TX, RX, and DE/RE — `18`, `19`, `20` unless you wired it differently. |
| **Motor / Modbus** | Leave every timing value at `0`. Those are for troubleshooting unusual hardware. |
| **BLE** | Leave Bluetooth enabled unless you have a reason not to. |

Click **Send Configuration**. The tool restarts the board, sends the settings, and then switches to the **Serial** panel automatically. Watch for the WiFi connection message and note the **IP address** it prints.

Your settings are remembered in the browser, so a second board is quicker to set up. **Reset to defaults** clears the form.

If configuration times out, check that you are on the Native USB port, click **Connect**, and try **Send Configuration** again.

### Open the interface

Browse to the IP address from the serial log, for example `http://192.168.1.123`, or try `http://ossm.local` if your network supports it.

> [!NOTE]
> **First run after wiring the motor.** On every startup the board talks to the servo and finds its travel limits. If the motor does not answer, the firmware searches other communication speeds. If it finds the motor on a different speed it corrects the drive and asks you — in the serial log — to unplug the motor's **24 V** supply for about three seconds and plug it back in.

---

## 4. Using the controls

The interface is the same in both setups, minus the WiFi and hardware pages where they do not apply.

**Main controls**

- **Speed (BPM)** — strokes per minute.
- **Depth** — how much of the travel range is used, from a short stroke to the full range.
- **Anchor** — whether a shortened stroke sits at the near end or the far end of the range.
- **Reverse** — mirrors the motion.
- **Pause / Resume** — pause holds the machine still, either where it is or at a position you choose.

**Motion shapes**

| Shape | Feel |
| --- | --- |
| **Sine** | Smooth and even |
| **Thrust** | Quick push, slower return. The sharpness slider sets how abrupt. |
| **Spline** | Your own curve, drawn with draggable points |
| **Macro** | A saved sequence of changes over time |
| **Funscript** | Plays a standard `.funscript` file |

**Spline** lets you place points between 0 (fully retracted) and 1 (fully extended); the machine passes smoothly through all of them. Some starting points:

| Pattern | Points |
| --- | --- |
| Simple stroke | `0 1` |
| Thrust with stepped return | `0 0 1 0.8 0.5 0.2` |
| Gentle triangle | `0 0.2 0.4 0.6 0.8 1.0 0.8 0.6 0.4 0.2` |
| Square-ish hold | `0 0 0 0 0 1 1 1 1 1` |
| Vibration | `0 0.2 0.1 0.4 0.3 0.6 0.5` |

**Macros** are timed sequences — for example, start slow, speed up after 15 seconds, stop after a minute. Build one in the Macro panel, save it, and play it back with optional looping. Macros can be exported and imported as files, so they are easy to share.

**Position display** — a diagram showing the full travel range, your active stroke window, and a live marker for where the machine is. Toggle it with the activity icon in the header; the choice is remembered.

**Language** — the interface, the flasher, and the motor tool all support English and 中文 via the header toggle.

---

## 5. Controlling it over USB

Any serial terminal at **115200 baud** works, as does the **Serial** panel in `flasher.html`. This is mainly useful for setup and troubleshooting.

Settings are read and written with `get` and `set` on dotted names. Type `paths` to list everything the device supports.

```
set net.ssid "MyNetwork"        # WiFi name
set net.password secret         # WiFi password
set net.wifi_enabled true

set pin.modbus_tx 18            # change a GPIO
get pin                         # show all hardware settings

set motor.bpm 45
set motor.depth 0.8
set motor.wave_func spline
set motor.spline_points 0 0.5 1
set motor.paused false

reset                           # restart the board
```

Each successful `set` replies with `<name> set to <value>`. `get pin`, `get net`, and `get motor` print a whole section at once.

The desktop version (setup A) accepts the same `motor.*` commands by typing into the terminal where it is running. It has no `pin.*` or `net.*` settings.

The full list of settings, along with the HTTP and Bluetooth APIs, is in the [Developer Guide](DEVELOPERS.md#7-api-reference).

---

## 6. Troubleshooting

**The interface loads but the machine does not move.** It boots paused — press Resume. If the position readout stays at zero, the board is not reaching the motor; check the section below.

**The motor never responds / startup homing never finishes.**
- **Check A / B and TX / RX:** Ensure A and B are not swapped. On the MAX3485 transceiver, confirm the ESP32's TX connects to **DI** and RX connects to **RO** (beware of modules where pins are misleadingly labelled TXD/RXD).
- **Check the two power domains:** The motor contains two separate, independent power domains: the main 24 V motor & internal driver domain, and the isolated 5 V RS-485 transceiver domain. (The internal driver logic is powered entirely from the 24 V input on the front terminal — it operates normally even with no PHB2.0 cable connected; the 5 V pin on the back terminal only powers the isolated RS-485 transceiver and optocoupler circuit). Note that RS-485 is differential, so communication itself does not require grounding across systems. Make sure each domain forms its own complete circuit:
  - **Driver domain (front 6-pin connector):** Connect **+V** and **GND** to your 24 V power supply to power the motor and internal driver.
  - **RS-485 transceiver domain (back 2×5-pin connector):** Connect **Pin 10 (5V)** and **Pin 6 (COM)** to your 5 V power source (from the USB-RS485 adapter, or ESP32 5 V / step-down converter) to power the communication transceiver.
  - **Grounding between domains:** GND (front) and COM (back) can either be isolated or shared. If using a 24 V → 5 V step-down buck converter to power the ESP32 or 5 V rail from the main supply, you can simply share ground between the 24 V and 5 V rails.
- **On the ESP32:** Confirm the configured GPIO numbers match how you actually wired the board.

**Nothing moves after the motor was powered separately.** Check the serial log for a message about a communication-speed mismatch, then power-cycle the motor's 24 V supply for three seconds.

**The browser flasher cannot see the board.** Use Chrome or Edge on a desktop, use a USB-C cable that carries data (not charge-only), and use the **USB / Native / JTAG** port.

**The desktop version starts but the motor is silent.** The adapter is the usual culprit: it must switch between sending and receiving automatically. Adapters that echo what they transmit, or that expect the host to toggle a direction pin, are not supported.

**`http://ossm.local` does not resolve.** Not every network supports that name. Use the numeric IP from the serial log.

**No WiFi after configuration.** Re-check the network name and password (both are case sensitive), and confirm the network is 2.4 GHz — these boards do not support 5 GHz.

---

## 7. Going further

- [Developer Guide](DEVELOPERS.md) — architecture, design decisions, build instructions, and the complete HTTP / WebSocket / Bluetooth API.
- `motor-control.html` in the release package — a diagnostic tool that reads and writes the 57AIM30 drive's own registers, styled after the vendor's utility. It connects through a USB-RS485 adapter, or through an ESP32 with its operating mode set to `rtu_relay`.
- `ossm-wasm.html` in the release package — an all-in-one browser control page that runs the engine locally using WebAssembly. Connects via Web Serial or WebSocket relay, and can be opened directly by double-clicking the file in Chrome or Edge (`file://` supported).
- [简体中文用户手册 (README.zh.md)](README.zh.md) — 完整中文版使用与配置指南
