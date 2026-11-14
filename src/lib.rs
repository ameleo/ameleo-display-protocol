// SPDX-License-Identifier: MIT OR Apache-2.0

//! D-Bus contract between the guest agent and the host.
//!
//! # Transport
//!
//! A peer-to-peer D-Bus connection running over the virtio-serial port
//! [`PORT_NAME`]. On the host the port is a `-chardev dbus` that QEMU exposes
//! as `org.qemu.Display1.Chardev`:
//!
//! ```text
//! -device virtio-serial-pci,id=virtserial1
//! -chardev dbus,id=agent0,name=fr.ameleo.agent.0
//! -device virtserialport,name=fr.ameleo.agent.0,chardev=agent0,bus=virtserial1.0,nr=1
//! ```
//!
//! The host is the D-Bus client (it starts the SASL handshake with `AUTH
//! ANONYMOUS`), the guest agent is the D-Bus server. A new handshake on the
//! stream always starts a new session: the guest drops all per-session state
//! (clipboard ownership, transfers) when it sees one.
//!
//! # Objects
//!
//! * The guest exports [`AGENT_PATH`] with the core interface and one
//!   interface per channel.
//! * The host exports [`HOST_PATH`] with the interfaces the guest calls back
//!   (clipboard data requests).
//!
//! # Conventions
//!
//! * Text is UTF-8 with `\n` line endings on the wire, the guest converts.
//! * Mouse buttons use the QEMU `InputButton` numbering, see [`Button`].
//! * Interfaces are versioned by name (`...1`), an incompatible change gets a
//!   new interface, never a changed signature.

/// virtio-serial port name (`-device virtserialport,name=...`).
pub const PORT_NAME: &str = "fr.ameleo.agent.0";

/// Object path of everything the guest agent exports.
pub const AGENT_PATH: &str = "/fr/ameleo/Agent";

/// Object path of everything the host exports.
pub const HOST_PATH: &str = "/fr/ameleo/Host";

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
}

/// Clipboard MIME types understood by the guest agent.
pub mod mime {
    pub const TEXT: &str = "text/plain;charset=utf-8";
    pub const PNG: &str = "image/png";
    pub const BMP: &str = "image/bmp";
    /// File list, MS-RDPECLIP style. The type name is the Windows / RDP
    /// format name, the data is a CLIPRDR_FILELIST (MS-RDPECLIP 2.2.5.2.3):
    /// `cItems: u32 LE` then `cItems` FILEDESCRIPTORW of 592 bytes. Names
    /// are relative, `\` separated, directories come before their content.
    /// File data is pulled with `FileClipboard1.FileContents`.
    pub const FILE_LIST: &str = "FileGroupDescriptorW";
}

/// File clipboard constants (MS-RDPECLIP 2.2.5.3).
pub mod file_contents {
    /// `dwFlags`: return the file size, 8 bytes little endian.
    pub const SIZE: u32 = 0x1;
    /// `dwFlags`: return `requested` bytes from `position`.
    pub const RANGE: u32 = 0x2;
    /// Largest `requested` honored in one call.
    pub const MAX_RANGE: u32 = 4 * 1024 * 1024;
    /// Size of one FILEDESCRIPTORW.
    pub const DESCRIPTOR_SIZE: usize = 592;
}

/// Maximum payload of one `FileTransfer1.Write` call.
pub const FILE_CHUNK_MAX: usize = 256 * 1024;

/// Mouse buttons, same numbering as QEMU `InputButton`.
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

/// Errors returned by the guest agent.
#[derive(zbus::DBusError, Debug)]
#[zbus(prefix = "fr.ameleo.Agent.Error")]
pub enum Error {
    #[zbus(error)]
    ZBus(zbus::Error),
    /// The user session is locked, the request is refused.
    SessionLocked(String),
    /// Not enough disk space, the message carries the free byte count.
    NotEnoughSpace(String),
    InvalidArgument(String),
    NotFound(String),
    /// The guest clipboard does not hold the requested data anymore.
    NoData(String),
    Failed(String),
}

