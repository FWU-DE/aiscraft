//! Longer tasks in the background (as in Nod): a task gets its own chat and runs while the
//! person keeps talking; the result comes back to the chat it was started from as a compact card
//! (#69). While tasks, schedules and page watches run, a status line above the composer names
//! them instead of filling the chat with progress.

use aiscraft_core::{Conversation, Error, Message, Result, Role};
use serde::Serialize;
use serde_json::{Value, json};

use crate::commands::{opt_str, req_str};
use crate::{Ctx, JobKind, NoticeAction, NoticeLevel, RunningJob, State, new_id, now_ms};

/// Label of the step that marks a task's result in the chat it came from; its detail is the
/// task's chat.
pub const TASK_RESULT: &str = "Aufgabe";

/// Shows a job in the status line while it lives.
pub(crate) struct Running {
    c: Ctx,
    id: String,
}

impl Running {
    pub(crate) fn start(c: &Ctx, id: &str, kind: JobKind, title: &str) -> Running {
        let job = RunningJob { id: id.to_string(), kind, title: title.chars().take(100).collect(), started_at: now_ms() };
        c.update(|s| s.running.push(job));
        Running { c: c.clone(), id: id.to_string() }
    }
}

impl Drop for Running {
    fn drop(&mut self) {
        let id = self.id.clone();
        self.c.update(|s| {
            if let Some(i) = s.running.iter().position(|j| j.id == id) {
                s.running.remove(i);
            }
        });
    }
}

/// The status line: „2 Aufgaben laufen · Elternbrief prüfen …“; `None` when nothing runs.
pub fn running_summary(jobs: &[RunningJob]) -> Option<String> {
    let first = jobs.first()?;
    let count = |k: JobKind| jobs.iter().filter(|j| j.kind == k).count();
    let mut parts = Vec::new();
    for (k, one, many) in [
        (JobKind::Task, "1 Aufgabe läuft", "Aufgaben laufen"),
        (JobKind::Schedule, "1 Zeitplan läuft", "Zeitpläne laufen"),
        (JobKind::Watch, "1 Beobachtung läuft", "Beobachtungen laufen"),
    ] {
        match count(k) {
            0 => {}
            1 => parts.push(one.to_string()),
            n => parts.push(format!("{n} {many}")),
        }
    }
    Some(format!("{} · {} …", parts.join(" · "), first.title))
}

/// A task's result as a compact card (#69).
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct TaskCard {
    /// The task's chat, which „Details“ opens.
    pub task_id: String,
    pub title: String,
    /// One line of the result.
    pub line: String,
    pub ok: bool,
}

/// The card for a message that is a task's result.
pub fn task_card(state: &State, m: &Message) -> Option<TaskCard> {
    let step = m.activity.iter().find(|s| s.label == TASK_RESULT)?;
    let task_id = step.detail.clone()?;
    let title = state.conversation(&task_id).map(|c| c.title.trim_start_matches("▶").trim().to_string()).unwrap_or_else(|| "Aufgabe".into());
    // The first line of the result that is not a heading.
    let heading = |l: &str| l.starts_with('#') || (l.starts_with("**") && l.ends_with("**"));
    let body = m
        .content
        .lines()
        .skip(1)
        .map(str::trim)
        .filter(|l| !l.is_empty() && !heading(l))
        .map(|l| l.trim_start_matches(['*', '-', '>', ' ']).trim())
        .find(|l| !l.is_empty());
    let mut line: String = body.unwrap_or("").chars().take(140).collect();
    if body.is_some_and(|b| b.chars().count() > 140) {
        line.push_str(" …");
    }
    Some(TaskCard { task_id, title, line, ok: !step.failed })
}

