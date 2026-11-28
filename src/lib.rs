// SPDX-License-Identifier: MIT OR Apache-2.0

//! The D-Bus contract between a guest agent and its host.
//!
//! # Transport
//!
//! The host and the agent talk over a peer-to-peer D-Bus connection that runs
//! on the virtio-serial port [`PORT_NAME`]. On the host, the port is a
//! `-chardev dbus`, which QEMU exposes as `org.qemu.Display1.Chardev`:
//!
//! ```text
//! -device virtio-serial-pci,id=virtserial1
//! -chardev dbus,id=agent0,name=fr.ameleo.agent.0
//! -device virtserialport,name=fr.ameleo.agent.0,chardev=agent0,bus=virtserial1.0,nr=1
//! ```
//!
//! The host is the D-Bus client and starts the SASL handshake with
//! `AUTH ANONYMOUS`; the agent is the D-Bus server. Every new handshake on the
//! stream starts a new session, and the agent then drops all per-session
//! state, such as clipboard ownership and transfers in progress.
//!
//! # Objects
//!
//! * The agent exports [`AGENT_PATH`], with the core interface and one
//!   interface per channel.
//! * The host exports [`HOST_PATH`], with the interfaces the agent calls back,
//!   for example to fetch clipboard data.
//!
//! # Conventions
//!
//! * Text is UTF-8 with `\n` line endings on the wire. The agent converts it
//!   to the guest's conventions.
//! * Mouse buttons use the QEMU `InputButton` numbering; see [`Button`].
//! * Interfaces are versioned by name (`...1`). An incompatible change creates
//!   a new interface; an existing signature never changes.

/// The virtio-serial port name (`-device virtserialport,name=...`).
pub const PORT_NAME: &str = "fr.ameleo.agent.0";

/// The object path where the agent exports all its interfaces.
pub const AGENT_PATH: &str = "/fr/ameleo/Agent";

/// The object path where the host exports all its interfaces.
pub const HOST_PATH: &str = "/fr/ameleo/Host";

pub mod print_link;

pub mod iface {
    pub const AGENT: &str = "fr.ameleo.Agent1";
    pub const MOUSE: &str = "fr.ameleo.Agent.Mouse1";
    pub const CLIPBOARD: &str = "fr.ameleo.Agent.Clipboard1";
    pub const DISPLAY: &str = "fr.ameleo.Agent.Display1";
    pub const FILE_TRANSFER: &str = "fr.ameleo.Agent.FileTransfer1";
    pub const FILE_CLIPBOARD: &str = "fr.ameleo.Agent.FileClipboard1";
    pub const CAMERA: &str = "fr.ameleo.Agent.Camera1";
    pub const PRINTER: &str = "fr.ameleo.Agent.Printer1";
    pub const HOST_PRINTER: &str = "fr.ameleo.Host.Printer1";
    pub const HOST_CLIPBOARD: &str = "fr.ameleo.Host.Clipboard1";
    pub const HOST_FILE_CLIPBOARD: &str = "fr.ameleo.Host.FileClipboard1";
    pub const DRIVE: &str = "fr.ameleo.Agent.Drive1";
    pub const HOST_DRIVE: &str = "fr.ameleo.Host.Drive1";
    pub const CONSENT: &str = "fr.ameleo.Agent.Consent1";
    pub const HOST_CONSENT: &str = "fr.ameleo.Host.Consent1";
}

/// Clipboard MIME types that the agent understands.
pub mod mime {
    pub const TEXT: &str = "text/plain;charset=utf-8";
    pub const PNG: &str = "image/png";
    pub const BMP: &str = "image/bmp";
    /// A file list in the MS-RDPECLIP format. The type name is the Windows
    /// and RDP clipboard format name. The data is a CLIPRDR_FILELIST
    /// (MS-RDPECLIP 2.2.5.2.3): a `cItems: u32` in little endian, followed by
    /// `cItems` FILEDESCRIPTORW structures of 592 bytes each. Names are
    /// relative and `\` separated, and each directory comes before its
    /// content. File data is read with `FileClipboard1.FileContents`.
    pub const FILE_LIST: &str = "FileGroupDescriptorW";
}

/// File clipboard constants (MS-RDPECLIP 2.2.5.3).
pub mod file_contents {
    /// `dwFlags`: return the file size, as 8 bytes in little endian.
    pub const SIZE: u32 = 0x1;
    /// `dwFlags`: return `requested` bytes starting at `position`.
    pub const RANGE: u32 = 0x2;
    /// The largest `requested` value honored in a single call.
    pub const MAX_RANGE: u32 = 4 * 1024 * 1024;
    /// The size of one FILEDESCRIPTORW structure.
    pub const DESCRIPTOR_SIZE: usize = 592;
}

/// The maximum payload of one `FileTransfer1.Write` call.
pub const FILE_CHUNK_MAX: usize = 256 * 1024;