pub type Result<T> = std::result::Result<T, Error>;

/// Core interface: identity and channel discovery.
#[zbus::proxy(
    interface = "fr.ameleo.Agent1",
    default_service = "fr.ameleo.Agent",
    default_path = "/fr/ameleo/Agent",
    gen_blocking = false
)]
pub trait Agent {
    /// Guest agent version.
    #[zbus(property)]
    fn version(&self) -> zbus::Result<String>;

    /// Channel interfaces implemented by this agent.
    #[zbus(property)]
    fn channels(&self) -> zbus::Result<Vec<String>>;

    /// True while the Windows session is locked (or on the logon screen).
    #[zbus(property)]
    fn session_locked(&self) -> zbus::Result<bool>;

    /// The remote client went away: drop clipboard ownership, abort
    /// transfers, remove the client cameras and printers, lock the Windows
    /// session, without tearing down the D-Bus session. Call it when the
    /// remote client disconnects.
    fn reset(&self) -> zbus::Result<()>;
}

/// Absolute pointer, multi-head aware.
///
/// Coordinates are relative to the head (`head` = virtio-gpu scanout index,
/// the same index as the QEMU console of that head).
#[zbus::proxy(
    interface = "fr.ameleo.Agent.Mouse1",
    default_service = "fr.ameleo.Agent",
    default_path = "/fr/ameleo/Agent",
    gen_blocking = false
)]
pub trait Mouse {
    #[zbus(no_reply)]
    fn set_abs_position(&self, head: u32, x: u32, y: u32) -> zbus::Result<()>;

    #[zbus(no_reply)]
    fn press(&self, button: u32) -> zbus::Result<()>;

    #[zbus(no_reply)]
    fn release(&self, button: u32) -> zbus::Result<()>;

    /// High resolution scroll, in Windows wheel units (120 = one notch).
    #[zbus(no_reply)]
    fn scroll(&self, dx: i32, dy: i32) -> zbus::Result<()>;
}

/// Clipboard, guest side.
///
/// Ownership model (same as QEMU `Display1.Clipboard`): whoever copied last
/// owns the clipboard and announces the MIME types it can provide. Data is
/// pulled on demand by the other side.
#[zbus::proxy(
    interface = "fr.ameleo.Agent.Clipboard1",
    default_service = "fr.ameleo.Agent",
    default_path = "/fr/ameleo/Agent",
    gen_blocking = false
)]
pub trait Clipboard {
    /// The host now owns the clipboard. Data will be fetched from
    /// `fr.ameleo.Host.Clipboard1.Request` when a guest app pastes.
    fn grab(&self, serial: u32, mimes: Vec<String>) -> zbus::Result<()>;

    /// The host gives up its ownership.
    fn release(&self) -> zbus::Result<()>;

    /// Fetch guest clipboard data in one of the announced types.
    fn request(&self, mimes: Vec<String>) -> zbus::Result<(String, Vec<u8>)>;

    /// Largest payload the host accepts, -1 for no limit.
    #[zbus(property)]
    fn max_size(&self) -> zbus::Result<i32>;
    #[zbus(property)]
    fn set_max_size(&self, value: i32) -> zbus::Result<()>;

    /// The guest clipboard changed.
    #[zbus(signal)]
    fn grabbed(&self, serial: u32, mimes: Vec<String>) -> zbus::Result<()>;

    /// The guest clipboard was emptied or holds nothing exportable anymore.
    #[zbus(signal)]
    fn released(&self) -> zbus::Result<()>;
}

/// Clipboard, host side, called by the guest on paste.
#[zbus::proxy(
    interface = "fr.ameleo.Host.Clipboard1",
    default_service = "fr.ameleo.Host",
    default_path = "/fr/ameleo/Host",
    gen_blocking = false
)]
pub trait HostClipboard {
    fn request(&self, serial: u32, mimes: Vec<String>) -> zbus::Result<(String, Vec<u8>)>;
}