/// Start a task. Returns at once; the task reports back when it is done.
pub(crate) async fn start(c: Ctx, p: Value) -> Result<Value> {
    let title = req_str(&p, "title")?.trim().chars().take(100).collect::<String>();
    let instructions = req_str(&p, "instructions")?;
    if title.is_empty() || instructions.trim().is_empty() {
        return Err(Error::invalid("Titel und Auftrag dürfen nicht leer sein"));
    }
    let origin = opt_str(&p, "conversation").or_else(|| c.state().current.clone());
    let base = origin.as_deref().and_then(|id| c.state().conversation(id).cloned());
    let sel = c.state().selection.clone();
    let now = now_ms();
    let mut sources = base.as_ref().map(|b| b.sources.clone()).unwrap_or_else(|| sel.sources.clone());
    // No task inside a task: the history search stays, starting tasks does not apply here.
    sources.history = sources.history && sources.tools;
    let conv = Conversation {
        id: new_id(),
        title: format!("▶ {title}"),
        backend_id: base.as_ref().map(|b| b.backend_id.clone()).unwrap_or(sel.backend_id),
        model_id: base.as_ref().map(|b| b.model_id.clone()).unwrap_or(sel.model_id),
        project_id: base.as_ref().and_then(|b| b.project_id.clone()),
        created_at: now,
        updated_at: now,
        pinned: false,
        sources,
        persona: None,
        kind: Default::default(),
        remote_id: None,
        temporary: false,
    };
    c.store(|st| st.save_conversation(&conv))?;
    let task_id = conv.id.clone();
    c.update(|s| {
        let pos = s.conversations.iter().position(|x| !x.pinned).unwrap_or(s.conversations.len());
        s.conversations.insert(pos, conv);
    });
    let ctx = c.clone();
    let tid = task_id.clone();
    // In the status line from now until the result is back.
    let running = Running::start(&c, &task_id, JobKind::Task, &title);
    c.spawn(async move {
        let _running = running;
        let prompt = format!("Aufgabe „{title}“:\n\n{instructions}\n\nArbeite die Aufgabe vollständig ab und schließe mit einem kurzen Ergebnis.");
        let out = crate::chat::send_background_boxed(ctx.clone(), tid.clone(), prompt).await;
        let (ok, text) = match out {
            Ok(v) => (true, v.get("text").and_then(Value::as_str).unwrap_or("").to_string()),
            Err(e) => (false, e.message),
        };
        let summary: String = text.chars().take(3000).collect();
        if let Some(origin) = origin {
            let mut m = aiscraft_core::Message {
                id: new_id(),
                conversation_id: origin.clone(),
                role: Role::Assistant,
                content: if ok {
                    format!("**Aufgabe „{title}“ ist fertig.**\n\n{summary}")
                } else {
                    format!("**Aufgabe „{title}“ ist gescheitert:** {summary}")
                },
                created_at: now_ms(),
                model: Some("Aufgabe".into()),
                attachments: vec![],
                activity: vec![],
                error: None,
                usage: None,
                tool_calls: vec![],
                tool_result: None,
                thinking: String::new(),
                citations: Vec::new(),
            };
            // Shown as a compact card with „Details“ (#69).
            m.activity.push(aiscraft_core::ActivityStep {
                label: TASK_RESULT.into(),
                detail: Some(tid.clone()),
                done: true,
                failed: !ok,
                ..Default::default()
            });
            if !ok {
                m.error = Some(summary.clone());
            }
            let _ = ctx.store(|st| st.save_message(&m));
            ctx.update(|s| {
                if s.current.as_deref() == Some(&origin) {
                    s.messages.push(m);
                }
            });
        }
        crate::inbox::notify(
            &ctx,
            if ok { NoticeLevel::Success } else { NoticeLevel::Error },
            "Aufgabe",
            &format!("„{title}“ {}", if ok { "ist fertig" } else { "ist gescheitert" }),
            Some(NoticeAction { label: "Öffnen".into(), command: "chat.open".into(), params: json!({"id": tid}) }),
        );
    });
    Ok(json!({"id": task_id}))
}