/// Mouse buttons, numbered like the QEMU `InputButton` enumeration.
#[repr(u32)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Button {
    Left = 0,
    Middle = 1,
    Right = 2,
    WheelUp = 3,
    WheelDown = 4,
    Side = 5,
    Extra = 6,
    WheelLeft = 7,
    WheelRight = 8,
}

impl TryFrom<u32> for Button {
    type Error = u32;
    fn try_from(v: u32) -> std::result::Result<Self, u32> {
        Ok(match v {
            0 => Self::Left,
            1 => Self::Middle,
            2 => Self::Right,
            3 => Self::WheelUp,
            4 => Self::WheelDown,
            5 => Self::Side,
            6 => Self::Extra,
            7 => Self::WheelLeft,
            8 => Self::WheelRight,
            other => return Err(other),
        })
    }
}

/// Errors returned by the agent.
#[derive(zbus::DBusError, Debug)]
#[zbus(prefix = "fr.ameleo.Agent.Error")]
pub enum Error {
    #[zbus(error)]
    ZBus(zbus::Error),
    /// The user session is locked, so the request is refused.
    SessionLocked(String),
    /// There is not enough disk space. The message holds the number of free
    /// bytes.
    NotEnoughSpace(String),
    InvalidArgument(String),
    NotFound(String),
    /// The guest clipboard no longer holds the requested data.
    NoData(String),
    Failed(String),
}

pub type Result<T> = std::result::Result<T, Error>;

/// The core interface: the agent's identity and the channels it supports.
#[zbus::proxy(
    interface = "fr.ameleo.Agent1",
    default_service = "fr.ameleo.Agent",
    default_path = "/fr/ameleo/Agent"
)]
pub trait Agent {
    /// The agent version.
    #[zbus(property)]
    fn version(&self) -> zbus::Result<String>;

    /// The channel interfaces that this agent implements.
    #[zbus(property)]
    fn channels(&self) -> zbus::Result<Vec<String>>;

    /// True while the guest session is locked, or while the logon screen is
    /// shown.
    #[zbus(property)]
    fn session_locked(&self) -> zbus::Result<bool>;

    /// Tells the agent that the remote client went away. The agent drops
    /// clipboard ownership, aborts transfers, removes the client cameras,
    /// printers and drives, and locks the guest session, but keeps the D-Bus
    /// session open. Call it when the remote client disconnects.
    fn reset(&self) -> zbus::Result<()>;
}

/// An absolute pointer that supports several heads.
///
/// Coordinates are relative to the head. `head` is the virtio-gpu scanout
/// index, which is also the index of the QEMU console for that head.
#[zbus::proxy(
    interface = "fr.ameleo.Agent.Mouse1",
    default_service = "fr.ameleo.Agent",
    default_path = "/fr/ameleo/Agent"
)]
pub trait Mouse {
    #[zbus(no_reply)]
    fn set_abs_position(&self, head: u32, x: u32, y: u32) -> zbus::Result<()>;

    #[zbus(no_reply)]
    fn press(&self, button: u32) -> zbus::Result<()>;

    #[zbus(no_reply)]
    fn release(&self, button: u32) -> zbus::Result<()>;

    /// Scrolls with high resolution, in Windows wheel units (120 is one
    /// notch).
    #[zbus(no_reply)]
    fn scroll(&self, dx: i32, dy: i32) -> zbus::Result<()>;
}

/// The clipboard, agent side.
///
/// The ownership model is the same as QEMU's `Display1.Clipboard`: whoever
/// copied last owns the clipboard and announces the MIME types it can
/// provide. The other side fetches the data on demand.
#[zbus::proxy(
    interface = "fr.ameleo.Agent.Clipboard1",
    default_service = "fr.ameleo.Agent",
    default_path = "/fr/ameleo/Agent"
)]
pub trait Clipboard {
    /// Tells the agent that the host now owns the clipboard. When a guest app
    /// pastes, the agent fetches the data with
    /// `fr.ameleo.Host.Clipboard1.Request`.
    fn grab(&self, serial: u32, mimes: Vec<String>) -> zbus::Result<()>;

    /// Tells the agent that the host gives up clipboard ownership.
    fn release(&self) -> zbus::Result<()>;

    /// Fetches the guest clipboard data in one of the announced types.
    fn request(&self, mimes: Vec<String>) -> zbus::Result<(String, Vec<u8>)>;

    /// The largest payload the host accepts, or -1 for no limit.
    #[zbus(property)]
    fn max_size(&self) -> zbus::Result<i32>;
    #[zbus(property)]
    fn set_max_size(&self, value: i32) -> zbus::Result<()>;

    /// Emitted when the guest clipboard changes.
    #[zbus(signal)]
    fn grabbed(&self, serial: u32, mimes: Vec<String>) -> zbus::Result<()>;

    /// Emitted when the guest clipboard is emptied or no longer holds
    /// anything the agent can export.
    #[zbus(signal)]
    fn released(&self) -> zbus::Result<()>;
}