/// File contents for a [`mime::FILE_LIST`] clipboard, the same calls as the
/// MS-RDPECLIP File Contents Request and Lock/Unlock Clipboard Data PDUs.
///
/// Implemented by the guest (`fr.ameleo.Agent.FileClipboard1`, files copied
/// in the guest) and by the host (`fr.ameleo.Host.FileClipboard1`, files
/// copied on the client and pasted in the guest), with the same signature.
///
/// * `lindex` is the index in the file list returned for the grab.
/// * `flags` is [`file_contents::SIZE`] or [`file_contents::RANGE`].
/// * With `has_clip_data_id`, the request is about the clipboard content
///   frozen by `LockClipData(clip_data_id)`, otherwise the current one.
#[zbus::proxy(
    interface = "fr.ameleo.Agent.FileClipboard1",
    default_service = "fr.ameleo.Agent",
    default_path = "/fr/ameleo/Agent",
    gen_blocking = false
)]
pub trait FileClipboard {
    /// Freezes the current clipboard content under `clip_data_id`: file
    /// contents stay readable after the clipboard changes, until unlocked.
    /// May come before the file list was requested.
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

/// Host side of [`FileClipboard`], called by the guest when an app pastes
/// files the host announced.
#[zbus::proxy(
    interface = "fr.ameleo.Host.FileClipboard1",
    default_service = "fr.ameleo.Host",
    default_path = "/fr/ameleo/Host",
    gen_blocking = false
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

/// Camera limits of the guest.
pub mod camera {
    /// Client cameras shown at the same time.
    pub const MAX_CAMERAS: u32 = 4;
    /// Largest frame size, per camera format.
    pub const MAX_WIDTH: u32 = 3840;
    pub const MAX_HEIGHT: u32 = 2160;
}

/// Client webcams (MS-RDPECAM on the RDP side), shown in the guest as
/// Windows 11 virtual cameras.
///
/// Flow:
/// 1. The client announces a camera: the host calls `AddCamera` with the
///    H.264 formats the camera offers. The camera shows up in the guest,
///    nothing streams yet.
/// 2. An app opens it: the guest emits `StreamRequested(id, format)`, the
///    host starts that stream on the client (Start Streams) and pushes each
///    H.264 access unit with `PushFrame`.
/// 3. The app closes it: `StreamStopped(id)`, the host stops the stream.
/// 4. `KeyFrameNeeded(id)`: the guest decoder lost track (it started on a
///    non-IDR frame or a frame was lost). The host restarts the client
///    stream to get an IDR.
/// 5. The camera goes away on the client: `RemoveCamera(id)`.
///
/// Controls (brightness, focus... MS-RDPECAM version 2): after `AddCamera`,
/// the host gives what the client offers (Property List Response) and the
/// current values (Property Value Response) with `SetControls`. An app
/// changing one emits `ControlRequested`: the host sends the Set Property
/// Value Request, then reports the value the camera took with
/// `ControlValue` (on an Error Response, the value it still has). Sets,
/// property ids, capabilities and modes are MS-RDPECAM's, see
/// [`camera_control`].
///
/// Ids are chosen by the host. Formats are indexes into the list given to
/// `AddCamera`; sizes are even and at most [`camera::MAX_WIDTH`] x
/// [`camera::MAX_HEIGHT`].
#[zbus::proxy(
    interface = "fr.ameleo.Agent.Camera1",
    default_service = "fr.ameleo.Agent",
    default_path = "/fr/ameleo/Agent",
    gen_blocking = false
)]
pub trait Camera {
    /// `formats`: `(width, height, fps numerator, fps denominator)`, H.264.
    /// At most [`camera::MAX_CAMERAS`] cameras at once.
    fn add_camera(
        &self,
        id: u32,
        name: &str,
        formats: Vec<(u32, u32, u32, u32)>,
    ) -> zbus::Result<()>;

