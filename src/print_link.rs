// SPDX-License-Identifier: MIT OR Apache-2.0

//! The guest-local link between a Windows agent and its Print Support App.
//!
//! On Windows, the client printers of `fr.ameleo.Agent.Printer1` are
//! virtual printers of a Print Support App (an MSIX package) on the inbox
//! Microsoft Virtual Print Class Driver, so that they work under Windows
//! protected print. The app is a separate program: this module is the
//! contract between it and the agent, and the host never sees it.
//!
//! The app and its background tasks connect to the agent's named pipe
//! [`PIPE_NAME`], send one [`Request`] and get one [`Reply`]. The agent
//! accepts only processes of the app's package, [`PACKAGE_FAMILY_NAME`],
//! whose executable is installed under `WindowsApps`: a package registered
//! from a folder in developer mode can claim any family name.
//!
//! Messages: `len: u32 LE` then `len` bytes: `kind: u8`, then strings as
//! `len: u32 LE` + UTF-8. A [`Request::Job`] is followed by the document in
//! chunks ([`write_chunk`]), an empty chunk at the end.

use std::io::{self, Read, Write};

pub const PIPE_NAME: &str = r"\\.\pipe\ameleo-display-agent-print";

/// The family name of the app's package (`Name_<publisher id>`, the id
/// derived from its publisher `CN=Ameleo`).
pub const PACKAGE_FAMILY_NAME: &str = "Ameleo.DisplayAgent.Printers_8ndmhckkynd76";
/// The URI scheme of the virtual printers: `ameleo-display-agent:<slot>`.
pub const URI_SCHEME: &str = "ameleo-display-agent";

/// Larger than any PrintTicket or PrintCapabilities.
const MAX_MESSAGE: usize = 16 * 1024 * 1024;
/// Largest document chunk.
pub const CHUNK_MAX: usize = 256 * 1024;

#[derive(Debug, Clone, PartialEq)]
pub enum Request {
    /// Show the client driver's document properties dialog for the guest
    /// printer `printer` (its Windows name), starting from `ticket`.
    DocumentProperties { printer: String, ticket: String },
    /// A job printed to the virtual printer `printer` (its URI), the
    /// document (OpenXPS) follows in chunks.
    Job {
        printer: String,
        title: String,
        ticket: String,
    },
    /// The print capabilities (PDC) of the virtual printer `printer` (URI).
    Capabilities { printer: String },
    /// The app, running in the user's session, attaches to the agent: it
    /// installs and removes the user's virtual printers for it (the agent,
    /// SYSTEM, cannot). `installed`: the ones it has. The connection stays:
    /// the agent sends [`Command`]s, the app answers each with a [`Reply`]
    /// (`Done` or `Failed`).
    Attach { installed: Vec<String> },
}

/// What the agent has the attached app do.
#[derive(Debug, Clone, PartialEq)]
pub enum Command {
    /// Install the virtual printer `name` with the URI `uri`.
    Install { name: String, uri: String },
    /// Remove the virtual printer `name`.
    Remove { name: String },
    /// Make `name` the user's default printer.
    SetDefault { name: String },
}

impl Command {
    pub fn write(&self, w: &mut impl Write) -> io::Result<()> {
        let mut p = Vec::new();
        match self {
            Command::Install { name, uri } => {
                p.push(1);
                put_str(&mut p, name);
                put_str(&mut p, uri);
            }
            Command::Remove { name } => {
                p.push(2);
                put_str(&mut p, name);
            }
            Command::SetDefault { name } => {
                p.push(3);
                put_str(&mut p, name);
            }
        }
        write_message(w, &p)
    }

    pub fn read(r: &mut impl Read) -> io::Result<Self> {
        let m = read_message(r)?;
        let mut i = In(&m);
        match i.u8()? {
            1 => Ok(Command::Install {
                name: i.str()?,
                uri: i.str()?,
            }),
            2 => Ok(Command::Remove { name: i.str()? }),
            3 => Ok(Command::SetDefault { name: i.str()? }),
            _ => Err(bad("unknown command")),
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub enum Reply {
    /// The PrintTicket the user chose.
    Ticket(String),
    Cancelled,
    Failed(String),
    /// The job was taken.
    Done,
    /// The client driver's PrintCapabilities XML (empty: none, the app keeps
    /// its package's) and default PrintTicket (empty: none).
    Capabilities {
        capabilities: String,
        default_ticket: String,
    },
}

fn put_str(out: &mut Vec<u8>, s: &str) {
    out.extend_from_slice(&(s.len() as u32).to_le_bytes());
    out.extend_from_slice(s.as_bytes());
}

fn bad(msg: &str) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, msg.to_string())
}

struct In<'a>(&'a [u8]);