/// The clipboard, host side. The agent calls it when a guest app pastes.
#[zbus::proxy(
    interface = "fr.ameleo.Host.Clipboard1",
    default_service = "fr.ameleo.Host",
    default_path = "/fr/ameleo/Host"
)]
pub trait HostClipboard {
    fn request(&self, serial: u32, mimes: Vec<String>) -> zbus::Result<(String, Vec<u8>)>;
}

/// File contents for a [`mime::FILE_LIST`] clipboard. The calls match the
/// MS-RDPECLIP File Contents Request and Lock/Unlock Clipboard Data PDUs.
///
/// Both sides implement the interface with the same signature: the agent as
/// `fr.ameleo.Agent.FileClipboard1`, for files copied in the guest, and the
/// host as `fr.ameleo.Host.FileClipboard1`, for files copied on the client
/// and pasted in the guest.
///
/// * `lindex` is the index of the file in the file list returned for the
///   grab.
/// * `flags` is [`file_contents::SIZE`] or [`file_contents::RANGE`].
/// * When `has_clip_data_id` is true, the request refers to the clipboard
///   content frozen by `LockClipData(clip_data_id)`; otherwise it refers to
///   the current content.
#[zbus::proxy(
    interface = "fr.ameleo.Agent.FileClipboard1",
    default_service = "fr.ameleo.Agent",
    default_path = "/fr/ameleo/Agent"
)]
pub trait FileClipboard {
    /// Freezes the current clipboard content under `clip_data_id`: its file
    /// contents stay readable after the clipboard changes, until the content
    /// is unlocked. This call may come before the file list is requested.
    fn lock_clip_data(&self, clip_data_id: u32) -> zbus::Result<()>;
    fn unlock_clip_data(&self, clip_data_id: u32) -> zbus::Result<()>;
    fn file_contents(
        &self,
        lindex: i32,
        flags: u32,
        position: u64,
        requested: u32,
        clip_data_id: u32,
        has_clip_data_id: bool,
    ) -> zbus::Result<Vec<u8>>;
}

/// The host side of [`FileClipboardProxy`]. The agent calls it when a guest app
/// pastes files that the host announced.
#[zbus::proxy(
    interface = "fr.ameleo.Host.FileClipboard1",
    default_service = "fr.ameleo.Host",
    default_path = "/fr/ameleo/Host"
)]
pub trait HostFileClipboard {
    fn lock_clip_data(&self, clip_data_id: u32) -> zbus::Result<()>;
    fn unlock_clip_data(&self, clip_data_id: u32) -> zbus::Result<()>;
    fn file_contents(
        &self,
        lindex: i32,
        flags: u32,
        position: u64,
        requested: u32,
        clip_data_id: u32,
        has_clip_data_id: bool,
    ) -> zbus::Result<Vec<u8>>;
}

/// Camera limits of the agent.
pub mod camera {
    /// The number of client cameras that can be shown at the same time.
    pub const MAX_CAMERAS: u32 = 4;
    /// The largest frame size of a camera format.
    pub const MAX_WIDTH: u32 = 3840;
    pub const MAX_HEIGHT: u32 = 2160;
}

/// Client webcams (MS-RDPECAM on the RDP side), shown as cameras in the guest.
///
/// Flow:
/// 1. The client announces a camera, and the host calls `AddCamera` with the
///    H.264 formats that the camera offers. The camera appears in the guest,
///    but nothing streams yet.
/// 2. When an app opens the camera, the agent emits
///    `StreamRequested(id, format)`. The host starts that stream on the
///    client (Start Streams) and pushes each H.264 access unit with
///    `PushFrame`.
/// 3. When the app closes the camera, the agent emits `StreamStopped(id)` and
///    the host stops the stream.
/// 4. The agent emits `KeyFrameNeeded(id)` when its decoder lost track, for
///    example because the stream started on a non-IDR frame or a frame was
///    lost. The host restarts the client stream to get an IDR frame.
/// 5. When the camera goes away on the client, the host calls
///    `RemoveCamera(id)`.
///
/// Controls such as brightness and focus (MS-RDPECAM version 2): after
/// `AddCamera`, the host calls `SetControls` with what the client offers
/// (Property List Response) and the current values (Property Value
/// Response). When an app changes a control, the agent emits
/// `ControlRequested`; the host sends the Set Property Value Request, then
/// reports the value the camera took with `ControlValue` (or, on an Error
/// Response, the value the camera kept). Sets, property ids, capabilities
/// and modes use the MS-RDPECAM numbering; see [`camera_control`].
///
/// The host chooses the camera ids. Formats are indexes into the list given
/// to `AddCamera`. Frame sizes are even, and at most [`camera::MAX_WIDTH`] x
/// [`camera::MAX_HEIGHT`].
#[zbus::proxy(
    interface = "fr.ameleo.Agent.Camera1",
    default_service = "fr.ameleo.Agent",
    default_path = "/fr/ameleo/Agent"
)]
pub trait Camera {
    /// `formats` lists the H.264 formats as
    /// `(width, height, fps numerator, fps denominator)`. The agent shows at
    /// most [`camera::MAX_CAMERAS`] cameras at once.
    fn add_camera(
        &self,
        id: u32,
        name: &str,
        formats: Vec<(u32, u32, u32, u32)>,
    ) -> zbus::Result<()>;