    fn remove_camera(&self, id: u32) -> zbus::Result<()>;

    /// One H.264 access unit (Annex B, start codes, SPS/PPS in band before
    /// each IDR), in the format of the last `StreamRequested`.
    /// `timestamp_us`: capture time on the client, microseconds.
    #[zbus(no_reply)]
    fn push_frame(&self, id: u32, timestamp_us: u64, data: Vec<u8>) -> zbus::Result<()>;

    /// An app opened camera `id` with format index `format`.
    #[zbus(signal)]
    fn stream_requested(&self, id: u32, format: u32) -> zbus::Result<()>;

    /// No app uses camera `id` anymore.
    #[zbus(signal)]
    fn stream_stopped(&self, id: u32) -> zbus::Result<()>;

    /// The guest needs an IDR frame to decode camera `id` again.
    #[zbus(signal)]
    fn key_frame_needed(&self, id: u32) -> zbus::Result<()>;

    /// The controls of camera `id`, replacing any given before: at most
    /// [`camera_control::MAX_CONTROLS`] [`camera_control::Control`].
    fn set_controls(&self, id: u32, controls: Vec<camera_control::Control>) -> zbus::Result<()>;

    /// What the client says a control of camera `id` is now (after a Set
    /// Property Value Request, or any time).
    fn control_value(
        &self,
        id: u32,
        set: u8,
        property: u8,
        mode: u8,
        value: i32,
    ) -> zbus::Result<()>;

    /// An app changed a control: send the client a Set Property Value
    /// Request. `value` means nothing when `mode` is
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

/// Camera controls, numbered as in MS-RDPECAM (PROPERTY_DESCRIPTION,
/// PROPERTY_VALUE): the host passes the client's values through.
pub mod camera_control {
    /// `(set, property, capabilities, min, max, step, default, mode,
    /// value)`: a PROPERTY_DESCRIPTION, then the PROPERTY_VALUE the client
    /// gave for it.
    pub type Control = (u8, u8, u8, i32, i32, i32, i32, u8, i32);

    pub const MAX_CONTROLS: usize = 16;

    /// Sets.
    pub const CAMERA_CONTROL: u8 = 0x01;
    pub const VIDEO_PROC_AMP: u8 = 0x02;

    /// CameraControl properties.
    pub const EXPOSURE: u8 = 0x01;
    pub const FOCUS: u8 = 0x02;
    pub const PAN: u8 = 0x03;
    pub const ROLL: u8 = 0x04;
    pub const TILT: u8 = 0x05;
    pub const ZOOM: u8 = 0x06;

    /// VideoProcAmp properties.
    pub const BACKLIGHT_COMPENSATION: u8 = 0x01;
    pub const BRIGHTNESS: u8 = 0x02;
    pub const CONTRAST: u8 = 0x03;
    pub const HUE: u8 = 0x04;
    pub const WHITE_BALANCE: u8 = 0x05;

