use serde::{Deserialize, Serialize};

/// What went wrong, in terms a person or an agent can act on.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ErrorKind {
    /// Not signed in, session expired or key rejected.
    Auth,
    /// The server could not be reached or the connection broke.
    Network,
    /// The server answered something we do not understand.
    Protocol,
    /// The server refused for a reason it named (content filter, not allowed).
    Refused,
    /// The model cannot answer right now: overloaded, a server error (5xx), a timeout, a rate
    /// limit or the quota used up. Another model may (#66).
    Busy,
    NotFound,
    /// Bad parameters from the UI, CLI, control channel or a file.
    Invalid,
    Unsupported,
    Storage,
    Cancelled,
}

#[derive(Debug, Clone, thiserror::Error, Serialize, Deserialize)]
#[error("{message}")]
pub struct Error {
    pub kind: ErrorKind,
    pub message: String,
}

pub type Result<T, E = Error> = std::result::Result<T, E>;

impl Error {
    pub fn new(kind: ErrorKind, message: impl Into<String>) -> Self {
        Self { kind, message: message.into() }
    }
    pub fn auth(m: impl Into<String>) -> Self {
        Self::new(ErrorKind::Auth, m)
    }
    pub fn network(m: impl Into<String>) -> Self {
        Self::new(ErrorKind::Network, m)
    }
    pub fn protocol(m: impl Into<String>) -> Self {
        Self::new(ErrorKind::Protocol, m)
    }
    pub fn refused(m: impl Into<String>) -> Self {
        Self::new(ErrorKind::Refused, m)
    }
    pub fn busy(m: impl Into<String>) -> Self {
        Self::new(ErrorKind::Busy, m)
    }
    pub fn not_found(m: impl Into<String>) -> Self {
        Self::new(ErrorKind::NotFound, m)
    }
    pub fn invalid(m: impl Into<String>) -> Self {
        Self::new(ErrorKind::Invalid, m)
    }
    pub fn unsupported(m: impl Into<String>) -> Self {
        Self::new(ErrorKind::Unsupported, m)
    }
    pub fn storage(m: impl Into<String>) -> Self {
        Self::new(ErrorKind::Storage, m)
    }
    pub fn cancelled() -> Self {
        Self::new(ErrorKind::Cancelled, "Abgebrochen")
    }
}

/// What AIS.ChatCraft says when the disk (or the person's quota) is full. Not a fault of the app,
/// so nothing to report: the person can fix it.
pub const DISK_FULL: &str = "Auf dem Rechner ist kein Speicherplatz mehr frei. Bitte Platz schaffen und noch einmal versuchen.";

/// OS error numbers for a full disk or quota, for errors that only carry the raw number.
#[cfg(any(target_os = "macos", target_os = "ios", target_os = "freebsd", target_os = "openbsd", target_os = "netbsd", target_os = "dragonfly"))]
const DISK_FULL_CODES: &[i32] = &[28, 69]; // ENOSPC, EDQUOT
#[cfg(all(
    unix,
    not(any(target_os = "macos", target_os = "ios", target_os = "freebsd", target_os = "openbsd", target_os = "netbsd", target_os = "dragonfly"))
))]
const DISK_FULL_CODES: &[i32] = &[28, 122]; // ENOSPC, EDQUOT (Linux)
#[cfg(windows)]
const DISK_FULL_CODES: &[i32] = &[112, 39]; // ERROR_DISK_FULL, ERROR_HANDLE_DISK_FULL
#[cfg(not(any(unix, windows)))]
const DISK_FULL_CODES: &[i32] = &[];

/// Whether an I/O error means the disk or the quota is full, from its kind or OS error number
/// (also when wrapped in another `io::Error`), never from its text.
pub fn is_disk_full(e: &std::io::Error) -> bool {
    use std::io::ErrorKind as K;
    matches!(e.kind(), K::StorageFull | K::QuotaExceeded)
        || e.raw_os_error().is_some_and(|c| DISK_FULL_CODES.contains(&c))
        || e.get_ref().and_then(|inner| inner.downcast_ref::<std::io::Error>()).is_some_and(is_disk_full)
}

/// Whether `host` is this computer (`localhost`, `127.x.x.x`, `::1`).
pub fn is_loopback_host(host: &str) -> bool {
    let h = host.trim().trim_start_matches('[').trim_end_matches(']').trim_end_matches('.').to_ascii_lowercase();
    h == "localhost" || h.ends_with(".localhost") || h == "::1" || h == "0:0:0:0:0:0:0:1" || h.starts_with("127.") || h == "::ffff:127.0.0.1"
}