    fn remove_camera(&self, id: u32) -> zbus::Result<()>;

    /// Pushes one H.264 access unit in Annex B format, with start codes and
    /// with the SPS and PPS in band before each IDR frame, in the format of
    /// the last `StreamRequested`. `timestamp_us` is the capture time on the
    /// client, in microseconds.
    #[zbus(no_reply)]
    fn push_frame(&self, id: u32, timestamp_us: u64, data: Vec<u8>) -> zbus::Result<()>;

    /// Emitted when an app opens camera `id` with format index `format`.
    #[zbus(signal)]
    fn stream_requested(&self, id: u32, format: u32) -> zbus::Result<()>;

    /// Emitted when no app uses camera `id` anymore.
    #[zbus(signal)]
    fn stream_stopped(&self, id: u32) -> zbus::Result<()>;

    /// Emitted when the agent needs an IDR frame to decode camera `id` again.
    #[zbus(signal)]
    fn key_frame_needed(&self, id: u32) -> zbus::Result<()>;

    /// Sets the controls of camera `id`, replacing any controls set before:
    /// at most [`camera_control::MAX_CONTROLS`] [`camera_control::Control`]
    /// values.
    fn set_controls(&self, id: u32, controls: Vec<camera_control::Control>) -> zbus::Result<()>;

    /// Reports the current value of a control of camera `id`, as the client
    /// gave it. The host calls it after a Set Property Value Request, and may
    /// call it at any other time.
    fn control_value(
        &self,
        id: u32,
        set: u8,
        property: u8,
        mode: u8,
        value: i32,
    ) -> zbus::Result<()>;

    /// Emitted when an app changes a control. The host sends the client a
    /// Set Property Value Request. `value` is meaningless when `mode` is
    /// [`camera_control::AUTO`].
    #[zbus(signal)]
    fn control_requested(
        &self,
        id: u32,
        set: u8,
        property: u8,
        mode: u8,
        value: i32,
    ) -> zbus::Result<()>;
}

/// Camera controls, numbered as in MS-RDPECAM (PROPERTY_DESCRIPTION and
/// PROPERTY_VALUE). The host passes the client's values through unchanged.
pub mod camera_control {
    /// `(set, property, capabilities, min, max, step, default, mode, value)`:
    /// a PROPERTY_DESCRIPTION, followed by the PROPERTY_VALUE that the client
    /// gave for it.
    pub type Control = (u8, u8, u8, i32, i32, i32, i32, u8, i32);

    pub const MAX_CONTROLS: usize = 16;

    /// Property sets.
    pub const CAMERA_CONTROL: u8 = 0x01;
    pub const VIDEO_PROC_AMP: u8 = 0x02;

    /// Properties of the CameraControl set.
    pub const EXPOSURE: u8 = 0x01;
    pub const FOCUS: u8 = 0x02;
    pub const PAN: u8 = 0x03;
    pub const ROLL: u8 = 0x04;
    pub const TILT: u8 = 0x05;
    pub const ZOOM: u8 = 0x06;

    /// Properties of the VideoProcAmp set.
    pub const BACKLIGHT_COMPENSATION: u8 = 0x01;
    pub const BRIGHTNESS: u8 = 0x02;
    pub const CONTRAST: u8 = 0x03;
    pub const HUE: u8 = 0x04;
    pub const WHITE_BALANCE: u8 = 0x05;

    /// Capability flags and modes.
    pub const MANUAL: u8 = 0x01;
    pub const AUTO: u8 = 0x02;
}

/// Printer limits of the agent.
pub mod printer {
    /// The number of client printers that can be shown at the same time.
    pub const MAX_PRINTERS: u32 = 32;
    /// The largest PrintCapabilities or PrintTicket accepted by
    /// `AddPrinter`, in bytes.
    pub const MAX_XML: usize = 4 * 1024 * 1024;
    /// The largest job document chunk in one `WriteJob` call.
    pub const CHUNK_MAX: usize = 256 * 1024;
}

