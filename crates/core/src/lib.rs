//! AIS.ChatCraft data model (layer 0). Pure data: no I/O, no async runtime, compiles for wasm.

pub mod attribution;
mod error;
pub mod h5p;
mod memory;
mod model;
pub mod worksheet;

pub use attribution::attribution_headers;
pub use error::{DISK_FULL, Error, ErrorKind, Result, is_connection_lost, is_disk_full, is_loopback_host};
pub use memory::{MemoryArea, MemoryNote, extract_explicit_memory, memory_prompt};
pub use model::*;

/// Milliseconds since the Unix epoch, as stored everywhere in AIS.ChatCraft.
pub type Millis = i64;