    /// Capabilities flags and modes.
    pub const MANUAL: u8 = 0x01;
    pub const AUTO: u8 = 0x02;
}

/// Printer limits of the guest.
pub mod printer {
    /// Client printers shown at the same time.
    pub const MAX_PRINTERS: u32 = 32;
    /// Largest `AddPrinter` PrintCapabilities or PrintTicket, in bytes.
    pub const MAX_XML: usize = 4 * 1024 * 1024;
    /// Largest job document chunk in one `WriteJob` call.
    pub const CHUNK_MAX: usize = 256 * 1024;
}

/// Client printers (RDP printer redirection, Easy Print), shown in the
/// guest as Windows printers.
///
/// Each client printer is a virtual printer of the guest's Print Support
/// App, on the inbox Microsoft Virtual Print Class Driver: no third-party
/// driver, it works under Windows protected print. The app hands each job
/// to the agent as OpenXPS.
///
/// The client driver's options show in the guest: the printer's
/// capabilities are made out of the PrintCapabilities the host gives in
/// `AddPrinter`, and its printing preferences open the client driver's own
/// dialog on the client ([`HostPrinter::document_properties`]). The settings
/// the user chooses there stay with the guest printer and come back with
/// every job.
///
/// Flow:
/// 1. The client announces a printer: the host calls `AddPrinter`. The call
///    returns at once; the guest installs the printer for the logged on
///    user (at once, or when the user logs on).
/// 2. Apps print: `BeginJob`, `WriteJob`..., `EndJob` on [`HostPrinter`].
/// 3. `RemovePrinter(id)` when the client printer goes away: deleted at
///    once. At the end of the D-Bus session (or `Agent1.Reset`) the printers
///    are kept 60 s, unusable, and reused by a printer of the same name of
///    the next session (the user's settings with them), then deleted.
///
/// Ids are chosen by the host.
#[zbus::proxy(
    interface = "fr.ameleo.Agent.Printer1",
    default_service = "fr.ameleo.Agent",
    default_path = "/fr/ameleo/Agent",
    gen_blocking = false
)]
pub trait Printer {
    /// `name`: shown in the guest. `is_default`: the client's default
    /// printer, made the default printer of the guest user.
    ///
    /// `capabilities`: the client driver's PrintCapabilities XML (MS-RDPEXPS
    /// `GetPrintCapabilities`). `default_ticket`: its default PrintTicket
    /// (the ticket of its default DEVMODE): the guest printer's defaults;
    /// the capabilities do not say which option is the default, without it
    /// the guest guesses (one-sided, portrait...). Either may be empty (no
    /// capabilities: the guest printer has generic options only). At most
    /// [`printer::MAX_XML`] bytes each.
    fn add_printer(
        &self,
        id: u32,
        name: &str,
        is_default: bool,
        capabilities: &str,
        default_ticket: &str,
    ) -> zbus::Result<()>;

    fn remove_printer(&self, id: u32) -> zbus::Result<()>;

    /// Answer to [`HostPrinter::document_properties`]: the PrintTicket the
    /// user chose in the client driver dialog, or `cancelled`.
    fn document_properties_done(
        &self,
        request_id: u32,
        print_ticket: &str,
        cancelled: bool,
    ) -> zbus::Result<()>;
}

/// Host side of [`Printer`]: the client printers.
///
/// A job document comes as `BeginJob`, `WriteJob` for each chunk in order
/// (at most [`printer::CHUNK_MAX`] bytes; flow control: the next chunk
/// follows the reply), then `EndJob` once the guest has it all. `AbortJob`
/// drops a job (cancelled in the guest, connection lost). Job ids are
/// chosen by the guest, unique for the agent's lifetime.
///
/// PrintTickets are XML (Print Schema) for the client driver: the standard
/// settings (`psk:`) and the client driver's own ones, in its namespaces
/// (their prefixes may differ from the client's: the namespace URIs are what
/// counts).
#[zbus::proxy(
    interface = "fr.ameleo.Host.Printer1",
    default_service = "fr.ameleo.Host",
    default_path = "/fr/ameleo/Host",
    gen_blocking = false
)]
pub trait HostPrinter {
    /// `document_format`: the job's MIME type, `application/oxps`: write the
    /// document to the client's Easy Print printer as it is.
    /// `job_name`, `user_name`: as the guest app and user gave them, may be
    /// empty. `print_ticket`: the settings to print with, for the client
    /// driver: the user's, from the guest dialogs and the client driver
    /// dialog, among them its `PageDevmodeSnapshot` (in the driver's
    /// `oemdriverpt` namespace: its whole DEVMODE, base64) when it has one.
    /// The document already carries it as its job PrintTicket
    /// (`Metadata/Job_PT.xml`); given here for information.
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
    /// The whole document was written; returns once the host took it.
    fn end_job(&self, job_id: u32) -> zbus::Result<()>;
    fn abort_job(&self, job_id: u32) -> zbus::Result<()>;