/// Client printers (RDP printer redirection with Easy Print), shown as
/// printers in the guest.
///
/// Jobs reach the host as OpenXPS documents, which the host writes to the
/// client's Easy Print printer.
///
/// The client driver's options are available in the guest: the agent builds
/// the printer's capabilities from the PrintCapabilities that the host gives
/// in `AddPrinter`, and the printing preferences open the client driver's
/// own dialog on the client ([`HostPrinterProxy::document_properties`]). The
/// settings the user chooses there stay with the guest printer and are sent
/// with every job.
///
/// Flow:
/// 1. The client announces a printer, and the host calls `AddPrinter`. The
///    call returns at once; the agent installs the printer for the logged on
///    user, either immediately or when the user logs on.
/// 2. When apps print, the agent calls `BeginJob`, `WriteJob`... and `EndJob`
///    on [`HostPrinterProxy`].
/// 3. When the client printer goes away, the host calls `RemovePrinter(id)`
///    and the agent deletes the printer at once. When the D-Bus session ends
///    (or on `Agent1.Reset`), the agent keeps the printers for 60 s, unusable.
///    A printer with the same name in the next session reuses its guest
///    printer, along with the user's settings; the others are then deleted.
///
/// The host chooses the printer ids.
#[zbus::proxy(
    interface = "fr.ameleo.Agent.Printer1",
    default_service = "fr.ameleo.Agent",
    default_path = "/fr/ameleo/Agent"
)]
pub trait Printer {
    /// `name` is the name shown in the guest. When `is_default` is true, the
    /// printer is the client's default printer, and the agent makes it the
    /// guest user's default printer.
    ///
    /// `capabilities` is the client driver's PrintCapabilities XML (MS-RDPEXPS
    /// `GetPrintCapabilities`). `default_ticket` is the driver's default
    /// PrintTicket (the ticket of its default DEVMODE), which gives the guest
    /// printer its defaults: the capabilities do not say which option is the
    /// default, so without it the agent guesses (one-sided, portrait, and so
    /// on). Either may be empty; without capabilities, the guest printer only
    /// has generic options. Each is at most [`printer::MAX_XML`] bytes.
    fn add_printer(
        &self,
        id: u32,
        name: &str,
        is_default: bool,
        capabilities: &str,
        default_ticket: &str,
    ) -> zbus::Result<()>;

    fn remove_printer(&self, id: u32) -> zbus::Result<()>;

    /// Answers [`HostPrinterProxy::document_properties`] with the PrintTicket the
    /// user chose in the client driver's dialog, or with `cancelled` set.
    fn document_properties_done(
        &self,
        request_id: u32,
        print_ticket: &str,
        cancelled: bool,
    ) -> zbus::Result<()>;
}

/// The host side of [`PrinterProxy`]: the client printers.
///
/// The agent sends a job document with `BeginJob`, then `WriteJob` for each
/// chunk in order (at most [`printer::CHUNK_MAX`] bytes each), then `EndJob`
/// once the host has received everything. The replies provide flow control:
/// the agent sends the next chunk after the previous call returns.
/// `AbortJob` drops a job, for example when it is cancelled in the guest or
/// the connection is lost. The agent chooses the job ids, which are unique
/// for the agent's lifetime.
///
/// PrintTickets are Print Schema XML for the client driver. They hold the
/// standard settings (`psk:`) and the client driver's own settings in its
/// namespaces. Their prefixes may differ from the client's: only the
/// namespace URIs matter.
#[zbus::proxy(
    interface = "fr.ameleo.Host.Printer1",
    default_service = "fr.ameleo.Host",
    default_path = "/fr/ameleo/Host"
)]
pub trait HostPrinter {
    /// `document_format` is the job's MIME type, `application/oxps`: the host
    /// writes the document to the client's Easy Print printer unchanged.
    /// `job_name` and `user_name` are as the guest app and user gave them,
    /// and may be empty. `print_ticket` holds the settings to print with, for
    /// the client driver: the user's choices from the guest dialogs and the
    /// client driver's dialog, including the driver's `PageDevmodeSnapshot`
    /// (its whole DEVMODE in base64, in the driver's `oemdriverpt` namespace)
    /// when it has one. The document already carries the same ticket as its
    /// job PrintTicket (`Metadata/Job_PT.xml`); it is given here for
    /// information.
    fn begin_job(
        &self,
        printer_id: u32,
        job_id: u32,
        document_format: &str,
        job_name: &str,
        user_name: &str,
        print_ticket: &str,
    ) -> zbus::Result<()>;
    fn write_job(&self, job_id: u32, data: Vec<u8>) -> zbus::Result<()>;
    /// Ends a job once the whole document is written. Returns when the host
    /// has taken the document.
    fn end_job(&self, job_id: u32) -> zbus::Result<()>;
    fn abort_job(&self, job_id: u32) -> zbus::Result<()>;

    /// Shows the client driver's own document properties dialog on the
    /// client (MS-RDPEXPS `ASYNC_DOC_PROPS_REQ`). The dialog starts from
    /// `print_ticket`, the guest printer's current settings as in
    /// [`HostPrinterProxy::begin_job`]: the client driver's ticket that the user
    /// chose last, whose `PageDevmodeSnapshot` is the DEVMODE to start from.
    /// The call returns at once. When the user closes the dialog, however
    /// long it stays open, the host calls [`PrinterProxy::document_properties_done`]
    /// with `request_id` and the client driver's PrintTicket. The agent keeps
    /// that ticket and sends it with the next jobs.
    fn document_properties(
        &self,
        printer_id: u32,
        request_id: u32,
        print_ticket: &str,
    ) -> zbus::Result<()>;
}

/// Drive limits and wire types.
pub mod drive {
    /// The number of client drives that can be shown at the same time.
    pub const MAX_DRIVES: u32 = 24;
    /// The largest `Read` length or `Write` payload, in bytes.
    pub const IO_MAX: u32 = 1024 * 1024;

