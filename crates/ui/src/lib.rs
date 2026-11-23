//! The AIS.ChatCraft desktop shell (layer 3), on egui. Thin: it renders the engine's state and
//! dispatches the engine's commands. Pure view state (open dialogs, text being typed, scroll)
//! lives here; everything else lives in the engine.

mod app;
mod artifact_panel;
mod conversation;
mod dialogs;
mod explore;
pub mod manual;
pub mod markup;
pub mod math;
pub mod mermaid;
mod palette;
pub mod plot;
mod share;
mod sidebar;
pub mod theme;
mod widgets;

pub use app::{AisCraftApp, UiRequest};