    /// Shows the client driver's own document properties dialog on the
    /// client (MS-RDPEXPS `ASYNC_DOC_PROPS_REQ`), starting from
    /// `print_ticket` (the guest printer's current settings, as in
    /// [`HostPrinter::begin_job`]: the client driver's ticket the user chose
    /// last, its `PageDevmodeSnapshot` being the DEVMODE to start the dialog
    /// from). Returns at once; the host calls
    /// [`Printer::document_properties_done`] with `request_id` and the
    /// client driver's PrintTicket when the user closes the dialog, however
    /// long it stays open. The guest keeps it: the next jobs carry it.
    fn document_properties(
        &self,
        printer_id: u32,
        request_id: u32,
        print_ticket: &str,
    ) -> zbus::Result<()>;
}

/// Drive limits and wire types.
pub mod drive {
    /// Client drives shown at the same time.
    pub const MAX_DRIVES: u32 = 24;
    /// Largest `Read` length or `Write` data, in bytes.
    pub const IO_MAX: u32 = 1024 * 1024;

    /// `information` of [`super::HostDrive::open`]: what the client did, the
    /// MS-RDPEFS `DR_CREATE_RSP` value. That spec has no `CREATED`: it
    /// says `SUPERSEDED` for "a new file was created" (and when the field
    /// is left out). `CREATED` is the NT `IO_STATUS_BLOCK` value some
    /// clients send; both mean created, see [`created`].
    pub mod information {
        pub const SUPERSEDED: u32 = 0;
        pub const OPENED: u32 = 1;
        pub const CREATED: u32 = 2;
        pub const OVERWRITTEN: u32 = 3;

        /// The client created the file.
        pub fn created(information: u32) -> bool {
            information == SUPERSEDED || information == CREATED
        }
    }

    /// `action` of a [`super::Drive::changed`] change, the
    /// `FILE_NOTIFY_INFORMATION` values.
    pub mod action {
        pub const ADDED: u32 = 1;
        pub const REMOVED: u32 = 2;
        pub const MODIFIED: u32 = 3;
        pub const RENAMED_OLD_NAME: u32 = 4;
        pub const RENAMED_NEW_NAME: u32 = 5;
    }

    /// A file or directory: `(name, attributes, size, creation_time,
    /// last_access_time, last_write_time, change_time)`. `name` is the
    /// entry's own name in a listing, the path's last component otherwise;
    /// `attributes` the `FILE_ATTRIBUTE_*` flags (`0x10` directory); times
    /// are FILETIME (100 ns since 1601, UTC), 0 when unknown. D-Bus
    /// signature `(suttttt)`.
    pub type Entry = (String, u32, u64, u64, u64, u64, u64);

    /// A change on the client drive: `(action, path)`, see [`action`].
    pub type Change = (u32, String);
}

/// Client drives (RDP drive redirection, MS-RDPEFS), shown in the guest as
/// drive letters.
///
/// The guest mounts each drive with Dokany (a user mode file system, LGPL,
/// installed separately): every file operation of a guest app goes to the
/// client as it happens ([`HostDrive`]), like on a Windows RDP server. No
/// copy of the client's files stays in the guest. Without Dokany installed,
/// `AddDrive` fails and the guest has no drives.
///
/// Flow:
/// 1. The client announces a drive (`on_device_announce`): the host calls
///    `AddDrive`. The drive is mounted on a free letter (from `Z:` down) for
///    the logged on user only: other sessions get access denied.
/// 2. Guest apps use it through [`HostDrive`].
/// 3. Changes made on the client (`NotifyChangeDirectory` on the drive
///    root, watching the tree): the host calls `Changed`, the guest tells
///    Explorer.
/// 4. `RemoveDrive(id)` when the client drive goes away. At the end of the
///    D-Bus session (or `Agent1.Reset`) all drives are unmounted.
///
/// Ids are chosen by the host (the RDPDR `DeviceId` fits).
///
/// Paths are relative to the drive root, `\` separated, without a leading
/// `\`; `""` is the root.
#[zbus::proxy(
    interface = "fr.ameleo.Agent.Drive1",
    default_service = "fr.ameleo.Agent",
    default_path = "/fr/ameleo/Agent",
    gen_blocking = false
)]
pub trait Drive {
    /// `name`: the client's name of the drive (RDPDR `PreferredDosName`,
    /// `C`). `label`: the volume label shown in the guest (`C sur
    /// PC-CLIENT`). Returns the guest drive letter (`Z`).
    fn add_drive(&self, id: u32, name: &str, label: &str) -> zbus::Result<String>;