    /// The `information` value returned by [`super::HostDriveProxy::open`]: what
    /// the client did, as the MS-RDPEFS `DR_CREATE_RSP` value. That
    /// specification has no `CREATED` value: it uses `SUPERSEDED` for "a new
    /// file was created", and also when the field is omitted. `CREATED` is
    /// the NT `IO_STATUS_BLOCK` value that some clients send. Both mean that
    /// the file was created; see [`information::created`].
    pub mod information {
        pub const SUPERSEDED: u32 = 0;
        pub const OPENED: u32 = 1;
        pub const CREATED: u32 = 2;
        pub const OVERWRITTEN: u32 = 3;

        /// Returns true if the client created the file.
        pub fn created(information: u32) -> bool {
            information == SUPERSEDED || information == CREATED
        }
    }

    /// The `action` of a change passed to [`super::DriveProxy::changed`], using
    /// the `FILE_NOTIFY_INFORMATION` values.
    pub mod action {
        pub const ADDED: u32 = 1;
        pub const REMOVED: u32 = 2;
        pub const MODIFIED: u32 = 3;
        pub const RENAMED_OLD_NAME: u32 = 4;
        pub const RENAMED_NEW_NAME: u32 = 5;
    }

    /// A file or directory: `(name, attributes, size, creation_time,
    /// last_access_time, last_write_time, change_time)`. In a listing,
    /// `name` is the entry's own name; otherwise it is the last component of
    /// the path. `attributes` holds the `FILE_ATTRIBUTE_*` flags (`0x10` for
    /// a directory). Times are FILETIME values (100 ns units since 1601,
    /// UTC), or 0 when unknown. The D-Bus signature is `(suttttt)`.
    pub type Entry = (String, u32, u64, u64, u64, u64, u64);

    /// A change on the client drive: `(action, path)`; see [`action`].
    pub type Change = (u32, String);
}

/// Client drives (RDP drive redirection, MS-RDPEFS), shown as drives in the
/// guest.
///
/// Every file operation of a guest app goes to the client as it happens,
/// through [`HostDriveProxy`], as on a Windows RDP server. The guest keeps no copy
/// of the client's files.
///
/// Flow:
/// 1. The client announces a drive, and the host calls `AddDrive`. The agent
///    mounts the drive for the logged on user only.
/// 2. Guest apps use the drive through [`HostDriveProxy`].
/// 3. When files change on the client (the host watches the tree with
///    `NotifyChangeDirectory` on the drive root), the host calls `Changed`
///    so that the agent can refresh the guest's views.
/// 4. When the client drive goes away, the host calls `RemoveDrive(id)`.
///    When the D-Bus session ends (or on `Agent1.Reset`), the agent unmounts
///    all drives.
///
/// The host chooses the drive ids; the RDPDR `DeviceId` is a good fit.
///
/// Paths are relative to the drive root, `\` separated, without a leading
/// `\`. The empty path `""` is the root.
#[zbus::proxy(
    interface = "fr.ameleo.Agent.Drive1",
    default_service = "fr.ameleo.Agent",
    default_path = "/fr/ameleo/Agent"
)]
pub trait Drive {
    /// `name` is the client's name for the drive (the RDPDR
    /// `PreferredDosName`, such as `C`). `label` is the volume label shown in
    /// the guest (such as `C on PC-CLIENT`). Returns the guest drive letter
    /// (such as `Z`).
    fn add_drive(&self, id: u32, name: &str, label: &str) -> zbus::Result<String>;

    fn remove_drive(&self, id: u32) -> zbus::Result<()>;

    /// Reports changes made on the client drive, in order, as the client
    /// reported them (`FILE_NOTIFY_INFORMATION`, with paths relative to the
    /// drive root). A client that cannot watch the tree sends nothing; the
    /// guest then sees the client's changes the next time it lists a
    /// directory.
    fn changed(&self, id: u32, changes: Vec<drive::Change>) -> zbus::Result<()>;
}

