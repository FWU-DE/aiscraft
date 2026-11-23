//! AIS.ChatCraft whiteboard.
//!
//! - [`Board`]: the document, a `yrs` (Yjs) CRDT so it can be shared live, with typed helpers,
//!   undo/redo and update exchange.
//! - [`BoardView`]: an egui canvas widget (pan, zoom, tools, selection, in-place text editing).
//! - [`Board::from_outline`] / [`Board::summary_text`]: pure hooks for the AI side
//!   (chat answer → mind map, board → text context).

mod model;
mod outline;
mod view;

pub use model::{
    Board, BoardError, BoardJson, COORD_LIMIT, Element, Kind, MAX_ELEMENTS, MAX_ID, MAX_POINTS, MAX_TEXT, MAX_UPDATE_BYTES, Result, SIZE_LIMIT, arrow_endpoints,
};
pub use outline::{OutlineNode, parse_outline};
pub use view::{BoardStyle, BoardView, Camera, PALETTE, Tool, ViewState, color_label};

#[cfg(test)]
mod tests;
