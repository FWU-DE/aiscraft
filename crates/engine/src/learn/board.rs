//! Whiteboards as an output format (#56): the model answers with one ```` ```aiscraft-board ````
//! block (content only, see [`aiscraft_board::spec`]), AIS.ChatCraft lays it out. Backends with
//! tools get `board_create` from the same description.

use aiscraft_board::Board;
use aiscraft_board::spec::{self, BoardSpec};
use aiscraft_core::{Artifact, ArtifactKind, Error, Result, ToolSpec};
use serde_json::{Value, json};

use crate::commands::{CommandSpec, opt_str, req_str};
use crate::{Ctx, NoticeLevel, new_id, now_ms};

/// First words of a whiteboard request; the chat folds what follows the topic away.
pub const BOARD_INTRO: &str = "Erstelle ein Whiteboard zum Thema: ";

/// The ```` ```aiscraft-board ```` blocks with JSON in an answer. Blocks with an indented list
/// (the older way to add points to an open board) are change blocks, not this format.
pub fn board_blocks(answer: &str) -> Vec<&str> {
    super::fenced_blocks(answer)
        .into_iter()
        .filter(|(l, b)| (l == spec::FENCE || l == "aiscraft-whiteboard") && b.trim_start().starts_with('{'))
        .map(|(_, b)| b)
        .collect()
}

pub(crate) fn description() -> String {
    spec::prompt_schema().to_string()
}

pub(crate) fn check(src: &str) -> Result<Vec<String>, String> {
    spec::parse_lenient(src).map(|(_, p)| p)
}

/// The request for a whiteboard; `layout`: the arrangement asked for, if any.
pub fn board_prompt(topic: &str, layout: Option<spec::SpecLayout>, tools: bool) -> String {
    let deliver = if tools {
        "Lege das Whiteboard mit dem Werkzeug board_create an und gib das JSON nicht zusätzlich im Chat aus."
    } else {
        "Antworte mit genau einem ```aiscraft-board-Codeblock (gültiges JSON) und danach höchstens zwei Sätzen."
    };
    let layout = match layout {
        Some(l) => format!("Anordnung: {}.", l.as_str()),
        None => "Wähle die passende Anordnung (mindmap für Begriffe, cluster für Sammlungen, flow für Abläufe, timeline für Zeitliches).".into(),
    };
    format!("{BOARD_INTRO}{topic}\n\nKurze Karten, klar gegliedert, Deutsch. {layout}\n\n{}\n\n{deliver}", description())
}

fn repair_system() -> String {
    format!(
        "Du reparierst ein Whiteboard im JSON-Format aiscraft-board. Behalte den Inhalt bei, behebe nur die genannten Probleme. Antworte nur mit dem korrigierten JSON in einem ```aiscraft-board-Codeblock.\n\n{}",
        description()
    )
}

/// Parse, and when that fails or parts were dropped, ask the model once to repair it; a second
/// failure keeps what the first parse found (or the error).
async fn parse_or_repair(c: &Ctx, src: &str, repair: bool) -> Result<(BoardSpec, Vec<String>)> {
    let first = spec::parse_lenient(src);
    let problems = match &first {
        Err(e) => vec![e.clone()],
        Ok((_, p)) => p.clone(),
    };
    if !repair || problems.is_empty() {
        return first.map_err(Error::invalid);
    }
    let capped: String = src.chars().take(spec::MAX_SPEC_BYTES).collect();
    let prompt = format!("Der Block war ungültig:\n- {}\n\nBitte korrigieren.\n\n<whiteboard>\n{capped}\n</whiteboard>", problems.join("\n- "));
    match crate::chat::complete(c, Some(&repair_system()), &prompt).await {
        Ok(out) => {
            let block = board_blocks(&out).into_iter().next().map(str::to_string).unwrap_or(out);
            match (spec::parse_lenient(&block), first) {
                (Ok(fixed), Ok(orig)) => Ok(if fixed.1.len() <= orig.1.len() { fixed } else { orig }),
                (Ok(fixed), Err(_)) => Ok(fixed),
                (Err(_), Ok(orig)) => Ok(orig),
                (Err(_), Err(e)) => Err(Error::invalid(format!("Das Whiteboard ist ungültig: {e}"))),
            }
        }
        Err(_) => first.map_err(Error::invalid),
    }
}

/// A new board artifact from a spec, opened next to the chat.
pub(crate) fn create_board(c: &Ctx, s: &BoardSpec, title: Option<String>) -> Result<String> {
    let mut s = s.clone();
    if let Some(t) = title.filter(|t| !t.trim().is_empty()) {
        s.title = t.chars().take(120).collect();
    }
    let board = Board::from_spec(&s);
    let bytes = board.encode_state();
    let now = now_ms();
    let state = c.state();
    let a = Artifact {
        id: new_id(),
        kind: ArtifactKind::Board,
        title: if s.title.trim().is_empty() { "Whiteboard".into() } else { s.title.chars().take(80).collect() },
        conversation_id: state.current.clone(),
        project_id: state.selection.project_id.clone(),
        created_at: now,
        updated_at: now,
    };
    c.store(|st| {
        st.save_artifact(&a, Some(&bytes))?;
        st.add_artifact_version(&a.id, "Erstellt", &bytes, now)?;
        Ok(())
    })?;
    let id = a.id.clone();
    c.update(|st| {
        st.artifacts.insert(0, a);
        st.open_artifact = Some(id.clone());
        st.artifact_rev = st.artifact_rev.wrapping_add(1);
    });
    Ok(id)
}

