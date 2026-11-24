//! AIS.ChatCraft data model (layer 0). Pure data: no I/O, no async runtime, compiles for wasm.

mod error;
pub mod h5p;
mod memory;
mod model;
pub mod worksheet;

pub use error::{Error, ErrorKind, Result};
pub use memory::{MemoryArea, MemoryNote, extract_explicit_memory, memory_prompt};
pub use model::*;

/// Milliseconds since the Unix epoch, as stored everywhere in AIS.ChatCraft.
pub type Millis = i64;
