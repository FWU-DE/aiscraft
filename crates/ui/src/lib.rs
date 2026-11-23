//! The AISCraft desktop shell (layer 3), on egui. Thin: it renders the engine's state and
//! dispatches the engine's commands. Pure view state (open dialogs, text being typed, scroll)
//! lives here; everything else lives in the engine.

mod app;
mod conversation;
mod dialogs;
mod palette;
mod sidebar;
pub mod theme;
mod widgets;

pub use app::{AisCraftApp, UiRequest};