    fn remove_drive(&self, id: u32) -> zbus::Result<()>;

    /// Changes made on the client drive, in order, as the client reported
    /// them (`FILE_NOTIFY_INFORMATION`, paths relative to the drive root).
    /// A client that cannot watch the tree sends nothing: Explorer then
    /// shows the client's changes when it lists a directory again.
    fn changed(&self, id: u32, changes: Vec<drive::Change>) -> zbus::Result<()>;
}

/// Host side of [`Drive`]: the client drive's files, one RDPDR request each
/// (MS-RDPEFS, `ironrdp-rdpdr`'s server side).
///
/// Every method returns the client's `NTSTATUS` first (`0` success,
/// `0xC0000034` not found, `0xC0000022` access denied, `0xC0000043` sharing
/// violation...), as the client answered: the guest gives it to the app. A
/// D-Bus error means the host could not ask the client at all (no such
/// drive, client gone).
///
/// Calls may be pending at the same time, on the same drive and the same
/// file too: the host sends each to the client at once and answers when the
/// client did.
///
/// * `Open`: `IRP_MJ_CREATE` with the guest's own NT parameters, as they
///   are; `Close`: `IRP_MJ_CLOSE`.
/// * `Read` / `Write` / `Flush`: `IRP_MJ_READ` / `WRITE` / `FLUSH_BUFFERS`.
/// * `Query`: `IRP_MJ_QUERY_INFORMATION`, `FileBasicInformation` and
///   `FileStandardInformation`.
/// * `SetBasic`: `FileBasicInformation`; `SetSize`: `FileEndOfFileInformation`
///   (or `FileAllocationInformation`); `SetDelete`:
///   `FileDispositionInformation`; `Rename`: `FileRenameInformation`; all
///   `IRP_MJ_SET_INFORMATION` on the open file.
/// * `List`: its own create on the directory (`FILE_LIST_DIRECTORY`,
///   `FILE_DIRECTORY_FILE`, all sharing), `IRP_MN_QUERY_DIRECTORY` with
///   `FileBothDirectoryInformation` from the initial query until
///   `STATUS_NO_MORE_FILES`, close. `.` and `..` left out.
/// * `Volume`: its own create on the root, `IRP_MJ_QUERY_VOLUME_INFORMATION`
///   with `FileFsFullSizeInformation`, `FileFsVolumeInformation` and
///   `FileFsAttributeInformation`, close.
#[zbus::proxy(
    interface = "fr.ameleo.Host.Drive1",
    default_service = "fr.ameleo.Host",
    default_path = "/fr/ameleo/Host",
    gen_blocking = false
)]
pub trait HostDrive {
    /// `IRP_MJ_CREATE`: `desired_access`, `file_attributes`, `share_access`,
    /// `create_disposition` (`FILE_OPEN` 1, `FILE_CREATE` 2, `FILE_OPEN_IF`
    /// 3...) and `create_options` (`FILE_DIRECTORY_FILE`...) as the guest app
    /// gave them, for `DR_CREATE_REQ`. Returns `(status, file_id,
    /// information)`, see [`drive::information`].
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

