//! The AIS.ChatCraft desktop shell (layer 3), on egui. Thin: it renders the engine's state and
//! dispatches the engine's commands. Pure view state (open dialogs, text being typed, scroll)
//! lives here; everything else lives in the engine.

mod action_log;
mod answer_tools;
mod app;
mod approval;
mod artifact_panel;
mod backdrop;
mod citations;
mod conversation;
mod dialogs;
mod downloads;
mod explore;
mod h5p_panel;
mod home;
pub mod icons;
mod kit;
mod know;
mod learn_panel;
mod looks;
pub mod manual;
pub mod markup;
pub mod math;
pub mod mermaid;
mod palette;
pub mod plot;
mod privacy_panel;
mod rich;
mod share;
mod sidebar;
pub mod theme;
mod trust_rules;
mod update;
mod widgets;
pub mod worksheet;

pub use app::{AisCraftApp, PanelLayout, UiRequest};
