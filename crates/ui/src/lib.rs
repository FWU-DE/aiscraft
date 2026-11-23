//! The AIS.ChatCraft desktop shell (layer 3), on egui. Thin: it renders the engine's state and
//! dispatches the engine's commands. Pure view state (open dialogs, text being typed, scroll)
//! lives here; everything else lives in the engine.

mod app;
mod artifact_panel;
mod conversation;
pub mod manual;
mod dialogs;
mod palette;
pub mod plot;
mod sidebar;
pub mod theme;
mod widgets;

pub use app::{AisCraftApp, UiRequest};