pub(crate) static COMMANDS: &[CommandSpec] = &[
    CommandSpec {
        id: "board.request",
        label: "Whiteboard erstellen lassen",
        group: "Whiteboard",
        shortcut: None,
        params: "topic: string, subject?: string, grade?: string, layout?: mindmap|cluster|flow|timeline",
        palette: false,
        run: |c, p| Box::pin(request(c, p)),
    },
    CommandSpec {
        id: "board.create",
        label: "Als Whiteboard öffnen",
        group: "Whiteboard",
        shortcut: None,
        params: "content?: string (aiscraft-board-JSON oder eine Antwort mit dem Block), board?: object, from_message?: string, title?: string, repair?: bool (einmal vom Modell reparieren lassen)",
        palette: false,
        run: |c, p| Box::pin(create_cmd(c, p)),
    },
];

pub(crate) async fn create_cmd(c: Ctx, p: Value) -> Result<Value> {
    let repair = p.get("repair").and_then(Value::as_bool).unwrap_or(false);
    let (s, problems) = if let Some(obj) = p.get("board").filter(|v| v.is_object()) {
        spec::from_value(obj).map_err(Error::invalid)?
    } else {
        let src = match opt_str(&p, "from_message") {
            Some(mid) => super::message_text(&c, &mid)?,
            None => req_str(&p, "content")?,
        };
        let block = board_blocks(&src).into_iter().next().map(str::to_string).unwrap_or(src);
        parse_or_repair(&c, &block, repair).await?
    };
    let id = create_board(&c, &s, opt_str(&p, "title"))?;
    if !problems.is_empty() {
        c.notice(NoticeLevel::Warning, format!("Whiteboard geöffnet, bitte prüfen: {}", problems.iter().take(3).cloned().collect::<Vec<_>>().join("; ")));
    }
    Ok(json!({"id": id, "nodes": s.nodes.len(), "layout": s.layout.as_str(), "problems": problems}))
}

async fn request(c: Ctx, p: Value) -> Result<Value> {
    let topic = super::topic_line(&p)?;
    let layout = match opt_str(&p, "layout").filter(|l| !l.trim().is_empty()) {
        Some(l) => Some(spec::SpecLayout::parse(&l).ok_or_else(|| Error::invalid("layout: mindmap, cluster, flow oder timeline"))?),
        None => None,
    };
    let tools = super::tools_on(&c);
    // Boards there before the request (a timestamp alone can't tell within one millisecond).
    let before: std::collections::HashSet<String> = c.state().artifacts.iter().map(|a| a.id.clone()).collect();
    crate::chat::send_with(c.clone(), board_prompt(&topic, layout, tools), vec![], vec![], None, false).await?;
    if let Some(id) = c.state().artifacts.iter().find(|a| a.kind == ArtifactKind::Board && !before.contains(&a.id)).map(|a| a.id.clone()) {
        return Ok(json!({"id": id, "created": true}));
    }
    let answer = super::last_answer(&c);
    let Some(block) = board_blocks(&answer).into_iter().next().map(str::to_string) else {
        c.notice(NoticeLevel::Warning, "Die Antwort enthält kein Whiteboard. Bitte das Modell, es als ```aiscraft-board-Block auszugeben.");
        return Ok(json!({"created": false}));
    };
    match parse_or_repair(&c, &block, true).await {
        Ok((s, problems)) => {
            let id = create_board(&c, &s, None)?;
            Ok(json!({"id": id, "created": true, "nodes": s.nodes.len(), "problems": problems}))
        }
        // After the one repair round: the answer stays as text, with a note.
        Err(e) => {
            c.notice(NoticeLevel::Warning, format!("{e}. Die Antwort bleibt als Text im Chat."));
            Ok(json!({"created": false, "error": e.to_string()}))
        }
    }
}

pub(crate) fn tool_specs() -> Vec<ToolSpec> {
    vec![ToolSpec {
        name: "board_create".into(),
        description: format!(
            "Legt ein Whiteboard neben dem Chat an (Mindmap, Cluster, Ablauf oder Zeitleiste); die Anordnung übernimmt AIS.ChatCraft.\n{}",
            description()
        ),
        input_schema: json!({"type": "object", "properties": {
            "board": {"type": "object", "description": "das Whiteboard im Format aiscraft-board (title, layout, nodes, links, groups)"}
        }, "required": ["board"], "additionalProperties": false}),
    }]
}

pub(crate) fn run_tool(c: &Ctx, name: &str, a: &Value) -> Option<Result<String>> {
    if name != "board_create" {
        return None;
    }
    let v = a.get("board").cloned().unwrap_or_else(|| a.clone());
    let parsed = match &v {
        Value::String(src) => spec::parse_lenient(src),
        _ => spec::from_value(&v),
    };
    Some(parsed.map_err(Error::invalid).and_then(|(s, problems)| {
        let id = create_board(c, &s, None)?;
        let mut out = format!("Whiteboard „{}“ mit {} Karten angelegt und geöffnet (ID {id}).", s.title, s.nodes.len());
        if !problems.is_empty() {
            out.push_str(&format!(" Übersprungen: {}", problems.join("; ")));
        }
        Ok(out)
    }))
}
