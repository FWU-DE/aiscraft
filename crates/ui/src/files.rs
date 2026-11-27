//! Native file dialogs that never block the event loop (craft-dialogs).
//!
//! A blocking `rfd` dialog runs AppKit's modal loop inside winit's event handler on macOS; a file
//! dragged over the window meanwhile re-enters the handler and the app aborts. Here a button only
//! *asks* (`self.files.ask(dialog, Then::…)`); `files_frame` shows what is queued at the end of
//! the frame and, on a later frame, hands each answer to `Then` to finish the action.

use std::path::PathBuf;

use craft_dialogs::{Answer, Dialogs, Wake};
use serde_json::{Value, json};

use crate::app::AisCraftApp;
use crate::dialogs::Dialog;

/// What to do with the path(s) the person picked.
pub(crate) enum Then {
    /// Run `cmd` with `params` plus `"path"` (one file or folder).
    Run { cmd: &'static str, params: Value },
    /// Like `Run`, after saving the open artifact (an export must hold the latest edit).
    ExportArtifact { cmd: &'static str, params: Value },
    /// `chat.save_image`: the badge setting is read when the path arrives.
    SaveImage { message_id: String, index: usize },
    /// Attach the files to the composer.
    AttachFiles,
    /// `chat.import` into the current project.
    ImportChat,
    /// Add each file to a project's knowledge.
    ProjectFiles { project: String },
    /// Save a chat; the format follows the extension the person chose (`.json` or Markdown).
    ExportChat { id: String },
    /// One Markdown file per chat into the chosen folder.
    ExportChats { ids: Vec<String> },
    /// Read a class list (for the name protection). `first_setup`: the settings page asked while
    /// the setup was never gone through, so the setup follows the first list.
    ClassList { project: Option<String>, first_setup: bool },
    /// The service account key of Vertex AI into the open „Dienst hinzufügen“ form.
    KeyFileToForm,
    /// Sign in to a Vertex AI service with the chosen key.
    KeyFileLogin { backend_id: String },
    /// A path for a field of the open connector install dialog.
    ExtensionField { key: String, multiple: bool },
}

/// The file types the name protection reads as a class list.
pub(crate) const CLASS_LIST: &[&str] = &["csv", "tsv", "txt", "xlsx", "xls", "ods", "vcf"];

pub(crate) type Files = Dialogs<Then>;

/// The service, woken through `ctx` when an answer is ready. Tests answer every dialog with
/// Cancel: they never open a window.
pub(crate) fn service(ctx: &egui::Context) -> Files {
    let ctx = ctx.clone();
    #[allow(unused_mut)]
    let mut files = Dialogs::new(Wake::new(move || ctx.request_repaint()));
    #[cfg(test)]
    files.set_override(Some(Box::new(|_| Answer::Cancelled)));
    files
}

impl AisCraftApp {
    /// Once per frame, last: finish what was answered, then show what is queued over the window.
    pub(crate) fn files_frame(&mut self, frame: &eframe::Frame) {
        for (then, answer) in self.files.take_answers() {
            let paths = match answer {
                Answer::Paths(paths) => paths,
                Answer::Saved { path, .. } => vec![path],
                Answer::Unavailable(why) => {
                    log::warn!("Dateidialog nicht verfügbar: {why}");
                    self.engine.ctx().notice(aiscraft_engine::NoticeLevel::Error, format!("Der Dateidialog lässt sich nicht öffnen: {why}"));
                    continue;
                }
                Answer::Cancelled | Answer::Message(_) => continue,
            };
            self.finish_files(then, paths);
        }
        self.files.show_pending(Some(frame));
    }

    fn finish_files(&mut self, then: Then, paths: Vec<PathBuf>) {
        let Some(first) = paths.first() else { return };
        let path = first.display().to_string();
        match then {
            Then::Run { cmd, params } => self.run(cmd, with_path(params, path)),
            Then::ExportArtifact { cmd, params } => {
                self.save_if_dirty();
                self.run(cmd, with_path(params, path));
            }
            Then::SaveImage { message_id, index } => self.run("chat.save_image", self.image_save_params(&message_id, index, &path)),
            Then::AttachFiles => self.take_files(paths),
            Then::ImportChat => {
                let project = self.state.selection.project_id.clone();
                self.run("chat.import", json!({"path": path, "project": project}));
            }
            Then::ProjectFiles { project } => {
                for p in paths {
                    self.run("project.file_add", json!({"project": project, "path": p.display().to_string()}));
                }
            }
            Then::ExportChat { id } => {
                let format = if first.extension().is_some_and(|e| e == "json") { "json" } else { "markdown" };
                self.run("chat.export", json!({"id": id, "format": format, "path": path}));
            }
            Then::ExportChats { ids } => self.export_many_into(&ids, first),
            Then::ClassList { project, first_setup } => {
                self.run("privacy.list_import", json!({"path": path, "project": project}));
                if first_setup {
                    self.dialog = Some(self.privacy_setup(1, true));
                }
            }
            Then::KeyFileToForm => {
                if let Some(Dialog::AddBackend { secret, .. }) = &mut self.dialog {
                    *secret = path;
                }
            }
            Then::KeyFileLogin { backend_id } => {
                self.run("backend.login", json!({"id": backend_id, "key_file": path}));
                if let Some(Dialog::Login { secret, sent, .. }) = &mut self.dialog {
                    *secret = crate::dialogs::file_name(&path);
                    *sent = true;
                }
            }
            Then::ExtensionField { key, multiple } => {
                if let Some(Dialog::Install { values, .. }) = &mut self.dialog {
                    let v = values.entry(key).or_default();
                    if multiple && !v.trim().is_empty() {
                        v.push('\n');
                        v.push_str(&path);
                    } else {
                        *v = path;
                    }
                }
            }
        }
    }
}

fn with_path(mut params: Value, path: String) -> Value {
    if let Some(o) = params.as_object_mut() {
        o.insert("path".into(), Value::String(path));
    }
    params
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_path_joins_the_params() {
        assert_eq!(with_path(json!({"id": "a"}), "/x/y".into()), json!({"id": "a", "path": "/x/y"}));
        assert_eq!(with_path(json!({}), "p".into()), json!({"path": "p"}));
    }
}
