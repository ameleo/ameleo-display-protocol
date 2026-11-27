# ameleo-display-protocol

This crate defines the D-Bus contract between a host that serves a QEMU `-display dbus` virtual
machine and the agent running in its guest. The two talk over one virtio-serial port, with a
peer-to-peer D-Bus session on it and one versioned interface per feature. The crate provides the
interfaces as [zbus](https://crates.io/crates/zbus) proxies, documented method by method: the
host uses them to call the agent, and the agent uses them to call the host back.

Known agents, both under the GPL version 3:
[ameleo-display-agent-win](https://github.com/ameleo/ameleo-display-agent-win) for Windows
guests, and [ameleo-display-agent-linux](https://github.com/ameleo/ameleo-display-agent-linux)
for Linux guests.

## QEMU setup

```
-device virtio-serial-pci,id=virtserial1
-chardev dbus,id=agent0,name=fr.ameleo.agent.0
-device virtserialport,name=fr.ameleo.agent.0,chardev=agent0,bus=virtserial1.0,nr=1
```

QEMU exposes the port as `org.qemu.Display1.Chardev`. The host opens it and speaks D-Bus over the
stream.

## Sessions

The host is the D-Bus client and authenticates with `AUTH ANONYMOUS`; the agent is the server.
Every new handshake on the stream starts a new session, and the agent then drops its clipboard
ownership, transfers in progress, client cameras, printers and drives. An agent should follow the
D-Bus framing on the stream rather than rely on the port to report that the host went away.

The agent exports these interfaces at `/fr/ameleo/Agent`:

| Interface | Content |
|---|---|
| `fr.ameleo.Agent1` | `Version`, `Channels` and `SessionLocked` properties; `Reset()`, called when the client goes away, which also locks the session |
| `fr.ameleo.Agent.Mouse1` | `SetAbsPosition(head, x, y)`, `Press/Release(button)`, `Scroll(dx, dy)` |
| `fr.ameleo.Agent.Clipboard1` | `Grab(serial, mimes)`, `Release()`, `Request(mimes)`, `MaxSize`; signals `Grabbed` and `Released` |
| `fr.ameleo.Agent.FileClipboard1` | `LockClipData(id)`, `UnlockClipData(id)`, `FileContents(lindex, flags, position, requested, clip_data_id, has_clip_data_id)` |
| `fr.ameleo.Agent.Display1` | `SetEffects(no_wallpaper, no_font_smoothing, no_animation)` |
| `fr.ameleo.Agent.FileTransfer1` | `Start(name, size)`, `Write(id, data)`, `Finish(id)`, `Cancel(id)` |
| `fr.ameleo.Agent.Camera1` | `AddCamera(id, name, formats)`, `RemoveCamera(id)`, `PushFrame(id, timestamp_us, h264)`; signals `StreamRequested`, `StreamStopped` and `KeyFrameNeeded` |
| `fr.ameleo.Agent.Printer1` | `AddPrinter(id, name, is_default, capabilities, default_ticket)`, `RemovePrinter(id)`, `DocumentPropertiesDone(request_id, print_ticket, cancelled)` |
| `fr.ameleo.Agent.Drive1` | Client drives; see [Drives](#drives-ms-rdpefs) |
| `fr.ameleo.Agent.Consent1` | `Ask(id, kind, who)`, `End(id)`: the signed-in user's consent before someone else views or controls the session; see [Consent](#consent) |

The host exports these interfaces at `/fr/ameleo/Host`:

* `fr.ameleo.Host.Clipboard1.Request(serial, mimes)`, which the agent calls when a guest app
  pastes data copied on the host side.
* `fr.ameleo.Host.FileClipboard1`, with the same methods as the agent's, which the agent calls
  when a guest app pastes files copied on the host side.
* `fr.ameleo.Host.Printer1`, for print jobs and the client driver's dialog.
* `fr.ameleo.Host.Drive1`, for the files of the client drives.

Conventions: `head` is the virtio-gpu scanout index. Buttons use the QEMU `InputButton`
numbering. Text is UTF-8 with `\n` line endings on the wire. Interfaces are versioned by name: an
incompatible change creates a new interface, and an existing signature never changes.

## File clipboard (MS-RDPECLIP)

Files go through the clipboard in the RDP format, so a host that bridges an RDP client can relay
the PDUs without translating them.

* The clipboard type is `FileGroupDescriptorW`. Its data is a CLIPRDR_FILELIST: a `cItems: u32`
  in little endian, followed by FILEDESCRIPTORW structures of 592 bytes each. Names are relative
  and `\` separated, and each directory comes before its content.
* `LockClipData` and `UnlockClipData` correspond to the Lock and Unlock Clipboard Data PDUs.
  `FileContents` corresponds to the File Contents Request: `flags` 1 (SIZE) returns the size as
  8 bytes in little endian, and `flags` 2 (RANGE) returns a range of bytes. A lock may arrive
  before the file list is requested.
* The agent may have several `FileContents` calls pending at once. The host matches each one to
  a stream id of its client, and may answer with fewer bytes than requested: the agent then asks
  for the rest, and an empty answer marks the end of the file.
* On the host side, never wait for the client inside the loop that relays input and encodes
  frames: the pointer and the screen would freeze for as long as a file chunk takes.

## Cameras (MS-RDPECAM)

Each client camera becomes a camera in the guest. The host forwards the client's H.264 stream:

1. When the client announces a camera, the host calls `AddCamera(id, name, formats)`, where
   `formats` lists the H.264 formats the camera offers as `(width, height, fps_num, fps_den)`.
   Sizes must be even and at most 3840x2160, with at most 4 cameras. Announce 30 fps or less.
2. When an app opens the camera, the agent emits `StreamRequested(id, format index)`. The host
   starts that stream on the client and sends every access unit with `PushFrame`, in Annex B
   format with the SPS and PPS in band. `PushFrame` has no reply.
3. When no app uses the camera anymore, the agent emits `StreamStopped(id)`.
4. When its decoder needs an IDR frame, the agent emits `KeyFrameNeeded(id)`, at most once per
   second. The host restarts the client stream.
5. The host calls `RemoveCamera(id)` when the camera goes away. Cameras also go away when the
   D-Bus session ends.

Controls such as brightness, contrast, focus and zoom (MS-RDPECAM version 2):

* After `AddCamera`, the host calls `SetControls(id, controls)` with the client's Property List
  Response and the current value of each property (Property Value Response). Each control is
  `(set, property, capabilities, min, max, step, default, mode, value)`, with the MS-RDPECAM
  numbers unchanged (see the `camera_control` module).
* When an app changes a control, the agent emits `ControlRequested(id, set, property, mode,
  value)`. The host sends the Set Property Value Request, then calls `ControlValue(id, set,
  property, mode, value)` with the value the camera took. On an Error Response, it reports the
  value the camera kept.

## Printers (Easy Print)

* **Options.** `AddPrinter` carries the client driver's PrintCapabilities and its default
  PrintTicket, so the guest printer offers the client driver's options with the client's
  defaults. `AddPrinter` returns at once; the agent may install the printer later, when its user
  logs on.
* **Preferences.** The client driver's own dialog opens on the client: the agent calls
  `Host.Printer1.DocumentProperties`, the host shows the dialog, then answers with
  `Printer1.DocumentPropertiesDone`.
* **Jobs.** Each job arrives as an OpenXPS document, with the ticket for the client driver inside
  it (`Metadata/Job_PT.xml`), through `Host.Printer1.BeginJob`, then `WriteJob` in chunks of up
  to 256 KiB, then `EndJob`. The host writes the document to the client's Easy Print printer.
* **Lifecycle.** `RemovePrinter`, `Agent1.Reset` and a new D-Bus session remove the printers.

## Drives (MS-RDPEFS)

`fr.ameleo.Agent.Drive1` and `fr.ameleo.Host.Drive1` make the client's redirected drives
available in the guest.

* **No copy in the guest.** Every operation of a guest app becomes one call to the host, which
  forwards it to the client as the matching RDPDR request: opening with the app's own NT
  parameters, reading, writing, querying and setting information, listing directories and
  querying volume information. The client's file system answers, and its `NTSTATUS` goes back to
  the app.
* **Client changes.** The host calls `Changed` (after a `NotifyChangeDirectory` on the root) so
  that the agent can refresh the guest's views.
* **End.** `RemoveDrive`, `Agent1.Reset` and a new D-Bus session remove the drives.

## Consent

Before someone else views the session (`VIEW`) or takes control of it (`CONTROL`), the host asks
the user signed in to the guest, through the agent, on their own desktop: the host never decides
for them. The agent answers once with `fr.ameleo.Host.Consent1.Answered(id, granted)`: no at
once when nobody is signed in or the session is locked, or while another request is shown; no
after 30 seconds without an answer. The request says it comes from remote access, and its
default is no: a key typed before it appeared does not answer it. The host asks `CONTROL` only
for a person who has `VIEW`.
While a consent lasts, the agent shows who views or controls the session, and a way to stop it,
which calls `fr.ameleo.Host.Consent1.Stopped(id)`. A consent ends with the session's lock, its
user's sign-out or a switch to another user: the agent calls `Stopped` for each. The host calls
`End(id)` when it is over on its side, or to withdraw an unanswered request.

## Companion services

On Windows, an agent can leave some `fr.ameleo.Agent.*` interfaces to a companion: another service
in the guest, in its own process. The host never sees it. The agent keeps the port, announces the
companion's interfaces in `Agent1.Channels`, and relays calls in both directions, unchanged. The
`companion` module defines this guest-local link: the named pipe and its security descriptor, the
`fr.ameleo.CompanionLink1` interface the agent serves, and the `fr.ameleo.Companion1` interface the
companion serves.

For example, ameleo-display-agent-win leaves features such as drives and printers to a proprietary
companion service, which depends on this crate only.

## Usage

```toml
[dependencies]
ameleo-display-protocol = { git = "https://github.com/ameleo/ameleo-display-protocol", tag = "v0.2.0" }
```

It uses zbus 6. The default `tokio` feature runs zbus on Tokio; the `default-rt` feature runs it on
zbus's own runtime (zruntime), for a program without Tokio. The proxies are async only: zbus 6
generates no blocking ones. A peer on zbus 5 and version 0.1 of this crate speaks the same D-Bus
interfaces on the wire.

## License

Licensed under either the [Apache License, Version 2.0](LICENSE-APACHE) or the
[MIT License](LICENSE-MIT), at your option, so that any host or agent can implement the
interfaces.
