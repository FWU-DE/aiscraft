//! Longer tasks in the background (as in Nod): a task gets its own chat and runs while the
//! person keeps talking; the result comes back to the chat it was started from.

use aiscraft_core::{Conversation, Error, Result, Role};
use serde_json::{Value, json};

use crate::commands::{opt_str, req_str};
use crate::{Ctx, NoticeAction, NoticeLevel, new_id, now_ms};

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
    c.spawn(async move {
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