/// The host side of [`DriveProxy`]: the files of the client drive, with one RDPDR
/// request per call (MS-RDPEFS).
///
/// Every method returns the client's `NTSTATUS` first, exactly as the client
/// answered (`0` for success, `0xC0000034` for not found, `0xC0000022` for
/// access denied, `0xC0000043` for a sharing violation, and so on); the agent
/// passes it on to the app. A D-Bus error means that the host could not ask
/// the client at all, for example because the drive does not exist or the
/// client is gone.
///
/// Several calls may be pending at the same time, on the same drive and even
/// on the same file: the host sends each one to the client at once, and
/// answers when the client has answered.
///
/// * `Open` sends `IRP_MJ_CREATE` with the guest's own NT parameters,
///   unchanged; `Close` sends `IRP_MJ_CLOSE`.
/// * `Read`, `Write` and `Flush` send `IRP_MJ_READ`, `IRP_MJ_WRITE` and
///   `IRP_MJ_FLUSH_BUFFERS`.
/// * `Query` sends `IRP_MJ_QUERY_INFORMATION` for `FileBasicInformation` and
///   `FileStandardInformation`.
/// * `SetBasic` (`FileBasicInformation`), `SetSize`
///   (`FileEndOfFileInformation`, or `FileAllocationInformation`),
///   `SetDelete` (`FileDispositionInformation`) and `Rename`
///   (`FileRenameInformation`) all send `IRP_MJ_SET_INFORMATION` on the open
///   file.
/// * `List` opens the directory itself (`FILE_LIST_DIRECTORY`,
///   `FILE_DIRECTORY_FILE`, all sharing), sends `IRP_MN_QUERY_DIRECTORY` with
///   `FileBothDirectoryInformation` from the initial query until
///   `STATUS_NO_MORE_FILES`, then closes it. The `.` and `..` entries are
///   left out.
/// * `Volume` opens the root itself, sends
///   `IRP_MJ_QUERY_VOLUME_INFORMATION` for `FileFsFullSizeInformation`,
///   `FileFsVolumeInformation` and `FileFsAttributeInformation`, then closes
///   it.
#[zbus::proxy(
    interface = "fr.ameleo.Host.Drive1",
    default_service = "fr.ameleo.Host",
    default_path = "/fr/ameleo/Host"
)]
pub trait HostDrive {
    /// Sends `IRP_MJ_CREATE` (`DR_CREATE_REQ`) with `desired_access`,
    /// `file_attributes`, `share_access`, `create_disposition` (`FILE_OPEN`
    /// 1, `FILE_CREATE` 2, `FILE_OPEN_IF` 3...) and `create_options`
    /// (`FILE_DIRECTORY_FILE`...) as the guest app gave them. Returns
    /// `(status, file_id, information)`; see [`drive::information`].
    #[allow(clippy::too_many_arguments)]
    fn open(
        &self,
        drive_id: u32,
        path: &str,
        desired_access: u32,
        file_attributes: u32,
        share_access: u32,
        create_disposition: u32,
        create_options: u32,
    ) -> zbus::Result<(u32, u32, u32)>;

    fn close(&self, drive_id: u32, file_id: u32) -> zbus::Result<u32>;

    /// Reads at most `length` bytes (and at most [`drive::IO_MAX`]) from
    /// `offset`. Returns fewer bytes at the end of the file, and none past
    /// it.
    fn read(
        &self,
        drive_id: u32,
        file_id: u32,
        offset: u64,
        length: u32,
    ) -> zbus::Result<(u32, Vec<u8>)>;

    /// Writes `data` (at most [`drive::IO_MAX`] bytes) at `offset`. Returns
    /// the number of bytes written.
    fn write(
        &self,
        drive_id: u32,
        file_id: u32,
        offset: u64,
        data: Vec<u8>,
    ) -> zbus::Result<(u32, u32)>;

    fn flush(&self, drive_id: u32, file_id: u32) -> zbus::Result<u32>;

    fn query(&self, drive_id: u32, file_id: u32) -> zbus::Result<(u32, drive::Entry)>;

    /// An `attributes` value of 0, or a time of 0, leaves that field
    /// unchanged.
    fn set_basic(
        &self,
        drive_id: u32,
        file_id: u32,
        attributes: u32,
        creation_time: u64,
        last_access_time: u64,
        last_write_time: u64,
    ) -> zbus::Result<u32>;

    /// Sets the end of the file or, when `allocation` is true, the allocation
    /// size.
    fn set_size(
        &self,
        drive_id: u32,
        file_id: u32,
        size: u64,
        allocation: bool,
    ) -> zbus::Result<u32>;

    /// Marks the file to be deleted when the client closes it
    /// (`DeleteFile` true), or clears that mark. A non-empty directory
    /// answers `STATUS_DIRECTORY_NOT_EMPTY`.
    fn set_delete(&self, drive_id: u32, file_id: u32, delete: bool) -> zbus::Result<u32>;

    /// Renames the file to `new_path`, relative to the drive root. When
    /// `replace` is true, an existing file at `new_path` is replaced.
    fn rename(
        &self,
        drive_id: u32,
        file_id: u32,
        new_path: &str,
        replace: bool,
    ) -> zbus::Result<u32>;

    /// Lists the entries of the directory `path`.
    fn list(&self, drive_id: u32, path: &str) -> zbus::Result<(u32, Vec<drive::Entry>)>;

    /// Returns `(status, total_bytes, free_bytes, label, file_system)`: the
    /// size of the client volume, the free space available to the user, the
    /// volume label and the file system name (such as `NTFS`).
    fn volume(&self, drive_id: u32) -> zbus::Result<(u32, u64, u64, String, String)>;
}

/// Desktop settings for remote sessions.
///
/// There is deliberately no monitor configuration here: resolution and
/// layout belong to the display driver (EDID and hotplug), and the agent
/// never changes the display configuration itself.
#[zbus::proxy(
    interface = "fr.ameleo.Agent.Display1",
    default_service = "fr.ameleo.Agent",
    default_path = "/fr/ameleo/Agent"
)]
pub trait Display {
    /// The agent keeps these settings across reboots and applies them again
    /// at every logon.
    fn set_effects(
        &self,
        disable_wallpaper: bool,
        disable_font_smoothing: bool,
        disable_animation: bool,
    ) -> zbus::Result<()>;
}

