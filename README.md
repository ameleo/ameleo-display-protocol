# ameleo-display-protocol

The D-Bus contract between a host serving a QEMU `-display dbus` virtual machine and the agent
in its guest: one virtio-serial port, a peer-to-peer D-Bus session on it, one versioned
interface per feature. This crate holds the interfaces as [zbus](https://crates.io/crates/zbus)
proxies, documented method by method; the host uses them to call the agent, the agent to call
the host back.

Agent: [ameleo-display-agent-windows](https://github.com/ameleo/ameleo-display-agent-windows)
(Windows guests).

## QEMU

```
-device virtio-serial-pci,id=virtserial1
-chardev dbus,id=agent0,name=fr.ameleo.agent.0
-device virtserialport,name=fr.ameleo.agent.0,chardev=agent0,bus=virtserial1.0,nr=1
```

QEMU exposes the port as `org.qemu.Display1.Chardev`; the host opens it and speaks D-Bus over
the stream.

## Session

The host is the D-Bus client (`AUTH ANONYMOUS`), the agent is the server. Every new handshake on
the stream is a new session: the agent drops clipboard ownership, transfers, client cameras,
printers and drives. An agent follows the D-Bus framing on the stream and should not rely on the
port reporting the host's departure.

Agent objects at `/fr/ameleo/Agent`:

| Interface | Content |
|---|---|
| `fr.ameleo.Agent1` | `Version`, `Channels`, `SessionLocked` properties, `Reset()` (the client went away; also locks the session) |
| `fr.ameleo.Agent.Mouse1` | `SetAbsPosition(head, x, y)`, `Press/Release(button)`, `Scroll(dx, dy)` |
| `fr.ameleo.Agent.Clipboard1` | `Grab(serial, mimes)`, `Release()`, `Request(mimes)`, `MaxSize`, signals `Grabbed`, `Released` |
| `fr.ameleo.Agent.FileClipboard1` | `LockClipData(id)`, `UnlockClipData(id)`, `FileContents(lindex, flags, position, requested, clip_data_id, has_clip_data_id)` |
| `fr.ameleo.Agent.Display1` | `SetEffects(no_wallpaper, no_font_smoothing, no_animation)` |
| `fr.ameleo.Agent.FileTransfer1` | `Start(name, size)`, `Write(id, data)`, `Finish(id)`, `Cancel(id)` |
| `fr.ameleo.Agent.Camera1` | `AddCamera(id, name, formats)`, `RemoveCamera(id)`, `PushFrame(id, timestamp_us, h264)`, signals `StreamRequested`, `StreamStopped`, `KeyFrameNeeded` |
| `fr.ameleo.Agent.Printer1` | `AddPrinter(id, name, is_default, capabilities, default_ticket)`, `RemovePrinter(id)`, `DocumentPropertiesDone(request_id, print_ticket, cancelled)` |
| `fr.ameleo.Agent.Drive1` | Client drives, see Drives |

Host object at `/fr/ameleo/Host`: `fr.ameleo.Host.Clipboard1.Request(serial, mimes)`, called
when a guest app pastes host data, `fr.ameleo.Host.FileClipboard1`, same methods as the agent
one, called when a guest app pastes host files, `fr.ameleo.Host.Printer1` (print jobs and the
client driver dialog) and `fr.ameleo.Host.Drive1`.

Conventions: `head` is the virtio-gpu scanout index. Buttons use the QEMU `InputButton`
numbering. Text is UTF-8 with `\n` line endings on the wire. Interfaces are versioned by name:
an incompatible change is a new interface, never a changed signature.

## File clipboard (MS-RDPECLIP)

Files go through the clipboard the RDP way, so a host bridging an RDP client relays the PDUs
without translating them:

* The clipboard type is `FileGroupDescriptorW`. Its data is a CLIPRDR_FILELIST: `cItems: u32 LE`
  then 592 byte FILEDESCRIPTORW, names relative and `\` separated, each directory before its
  content.
* `LockClipData` / `UnlockClipData` are the Lock / Unlock Clipboard Data PDUs, `FileContents` is
  the File Contents Request (`flags` 1 = SIZE, answered with 8 bytes LE, 2 = RANGE). A lock may
  come before the file list is requested.
* The agent may have several `FileContents` calls pending: the host matches each one to its
  client's stream id, and may answer with fewer bytes than asked (the agent asks for the rest,
  an empty answer is the end of the file).
* Host side, never wait for the client inside the loop that relays input and encodes frames: the
  pointer and the screen would stop for as long as a file chunk takes.

## Cameras (MS-RDPECAM)

Each client camera becomes a camera of the guest. The host forwards the client's H.264 stream:

1. `AddCamera(id, name, formats)` when the client announces a camera, `formats` being the H.264
   `(width, height, fps_num, fps_den)` it offers (even sizes, at most 3840x2160, 4 cameras;
   announce 30 fps or less).
2. An app opens the camera: signal `StreamRequested(id, format index)`, the host starts that
   stream on the client and sends every access unit with `PushFrame` (Annex B, SPS/PPS in band,
   no reply).
3. Signal `StreamStopped(id)` when no app uses it anymore.
4. Signal `KeyFrameNeeded(id)` (at most once a second) when the decoder needs an IDR: restart the
   client stream.
5. `RemoveCamera(id)`. Cameras also go away with the D-Bus session.

Controls (brightness, contrast, focus, zoom... MS-RDPECAM version 2):

* After `AddCamera`, `SetControls(id, controls)` with the client's Property List Response and the
  current value of each property (Property Value Response): `(set, property, capabilities, min,
  max, step, default, mode, value)`, MS-RDPECAM numbers as they are (`camera_control`).
* An app changes one: signal `ControlRequested(id, set, property, mode, value)`. The host sends
  the Set Property Value Request, then `ControlValue(id, set, property, mode, value)` with what
  the camera took (on an Error Response, the value it kept).

## Printers (Easy Print)

* **Options.** `AddPrinter` carries the client driver's PrintCapabilities and default
  PrintTicket: the guest printer offers the client driver's options, with the client's
  defaults. `AddPrinter` returns at once; the agent may install the printer later (when its
  user logs on).
* **Preferences.** The client driver's own dialog opens on the client: the agent calls
  `Host.Printer1.DocumentProperties`, the host shows the dialog and answers with
  `Printer1.DocumentPropertiesDone`.
* **Jobs.** Each job comes as OpenXPS, the ticket for the client driver in the document
  (`Metadata/Job_PT.xml`): `Host.Printer1.BeginJob`, `WriteJob` in 256 KiB chunks, `EndJob`. The
  host writes it to the client's Easy Print printer.
* **Lifecycle.** `RemovePrinter`, `Agent1.Reset` and a new D-Bus session remove the printers.

## Drives (MS-RDPEFS)

`fr.ameleo.Agent.Drive1` / `fr.ameleo.Host.Drive1`: the client's redirected drives as guest
drives.

* **No copy in the guest.** Every operation of a guest app is one call to the host, forwarded to
  the client as the RDPDR request: the open with the app's own NT parameters, read, write, query
  and set information, directory listing, volume information. The client's file system answers,
  with its `NTSTATUS` back to the app.
* **Client changes.** `Changed` (the host's `NotifyChangeDirectory` on the root) tells the agent
  to refresh.
* **End.** `RemoveDrive`, `Agent1.Reset` or a new D-Bus session remove the drives.

## Use

```toml
[dependencies]
ameleo-display-protocol = { git = "https://github.com/ameleo/ameleo-display-protocol", tag = "v0.1.0" }
```

The `tokio` feature (default) runs zbus on Tokio; `async-io` on its own executor.

## License

Licensed under either of the [Apache License, Version 2.0](LICENSE-APACHE) or the
[MIT License](LICENSE-MIT), at your option, so that any host or agent may implement the
interfaces.
