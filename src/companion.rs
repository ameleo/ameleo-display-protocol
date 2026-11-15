//! The companion link: a guest-local contract between the agent and a companion service.
//!
//! A companion is another service in the guest that implements more `fr.ameleo.Agent.*`
//! interfaces in its own process. The host never sees it: the agent keeps the
//! [`PORT_NAME`](crate::PORT_NAME) port, announces the companion's interfaces in
//! `Agent1.Channels` and relays calls in both directions, unchanged.
//!
//! # Transport
//!
//! On Windows, the agent creates the named pipe [`PIPE_NAME`] with the security descriptor
//! [`PIPE_SDDL`] (LocalSystem only) and as the first instance of that name. The companion
//! connects to it and both run a peer-to-peer D-Bus connection over it, the agent being the
//! server. One companion at a time: a new connection replaces the previous one, and the agent
//! drops the interfaces of a companion that disconnects. A companion should reconnect when the
//! agent restarts, and register again.
//!
//! Security requirements:
//!
//! * The pipe gives access to the guest clipboard and to every relayed interface. An agent
//!   must restrict it to LocalSystem, as [`PIPE_SDDL`] does, and must create it as the first
//!   instance, so that no other process holds the name.
//! * A companion should check, before it registers, that the pipe server is the agent: for
//!   example its process (`GetNamedPipeServerProcessId`) and the signature of its executable.
//!
//! # Objects
//!
//! * The agent serves [`CompanionLinkProxy`] (`fr.ameleo.CompanionLink1`) at [`LINK_PATH`]. The
//!   companion first calls [`CompanionLinkProxy::register`].
//! * The companion serves its `fr.ameleo.Agent.*` interfaces at
//!   [`AGENT_PATH`](crate::AGENT_PATH), calls the `fr.ameleo.Host.*` interfaces at
//!   [`HOST_PATH`](crate::HOST_PATH) through the agent, and serves [`CompanionProxy`]
//!   (`fr.ameleo.Companion1`) at [`COMPANION_PATH`].
//!
//! The agent waits a few seconds at most for a [`CompanionProxy`] call to return, and logs the
//! calls that fail.

/// The named pipe the companion connects to.
pub const PIPE_NAME: &str = r"\\.\pipe\ameleo-display-agent-companion";

/// The security descriptor of [`PIPE_NAME`], in SDDL: full access for LocalSystem only.
pub const PIPE_SDDL: &str = "D:P(A;;GA;;;SY)";

/// Where the agent serves `fr.ameleo.CompanionLink1` on the link.
pub const LINK_PATH: &str = "/fr/ameleo/CompanionLink";

/// Where the companion serves `fr.ameleo.Companion1` on the link.
pub const COMPANION_PATH: &str = "/fr/ameleo/Companion";

/// The most distinct interface names, agent and host together, the agent relays during its
/// lifetime.
pub const MAX_RELAYED_INTERFACES: usize = 16;

/// The most clipboard formats a companion can register.
pub const MAX_CLIPBOARD_FORMATS: usize = 8;

/// The interface names of the companion link.
pub mod iface {
    pub const LINK: &str = "fr.ameleo.CompanionLink1";
    pub const COMPANION: &str = "fr.ameleo.Companion1";
}

/// The link, agent side. The companion calls it.
#[zbus::proxy(
    interface = "fr.ameleo.CompanionLink1",
    default_service = "fr.ameleo.Agent",
    default_path = "/fr/ameleo/CompanionLink",
    gen_blocking = false
)]
pub trait CompanionLink {
    /// Registers the companion. `agent`: the `fr.ameleo.Agent.*` interfaces it serves at
    /// `/fr/ameleo/Agent`, which the agent announces and relays to it. `host`: the
    /// `fr.ameleo.Host.*` interfaces it calls at `/fr/ameleo/Host`, which the agent relays to
    /// the host. A new call replaces the previous registration.
    ///
    /// Fails with `fr.ameleo.Agent.Error.InvalidArgument` for a name outside these prefixes,
    /// for `fr.ameleo.Agent1`, for an interface the agent implements itself, or beyond
    /// [`MAX_RELAYED_INTERFACES`].
    fn register(&self, agent: Vec<String>, host: Vec<String>) -> zbus::Result<()>;

    /// The guest clipboard formats the companion handles, as (Windows clipboard format, MIME
    /// type) pairs, at most [`MAX_CLIPBOARD_FORMATS`]. A predefined format is written `#<id>`,
    /// such as `#15` for `CF_HDROP`; any other name is a registered format. For these formats,
    /// the agent announces the MIME type in `Clipboard1` and asks the companion for the data
    /// ([`CompanionProxy::read_clipboard`]); when the host's clipboard holds that MIME type,
    /// the companion fills the guest clipboard itself ([`CompanionProxy::host_grab`]).
    fn register_clipboard(&self, formats: Vec<(String, String)>) -> zbus::Result<()>;

    /// The processes whose guest clipboard changes are the companion's own, such as its helper
    /// in the user session: the agent does not take them for copies made by the user.
    fn set_clipboard_owners(&self, pids: Vec<u32>) -> zbus::Result<()>;
}

/// The companion side of the link. The agent calls it.
#[zbus::proxy(
    interface = "fr.ameleo.Companion1",
    default_service = "fr.ameleo.Companion",
    default_path = "/fr/ameleo/Companion",
    gen_blocking = false
)]
pub trait Companion {
    /// Whether a host is connected. Called when the companion connects, then on each change.
    fn host_connected(&self, connected: bool) -> zbus::Result<()>;

    /// Whether the user session is locked, or the logon screen shown. Called when the companion
    /// connects, then on each change.
    fn session_locked(&self, locked: bool) -> zbus::Result<()>;

    /// Drop the remote client's state. The agent calls it when the host calls `Agent1.Reset`,
    /// and when a new D-Bus session with the host starts, since a new handshake replaces the
    /// previous client.
    fn reset(&self) -> zbus::Result<()>;

    /// The agent is about to exit.
    fn stop(&self) -> zbus::Result<()>;

    /// The guest clipboard changed and holds a registered format. `serial` identifies the
    /// copy in later calls.
    fn clipboard_changed(&self, serial: u32) -> zbus::Result<()>;

    /// Makes the host's data in `mime` from `raw`, the guest clipboard's data in the
    /// registered format, read by the agent on the user's desktop. A
    /// `fr.ameleo.Agent.Error.*` error is passed to the host as it is.
    fn read_clipboard(&self, mime: &str, serial: u32, raw: Vec<u8>) -> zbus::Result<Vec<u8>>;

    /// The host's clipboard holds `mimes`, at least one of them registered: the companion puts
    /// the data on the guest clipboard itself, from one of its clipboard owner processes.
    fn host_grab(&self, serial: u32, mimes: Vec<String>) -> zbus::Result<()>;

    /// The host gave its clipboard up.
    fn host_release(&self) -> zbus::Result<()>;
}

#[cfg(test)]
mod tests {
    use super::*;
    use zbus::proxy::Defaults;

    #[test]
    fn proxies_match_the_constants() {
        assert_eq!(
            CompanionLinkProxy::INTERFACE.as_ref().unwrap().as_str(),
            iface::LINK
        );
        assert_eq!(
            CompanionLinkProxy::PATH.as_ref().unwrap().as_str(),
            LINK_PATH
        );
        assert_eq!(
            CompanionProxy::INTERFACE.as_ref().unwrap().as_str(),
            iface::COMPANION
        );
        assert_eq!(
            CompanionProxy::PATH.as_ref().unwrap().as_str(),
            COMPANION_PATH
        );
    }
}