/// Files pushed from the host to the guest, saved in the user's Downloads
/// folder.
///
/// The method replies provide flow control: the host sends the next chunk
/// once `Write` has returned.
#[zbus::proxy(
    interface = "fr.ameleo.Agent.FileTransfer1",
    default_service = "fr.ameleo.Agent",
    default_path = "/fr/ameleo/Agent"
)]
pub trait FileTransfer {
    /// Starts a transfer and returns its id.
    fn start(&self, name: &str, size: u64) -> zbus::Result<u32>;
    /// Writes the next chunk, at most [`FILE_CHUNK_MAX`] bytes.
    fn write(&self, id: u32, data: Vec<u8>) -> zbus::Result<()>;
    /// Checks the size, keeps the file and shows the folder to the user.
    /// Returns the final file name, which may carry a ` (n)` suffix.
    fn finish(&self, id: u32) -> zbus::Result<String>;
    /// Cancels the transfer and deletes the partial file.
    fn cancel(&self, id: u32) -> zbus::Result<()>;
}

/// What the host asks the user's consent for: [`ConsentProxy::ask`]'s `kind`.
pub mod consent {
    /// Someone else sees the guest's screens (and hears its sound), and nothing more.
    pub const VIEW: u32 = 1;
    /// Someone else's keyboard and pointer act in the guest, besides the user's own.
    pub const CONTROL: u32 = 2;
    /// How long the user has to answer: past it, the agent answers no.
    pub const TIMEOUT_SECS: u32 = 30;
    /// The longest `who` the host sends, in characters; the agent cuts a longer one.
    pub const WHO_MAX: usize = 128;
}

/// The consent of the user signed in to the guest's session, before someone
/// else watches it or takes control of it (shadowing, remote assistance).
///
/// Only the agent asks, on the user's own desktop: the host never decides
/// for the user, and the remote client is not where consent is given.
///
/// 1. The host calls `Ask(id, kind, who)`. `id` names the request; the host
///    chooses it, unique for the D-Bus session. `kind` is one of the
///    [`consent`] kinds; `who` names the person asking, as the host knows
///    them (at most [`consent::WHO_MAX`] characters, shown as plain text).
/// 2. The agent shows the request on the user's desktop and answers once,
///    with `fr.ameleo.Host.Consent1.Answered(id, granted)`. It answers false
///    at once when no user is signed in, the session is locked, or the
///    logon screen is shown; false to a request that comes while another is
///    shown; and false after [`consent::TIMEOUT_SECS`] without an answer.
///    The request says it comes from remote access, and its default is no:
///    no key typed before it appeared answers it.
/// 3. While a consent it granted lasts, the agent shows the user who sees
///    or controls their session, with a way to stop it. When the user stops
///    it, the agent calls `fr.ameleo.Host.Consent1.Stopped(id)`.
/// 4. The host calls `End(id)` when what `id` granted is over (the person
///    left, or the host ended it), or to withdraw a request not answered
///    yet. The agent then removes what it shows for `id`, and sends nothing
///    more for it.
///
/// The host asks `CONTROL` only for a person whose `VIEW` consent lasts.
///
/// A consent belongs to the user who gave it, in their session: when the
/// session locks, its user signs out, or another user's session takes the
/// screen, the agent calls `Stopped` for every consent it granted. A new
/// D-Bus session drops them all, on both sides, without a call. Stopping a
/// view consent stops the control consent granted to the same person too:
/// the host ends both.
///
/// The agent lists `fr.ameleo.Agent.Consent1` in `Agent1.Channels` when it
/// implements it; the host asks nothing of an agent that does not.
#[zbus::proxy(
    interface = "fr.ameleo.Agent.Consent1",
    default_service = "fr.ameleo.Agent",
    default_path = "/fr/ameleo/Agent"
)]
pub trait Consent {
    /// Asks the signed-in user whether `who` may do `kind`. Returns at once;
    /// the answer comes through `fr.ameleo.Host.Consent1.Answered`.
    fn ask(&self, id: u32, kind: u32, who: &str) -> zbus::Result<()>;
    /// What `id` granted is over, or its request is withdrawn.
    fn end(&self, id: u32) -> zbus::Result<()>;
}

/// The host side of [`ConsentProxy`]: the user's answers, and their stops.
#[zbus::proxy(
    interface = "fr.ameleo.Host.Consent1",
    default_service = "fr.ameleo.Host",
    default_path = "/fr/ameleo/Host"
)]
pub trait HostConsent {
    /// The answer to `Ask(id)`, sent once.
    fn answered(&self, id: u32, granted: bool) -> zbus::Result<()>;
    /// The user stopped what `id` granted, or it no longer holds (the
    /// session locked, or its user changed).
    fn stopped(&self, id: u32) -> zbus::Result<()>;
}