/// Whether an error (or one of its causes) is a refused, reset or aborted connection, from the
/// `io::ErrorKind` in its source chain.
pub fn is_connection_lost(e: &(dyn std::error::Error + 'static)) -> bool {
    use std::io::ErrorKind as K;
    let mut cur: Option<&(dyn std::error::Error + 'static)> = Some(e);
    let mut depth = 0;
    while let Some(err) = cur {
        if let Some(io) = err.downcast_ref::<std::io::Error>()
            && matches!(io.kind(), K::ConnectionRefused | K::ConnectionReset | K::ConnectionAborted)
        {
            return true;
        }
        depth += 1;
        if depth > 16 {
            break;
        }
        cur = err.source();
    }
    false
}

/// The programs people usually run on these ports.
fn local_service_name(port: Option<u16>) -> Option<&'static str> {
    match port? {
        11434 => Some("Ollama"),
        1234 => Some("LM Studio"),
        _ => None,
    }
}

impl Error {
    /// The disk is full: [`DISK_FULL`], with the original error text as details.
    pub fn disk_full(detail: impl std::fmt::Display) -> Self {
        Self::storage(format!("{DISK_FULL} (Details: {detail})"))
    }

    /// A file error about `what`: a full disk gets [`DISK_FULL`], anything else `what: error`.
    pub fn io(what: impl std::fmt::Display, e: &std::io::Error) -> Self {
        if is_disk_full(e) { Self::disk_full(format_args!("{what}: {e}")) } else { Self::storage(format!("{what}: {e}")) }
    }

    /// A service on this computer (Ollama, LM Studio, the demo server) stopped answering: say
    /// where and what to do, in the style of the connector messages.
    pub fn local_service_gone(host: &str, port: Option<u16>) -> Self {
        let addr = match port {
            Some(p) if host.contains(':') && !host.starts_with('[') => format!("[{host}]:{p}"),
            Some(p) => format!("{host}:{p}"),
            None => host.to_string(),
        };
        Self::network(match local_service_name(port) {
            Some(name) => format!("Der lokale Dienst {name} unter {addr} antwortet nicht. Bitte {name} starten und noch einmal versuchen."),
            None => format!(
                "Der lokale Dienst unter {addr} antwortet nicht. Bitte das Programm starten, das dort läuft (zum Beispiel Ollama oder LM Studio), und noch einmal versuchen."
            ),
        })
    }
}

impl From<serde_json::Error> for Error {
    fn from(e: serde_json::Error) -> Self {
        Self::invalid(format!("JSON: {e}"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io;

    #[test]
    fn full_disk_from_kind_errno_and_wrapped_errors() {
        assert!(is_disk_full(&io::Error::from(io::ErrorKind::StorageFull)));
        assert!(is_disk_full(&io::Error::from(io::ErrorKind::QuotaExceeded)));
        for code in DISK_FULL_CODES {
            assert!(is_disk_full(&io::Error::from_raw_os_error(*code)), "{code}");
        }
        assert!(is_disk_full(&io::Error::other(io::Error::from(io::ErrorKind::StorageFull))));
        assert!(!is_disk_full(&io::Error::from(io::ErrorKind::PermissionDenied)));
        // The text alone does not count.
        assert!(!is_disk_full(&io::Error::other("No space left on device")));
    }

    #[test]
    fn io_errors_keep_their_details() {
        let e = Error::io("/tmp/x.docx", &io::Error::from(io::ErrorKind::StorageFull));
        assert_eq!(e.kind, ErrorKind::Storage);
        assert!(e.message.starts_with(DISK_FULL), "{}", e.message);
        assert!(e.message.contains("/tmp/x.docx"), "{}", e.message);
        let e = Error::io("/tmp/x.docx", &io::Error::from(io::ErrorKind::PermissionDenied));
        assert_eq!(e.kind, ErrorKind::Storage);
        assert!(!e.message.contains(DISK_FULL));
        assert!(e.message.starts_with("/tmp/x.docx: "));
    }

    #[test]
    fn lost_connections_from_the_source_chain() {
        #[derive(Debug)]
        struct Outer(io::Error);
        impl std::fmt::Display for Outer {
            fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                f.write_str("outer")
            }
        }
        impl std::error::Error for Outer {
            fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
                Some(&self.0)
            }
        }
        assert!(is_connection_lost(&Outer(io::Error::from(io::ErrorKind::ConnectionRefused))));
        assert!(is_connection_lost(&io::Error::from(io::ErrorKind::ConnectionReset)));
        assert!(!is_connection_lost(&Outer(io::Error::from(io::ErrorKind::TimedOut))));
    }

    #[test]
    fn local_services_by_host_and_port() {
        for h in ["localhost", "127.0.0.1", "127.1.2.3", "::1", "[::1]", "LOCALHOST."] {
            assert!(is_loopback_host(h), "{h}");
        }
        for h in ["example.org", "10.0.0.1", "localhost.example.org", "128.0.0.1"] {
            assert!(!is_loopback_host(h), "{h}");
        }
        let e = Error::local_service_gone("127.0.0.1", Some(11434));
        assert_eq!(e.kind, ErrorKind::Network);
        assert!(e.message.contains("Ollama unter 127.0.0.1:11434 antwortet nicht"), "{}", e.message);
        assert!(Error::local_service_gone("localhost", Some(1234)).message.contains("LM Studio starten"));
        let e = Error::local_service_gone("::1", Some(9));
        assert!(e.message.starts_with("Der lokale Dienst unter [::1]:9 antwortet nicht."), "{}", e.message);
    }
}
