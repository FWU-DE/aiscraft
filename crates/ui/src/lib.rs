//! The AIS.ChatCraft desktop shell (layer 3), on egui. Thin: it renders the engine's state and
//! dispatches the engine's commands. Pure view state (open dialogs, text being typed, scroll)
//! lives here; everything else lives in the engine.

mod action_log;
mod answer_tools;
mod app;
mod approval;
mod artifact_panel;
mod backdrop;
mod chat_import;
mod citations;
mod conversation;
mod dialogs;
mod downloads;
mod explore;
mod extensions;
mod h5p_panel;
mod home;
pub mod icons;
mod kit;
mod know;
mod learn_panel;
mod local_files;
mod looks;
pub mod manual;
pub mod markup;
pub mod math;
pub mod mermaid;
mod nod_settings;
mod palette;
pub mod plot;
mod privacy_panel;
mod rich;
mod share;
mod sidebar;
mod status_line;
pub mod theme;
mod trust_rules;
mod update;
mod views;
mod widgets;
pub mod worksheet;

pub use app::{AisCraftApp, PanelLayout, UiRequest};
