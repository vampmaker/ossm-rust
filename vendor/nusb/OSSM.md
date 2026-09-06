Patched `nusb` 0.2.7 for OSSM.

Android/Termux `usbfs` rejects `mmap` of the USB device fd (`EPERM`). Upstream logs
`WARN Failed to allocate zero-copy buffer` on every `Endpoint::allocate` (interrupt +
bulk IN, 2 in-flight each). After the first `EPERM`/`EACCES`/`EINVAL`, this tree
disables mmap for the device and uses heap `Buffer::new` with no warning.
