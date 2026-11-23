//! Live collaboration for AIS.ChatCraft's yrs artifacts (documents, whiteboards, later more).
//!
//! A plug-in: the engine works without it, and nothing here needs a server until someone shares.
//! The wire format is the y-sync protocol (as y-websocket speaks it) over a WebSocket, so a web
//! client with Yjs can join a room later. A room is one artifact; its id is the artifact id.
//!
//! - [`code`]: join codes (`K7M2Q-X9TDA`), typed or sent in place of a link.
//! - [`proto`]: message helpers and the limits both sides keep.
//! - [`api`]: the relay's HTTP side (create a room, resolve a code, end a room).
//! - [`Session`]: one artifact kept in sync with its room, reconnecting by state vector.

pub mod api;
pub mod code;
pub mod proto;
mod session;

pub use api::{Created, JoinInfo, NewRoom, Server};
pub use session::{Options, Session, Status};

/// What can go wrong, with a German message a person can act on.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum CollabError {
    #[error("{0}")]
    Invalid(String),
    #[error("{0}")]
    Network(String),
    /// The server said no (wrong token, room ended, too many attempts).
    #[error("{0}")]
    Refused(String),
    #[error("{0}")]
    NotFound(String),
    #[error("{0}")]
    Protocol(String),
}

pub type Result<T> = std::result::Result<T, CollabError>;