impl In<'_> {
    fn take(&mut self, n: usize) -> io::Result<&[u8]> {
        if self.0.len() < n {
            return Err(bad("truncated message"));
        }
        let (a, b) = self.0.split_at(n);
        self.0 = b;
        Ok(a)
    }
    fn u8(&mut self) -> io::Result<u8> {
        Ok(self.take(1)?[0])
    }
    fn str(&mut self) -> io::Result<String> {
        let n = u32::from_le_bytes(self.take(4)?.try_into().unwrap()) as usize;
        String::from_utf8(self.take(n)?.to_vec()).map_err(|_| bad("string is not UTF-8"))
    }
}

fn write_message(w: &mut impl Write, payload: &[u8]) -> io::Result<()> {
    let mut m = Vec::with_capacity(4 + payload.len());
    m.extend_from_slice(&(payload.len() as u32).to_le_bytes());
    m.extend_from_slice(payload);
    w.write_all(&m)?;
    w.flush()
}

fn read_len(r: &mut impl Read) -> io::Result<usize> {
    let mut len = [0u8; 4];
    r.read_exact(&mut len)?;
    Ok(u32::from_le_bytes(len) as usize)
}

fn read_message(r: &mut impl Read) -> io::Result<Vec<u8>> {
    let len = read_len(r)?;
    if len == 0 || len > MAX_MESSAGE {
        return Err(bad("bad message length"));
    }
    let mut buf = vec![0u8; len];
    r.read_exact(&mut buf)?;
    Ok(buf)
}

/// One document chunk (at most [`CHUNK_MAX`] bytes); empty: the end.
pub fn write_chunk(w: &mut impl Write, data: &[u8]) -> io::Result<()> {
    if data.len() > CHUNK_MAX {
        return Err(bad("chunk too large"));
    }
    w.write_all(&(data.len() as u32).to_le_bytes())?;
    w.write_all(data)?;
    if data.is_empty() {
        w.flush()?;
    }
    Ok(())
}

/// The next document chunk; `None` at the end.
pub fn read_chunk(r: &mut impl Read) -> io::Result<Option<Vec<u8>>> {
    let len = read_len(r)?;
    if len == 0 {
        return Ok(None);
    }
    if len > CHUNK_MAX {
        return Err(bad("chunk too large"));
    }
    let mut buf = vec![0u8; len];
    r.read_exact(&mut buf)?;
    Ok(Some(buf))
}

impl Request {
    pub fn write(&self, w: &mut impl Write) -> io::Result<()> {
        let mut p = Vec::new();
        match self {
            Request::DocumentProperties { printer, ticket } => {
                p.push(1);
                put_str(&mut p, printer);
                put_str(&mut p, ticket);
            }
            Request::Job {
                printer,
                title,
                ticket,
            } => {
                p.push(2);
                put_str(&mut p, printer);
                put_str(&mut p, title);
                put_str(&mut p, ticket);
            }
            Request::Capabilities { printer } => {
                p.push(3);
                put_str(&mut p, printer);
            }
            Request::Attach { installed } => {
                p.push(4);
                p.extend_from_slice(&(installed.len() as u32).to_le_bytes());
                for n in installed {
                    put_str(&mut p, n);
                }
            }
        }
        write_message(w, &p)
    }

    pub fn read(r: &mut impl Read) -> io::Result<Self> {
        let m = read_message(r)?;
        let mut i = In(&m);
        match i.u8()? {
            1 => Ok(Request::DocumentProperties {
                printer: i.str()?,
                ticket: i.str()?,
            }),
            2 => Ok(Request::Job {
                printer: i.str()?,
                title: i.str()?,
                ticket: i.str()?,
            }),
            3 => Ok(Request::Capabilities { printer: i.str()? }),
            4 => {
                let n = u32::from_le_bytes(i.take(4)?.try_into().unwrap()) as usize;
                let installed = (0..n.min(1024))
                    .map(|_| i.str())
                    .collect::<io::Result<_>>()?;
                Ok(Request::Attach { installed })
            }
            _ => Err(bad("unknown request")),
        }
    }
}

