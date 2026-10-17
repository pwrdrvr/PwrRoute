//! The wire protocol between a second launch and the running router.
//!
//! One request per connection, each message one line of JSON, at most
//! [`MAX_MESSAGE`] bytes. The socket is local and private to the user (see
//! [`crate::instance`]), so full URLs may cross it; they are never logged.

use std::io::{self, BufRead, Read as _, Write};

use serde::{Deserialize, Serialize, de::DeserializeOwned};

use crate::{link::Link, trace::DispatchId};

/// Bumped on any incompatible change. A router refuses other versions, and
/// the launch that asked then routes its links itself.
pub const PROTOCOL_VERSION: u32 = 1;

/// The largest message either side accepts: room for a few long links.
pub const MAX_MESSAGE: u64 = 1024 * 1024;

/// What a second launch asks of the running router.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "op", rename_all = "camelCase")]
pub enum Request {
    /// Route these links.
    Open { links: Vec<Link> },
    /// Launched with no links: show the window.
    Activate,
    /// Is anyone there?
    Ping,
}

/// The router's answer.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Response {
    pub ok: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
    /// The dispatch ids an `Open` queued, in order.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub dispatched: Vec<DispatchId>,
    /// The router's process id.
    pub pid: u32,
}

impl Response {
    pub fn ok() -> Self {
        Self {
            ok: true,
            error: None,
            dispatched: Vec::new(),
            pid: std::process::id(),
        }
    }

    pub fn error(message: impl Into<String>) -> Self {
        Self {
            ok: false,
            error: Some(message.into()),
            ..Self::ok()
        }
    }
}

#[derive(Serialize, Deserialize)]
struct Envelope<T> {
    v: u32,
    #[serde(flatten)]
    body: T,
}

/// Writes one message as a line of JSON.
pub fn write_message<T: Serialize>(w: &mut impl Write, message: &T) -> io::Result<()> {
    let mut line = serde_json::to_vec(&Envelope {
        v: PROTOCOL_VERSION,
        body: message,
    })?;
    if line.len() as u64 > MAX_MESSAGE {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "message is larger than 1 MiB",
        ));
    }
    line.push(b'\n');
    w.write_all(&line)?;
    w.flush()
}

/// Reads one message. A message of another protocol version is an error of
/// kind `InvalidData` that says so.
pub fn read_message<T: DeserializeOwned>(r: &mut impl BufRead) -> io::Result<T> {
    let mut line = Vec::new();
    r.by_ref()
        .take(MAX_MESSAGE + 1)
        .read_until(b'\n', &mut line)?;
    if line.len() as u64 > MAX_MESSAGE {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "message is larger than 1 MiB",
        ));
    }
    if line.is_empty() {
        return Err(io::Error::new(
            io::ErrorKind::UnexpectedEof,
            "the connection closed before a message",
        ));
    }
    #[derive(Deserialize)]
    struct Version {
        v: u32,
    }
    let version: Version = serde_json::from_slice(&line)?;
    if version.v != PROTOCOL_VERSION {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            format!(
                "protocol version {} (expected {PROTOCOL_VERSION})",
                version.v
            ),
        ));
    }
    let envelope: Envelope<T> = serde_json::from_slice(&line)?;
    Ok(envelope.body)
}

#[cfg(test)]
mod tests {
    use std::io::Cursor;

    use super::*;
    use crate::link::{OpenerInfo, Source};

    #[test]
    fn requests_round_trip() {
        let requests = [
            Request::Open {
                links: vec![
                    Link::new("https://example.com/a", Source::CommandLine),
                    Link::new("https://example.org/b", Source::AppleEvent).with_opener(Some(
                        OpenerInfo {
                            name: "Mail".into(),
                            bundle_id: "com.apple.mail".into(),
                            path: "/System/Applications/Mail.app".into(),
                            window_title: None,
                        },
                    )),
                ],
            },
            Request::Activate,
            Request::Ping,
        ];
        let mut wire = Vec::new();
        for request in &requests {
            write_message(&mut wire, request).unwrap();
        }
        let mut reader = Cursor::new(wire);
        for request in &requests {
            assert_eq!(&read_message::<Request>(&mut reader).unwrap(), request);
        }
    }

    #[test]
    fn the_wire_format_is_one_json_line() {
        let mut wire = Vec::new();
        write_message(&mut wire, &Request::Ping).unwrap();
        assert_eq!(wire, b"{\"v\":1,\"op\":\"ping\"}\n");
    }

    #[test]
    fn other_protocol_versions_are_refused() {
        let mut reader = Cursor::new(b"{\"v\":99,\"op\":\"ping\"}\n".to_vec());
        let err = read_message::<Request>(&mut reader).unwrap_err();
        assert_eq!(err.kind(), io::ErrorKind::InvalidData);
        assert!(err.to_string().contains("99"));
    }

    #[test]
    fn oversized_and_empty_messages_are_refused() {
        let huge = format!(
            "{{\"v\":1,\"op\":\"open\",\"links\":[{{\"url\":\"https://e.com/{}\"}}]}}\n",
            "a".repeat(usize::try_from(MAX_MESSAGE).unwrap())
        );
        let err = read_message::<Request>(&mut Cursor::new(huge.into_bytes())).unwrap_err();
        assert_eq!(err.kind(), io::ErrorKind::InvalidData);

        let err = read_message::<Request>(&mut Cursor::new(Vec::new())).unwrap_err();
        assert_eq!(err.kind(), io::ErrorKind::UnexpectedEof);
    }
}
