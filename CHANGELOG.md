# Changelog

What each release brings.

## 0.1.0 — 2026-11-14

The first release: the D-Bus contract between a QEMU `-display dbus` host and a guest agent,
over one virtio-serial port, with the zbus proxies of each interface.

- `fr.ameleo.Agent1`: version, channels, session lock state, reset.
- `fr.ameleo.Agent.Mouse1`: absolute pointer on several heads, buttons, both wheels.
- `fr.ameleo.Agent.Clipboard1` / `fr.ameleo.Host.Clipboard1`: text and images, on demand.
- `fr.ameleo.Agent.FileClipboard1` / `fr.ameleo.Host.FileClipboard1`: files and folders, in the
  MS-RDPECLIP format.
- `fr.ameleo.Agent.FileTransfer1`: files dropped from the client.
- `fr.ameleo.Agent.Display1`: display effects.
- `fr.ameleo.Agent.Camera1`: client cameras, H.264, with their controls (MS-RDPECAM).
- `fr.ameleo.Agent.Printer1` / `fr.ameleo.Host.Printer1`: client printers, jobs as OpenXPS.
- `fr.ameleo.Agent.Drive1` / `fr.ameleo.Host.Drive1`: client drives (MS-RDPEFS).