impl Reply {
    pub fn write(&self, w: &mut impl Write) -> io::Result<()> {
        let mut p = Vec::new();
        match self {
            Reply::Ticket(t) => {
                p.push(1);
                put_str(&mut p, t);
            }
            Reply::Cancelled => p.push(2),
            Reply::Failed(e) => {
                p.push(3);
                put_str(&mut p, e);
            }
            Reply::Done => p.push(4),
            Reply::Capabilities {
                capabilities,
                default_ticket,
            } => {
                p.push(5);
                put_str(&mut p, capabilities);
                put_str(&mut p, default_ticket);
            }
        }
        write_message(w, &p)
    }

    pub fn read(r: &mut impl Read) -> io::Result<Self> {
        let m = read_message(r)?;
        let mut i = In(&m);
        match i.u8()? {
            1 => Ok(Reply::Ticket(i.str()?)),
            2 => Ok(Reply::Cancelled),
            3 => Ok(Reply::Failed(i.str()?)),
            4 => Ok(Reply::Done),
            5 => Ok(Reply::Capabilities {
                capabilities: i.str()?,
                default_ticket: i.str()?,
            }),
            _ => Err(bad("unknown reply")),
        }
    }
}

/// The slot of a virtual printer URI `ameleo-display-agent:<slot>`.
pub fn slot_of_uri(uri: &str) -> Option<u16> {
    let (scheme, rest) = uri.split_once(':')?;
    scheme
        .eq_ignore_ascii_case(URI_SCHEME)
        .then(|| rest.trim_matches('/').parse().ok())
        .flatten()
}

pub fn uri_of_slot(slot: u16) -> String {
    format!("{URI_SCHEME}:{slot}")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trips() {
        let reqs = [
            Request::DocumentProperties {
                printer: "SHARP (redirected)".into(),
                ticket: "<psf:PrintTicket/>".into(),
            },
            Request::Job {
                printer: "ameleo-display-agent:3".into(),
                title: "doc".into(),
                ticket: "<t/>".into(),
            },
            Request::Capabilities {
                printer: "ameleo-display-agent:3".into(),
            },
            Request::Attach {
                installed: vec!["A (redirected)".into(), "B (redirected)".into()],
            },
            Request::Attach { installed: vec![] },
        ];
        for req in reqs {
            let mut buf = Vec::new();
            req.write(&mut buf).unwrap();
            assert_eq!(Request::read(&mut buf.as_slice()).unwrap(), req);
        }
        for reply in [
            Reply::Ticket("<t/>".into()),
            Reply::Cancelled,
            Reply::Failed("no session".into()),
            Reply::Done,
            Reply::Capabilities {
                capabilities: "<pc/>".into(),
                default_ticket: String::new(),
            },
        ] {
            let mut buf = Vec::new();
            reply.write(&mut buf).unwrap();
            assert_eq!(Reply::read(&mut buf.as_slice()).unwrap(), reply);
        }
        assert!(Reply::read(&mut [0u8, 0, 0, 0].as_slice()).is_err());
        assert!(Request::read(&mut [1u8, 0, 0, 0, 9].as_slice()).is_err());
    }

    #[test]
    fn commands() {
        for c in [
            Command::Install {
                name: "A (redirected)".into(),
                uri: "ameleo-display-agent:0".into(),
            },
            Command::Remove { name: "A".into() },
            Command::SetDefault { name: "A".into() },
        ] {
            let mut buf = Vec::new();
            c.write(&mut buf).unwrap();
            assert_eq!(Command::read(&mut buf.as_slice()).unwrap(), c);
        }
    }

    #[test]
    fn chunks() {
        let mut buf = Vec::new();
        write_chunk(&mut buf, b"abc").unwrap();
        write_chunk(&mut buf, b"").unwrap();
        let mut r = buf.as_slice();
        assert_eq!(read_chunk(&mut r).unwrap().as_deref(), Some(&b"abc"[..]));
        assert_eq!(read_chunk(&mut r).unwrap(), None);
    }

    #[test]
    fn uris() {
        assert_eq!(uri_of_slot(4), "ameleo-display-agent:4");
        assert_eq!(slot_of_uri("ameleo-display-agent:4"), Some(4));
        assert_eq!(slot_of_uri("AMELEO-DISPLAY-AGENT:4/"), Some(4));
        assert_eq!(slot_of_uri("other-printer:4"), None);
        assert_eq!(slot_of_uri("ameleo-display-agent:x"), None);
    }
}