    /// At most `length` bytes ([`drive::IO_MAX`]) from `offset`; fewer at
    /// the end of the file, none past it.
    fn read(
        &self,
        drive_id: u32,
        file_id: u32,
        offset: u64,
        length: u32,
    ) -> zbus::Result<(u32, Vec<u8>)>;

    /// Writes `data` (at most [`drive::IO_MAX`] bytes) at `offset`; returns
    /// the byte count written.
    fn write(
        &self,
        drive_id: u32,
        file_id: u32,
        offset: u64,
        data: Vec<u8>,
    ) -> zbus::Result<(u32, u32)>;

    fn flush(&self, drive_id: u32, file_id: u32) -> zbus::Result<u32>;

    fn query(&self, drive_id: u32, file_id: u32) -> zbus::Result<(u32, drive::Entry)>;

    /// `attributes` 0 and times 0 leave them as they are.
    fn set_basic(
        &self,
        drive_id: u32,
        file_id: u32,
        attributes: u32,
        creation_time: u64,
        last_access_time: u64,
        last_write_time: u64,
    ) -> zbus::Result<u32>;

    /// The end of file, or with `allocation` the allocation size.
    fn set_size(
        &self,
        drive_id: u32,
        file_id: u32,
        size: u64,
        allocation: bool,
    ) -> zbus::Result<u32>;

    /// Deleted when the client closes the file (`DeleteFile` true), or not
    /// anymore. A non-empty directory answers `STATUS_DIRECTORY_NOT_EMPTY`.
    fn set_delete(&self, drive_id: u32, file_id: u32, delete: bool) -> zbus::Result<u32>;

    /// `new_path` from the drive root; `replace`: an existing one is
    /// replaced.
    fn rename(
        &self,
        drive_id: u32,
        file_id: u32,
        new_path: &str,
        replace: bool,
    ) -> zbus::Result<u32>;

    /// The entries of the directory `path`.
    fn list(&self, drive_id: u32, path: &str) -> zbus::Result<(u32, Vec<drive::Entry>)>;

    /// `(status, total_bytes, free_bytes, label, file_system)`: the client
    /// volume's size, free space for the user, label and file system name
    /// (`NTFS`).
    fn volume(&self, drive_id: u32) -> zbus::Result<(u32, u64, u64, String, String)>;
}

/// Desktop tuning for remote sessions.
///
/// There is deliberately no monitor configuration here: resolution and
/// layout belong to the display driver (EDID / hotplug), the agent never
/// calls `SetDisplayConfig`.
#[zbus::proxy(
    interface = "fr.ameleo.Agent.Display1",
    default_service = "fr.ameleo.Agent",
    default_path = "/fr/ameleo/Agent",
    gen_blocking = false
)]
pub trait Display {
    /// Persisted across reboots and re-applied at every logon.
    fn set_effects(
        &self,
        disable_wallpaper: bool,
        disable_font_smoothing: bool,
        disable_animation: bool,
    ) -> zbus::Result<()>;
}

/// Host to guest file push, saved into the user Downloads folder.
///
/// Flow control comes from the method replies: the host sends the next
/// chunk once `Write` returned.
#[zbus::proxy(
    interface = "fr.ameleo.Agent.FileTransfer1",
    default_service = "fr.ameleo.Agent",
    default_path = "/fr/ameleo/Agent",
    gen_blocking = false
)]
pub trait FileTransfer {
    /// Returns the transfer id.
    fn start(&self, name: &str, size: u64) -> zbus::Result<u32>;
    /// At most [`FILE_CHUNK_MAX`] bytes per call.
    fn write(&self, id: u32, data: Vec<u8>) -> zbus::Result<()>;
    /// Checks the size, keeps the file and opens the folder in Explorer.
    /// Returns the final file name (it may carry a ` (n)` suffix).
    fn finish(&self, id: u32) -> zbus::Result<String>;
    /// Deletes the partial file.
    fn cancel(&self, id: u32) -> zbus::Result<()>;
}
