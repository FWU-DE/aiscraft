//! Reading what a Next.js page ships: chunk URLs, server action ids, the embedded RSC payload.

use std::collections::{BTreeMap, BTreeSet};
use std::sync::OnceLock;

use regex::Regex;
use serde_json::Value;

use crate::flight::{Row, RowParser};

fn re(cell: &'static OnceLock<Option<Regex>>, pattern: &str) -> Option<&'static Regex> {
    cell.get_or_init(|| Regex::new(pattern).ok()).as_ref()
}

/// JS chunk paths (`/_next/static/chunks/…js`) referenced by a page, in the HTML or the RSC
/// payload (which lists them without the leading `/_next/`).
pub fn chunk_paths(html: &str) -> Vec<String> {
    static RE: OnceLock<Option<Regex>> = OnceLock::new();
    let Some(r) = re(&RE, r#"(?:/_next/)?static/chunks/[A-Za-z0-9_\-./\[\]%~]+?\.js"#) else { return Vec::new() };
    let mut out = BTreeSet::new();
    for m in r.find_iter(html) {
        let p = m.as_str();
        let p = p.strip_prefix("/_next/").unwrap_or(p);
        if !p.contains("..") {
            out.insert(format!("/_next/{p}"));
        }
    }
    out.into_iter().collect()
}

/// Server action ids by exported name, from a client JS chunk. Next.js emits
/// `createServerReference)("<hex id>", callServer, void 0, findSourceMapURL, "<name>")`.
pub fn action_ids(js: &str) -> BTreeMap<String, String> {
    static RE: OnceLock<Option<Regex>> = OnceLock::new();
    let Some(r) = re(&RE, r#"createServerReference\)?\(\s*"([0-9a-f]{40,48})"\s*,[^"()]{0,200}?"([A-Za-z_$][A-Za-z0-9_$]*)"\s*\)"#) else {
        return BTreeMap::new();
    };
    r.captures_iter(js).filter_map(|c| Some((c.get(2)?.as_str().to_string(), c.get(1)?.as_str().to_string()))).collect()
}

/// The RSC payload a server-rendered page carries in `self.__next_f.push([1, "<string>"])`
/// calls, concatenated.
pub fn rsc_payload(html: &str) -> String {
    static RE: OnceLock<Option<Regex>> = OnceLock::new();
    let Some(r) = re(&RE, r#"self\.__next_f\.push\(\[1,("(?:[^"\\]|\\.)*")\]\)"#) else { return String::new() };
    let mut out = String::new();
    for c in r.captures_iter(html) {
        if let Some(lit) = c.get(1)
            && let Ok(s) = serde_json::from_str::<String>(lit.as_str())
        {
            out.push_str(&s);
        }
    }
    out
}

/// Every JSON value in an RSC payload (rows that are not JSON are skipped).
pub fn payload_values(payload: &str) -> Vec<Value> {
    payload_rows(payload).into_iter().map(|(_, v)| v).collect()
}

/// Every JSON row of an RSC payload with its row id, in order.
fn payload_rows(payload: &str) -> Vec<(u64, Value)> {
    // Parse line by line so that one odd row does not hide the rest.
    let mut out = Vec::new();
    let mut parser = RowParser::new();
    for line in payload.split_inclusive('\n') {
        match parser.feed(line.as_bytes()) {
            Ok(rows) => out.extend(rows.into_iter().filter_map(|r| match r {
                Row::Json { id, value } => Some((id, value)),
                _ => None,
            })),
            Err(_) => parser = RowParser::new(),
        }
    }
    out
}

/// Follow a React Flight back-reference such as `"$1f:0:props:models:0:priceMetadata"` (row
/// `1f`, then the path) to the value it points at. React writes these for objects it already
/// sent, e.g. the second occurrence of a model's `priceMetadata`. Anything else passes through.
fn resolve_ref<'a>(v: &'a Value, rows: &BTreeMap<u64, &'a Value>) -> &'a Value {
    let Some(r) = v.as_str().and_then(|s| s.strip_prefix('$')) else { return v };
    let mut parts = r.split(':');
    let Some(row) = parts.next().and_then(|id| u64::from_str_radix(id, 16).ok()).and_then(|id| rows.get(&id).copied()) else { return v };
    let mut cur = row;
    for key in parts.take(32) {
        let next = match cur {
            // A React element `["$", type, key, props]` is addressed by field name.
            Value::Array(a) if a.first().and_then(Value::as_str) == Some("$") && !key.bytes().all(|b| b.is_ascii_digit()) => match key {
                "type" => a.get(1),
                "key" => a.get(2),
                "props" => a.get(3),
                _ => None,
            },
            Value::Array(a) => key.parse::<usize>().ok().and_then(|i| a.get(i)),
            Value::Object(o) => o.get(key),
            _ => None,
        };
        match next {
            Some(n) => cur = n,
            None => return v,
        }
    }
    cur
}

/// A model as AIS.chat's `llm_model` table describes it.
#[derive(Debug, Clone, PartialEq)]
pub struct PageModel {
    pub id: String,
    pub name: String,
    pub display_name: String,
    pub description: String,
    pub kind: String,
    pub vision: bool,
}

/// Find AIS.chat model rows (objects with `id`, `name`, `displayName` and `priceMetadata`)
/// anywhere in the page payload. Deduplicated by id, in first-seen order.
///
/// A model often appears twice (the image page sends `models` and `defaultImageModel`); the second
/// copy carries back-references (`"$1f:0:props:models:0:priceMetadata"`) instead of objects. Those
/// are resolved, so the kind (`text`, `image`, `embedding`) does not depend on which copy is seen
/// first.
pub fn page_models(payload: &str) -> Vec<PageModel> {
    let list = payload_rows(payload);
    let rows: BTreeMap<u64, &Value> = list.iter().map(|(id, v)| (*id, v)).collect();
    let mut out: Vec<PageModel> = Vec::new();
    for (_, v) in &list {
        walk(v, 0, &mut |o| {
            let s = |k: &str| resolve_ref(o.get(k).unwrap_or(&Value::Null), &rows).as_str().map(str::to_string);
            let (Some(id), Some(name), Some(display_name)) = (s("id"), s("name"), s("displayName")) else { return };
            let Some(price) = o.get("priceMetadata").map(|p| resolve_ref(p, &rows)) else { return };
            if o.get("isDeleted").and_then(Value::as_bool) == Some(true) || out.iter().any(|m| m.id == id) {
                return;
            }
            let kind = price.get("type").and_then(Value::as_str).unwrap_or("text").to_string();
            let formats = o.get("supportedImageFormats").map(|f| resolve_ref(f, &rows));
            let vision = formats.and_then(Value::as_array).is_some_and(|a| !a.is_empty());
            out.push(PageModel { id, name, display_name, description: s("description").unwrap_or_default(), kind, vision });
        });
    }
    out
}

/// The token-point budget the page shows (`usedBudget`, `maxBudget`), if any.
pub fn page_budget(payload: &str) -> Option<(f64, f64)> {
    let mut found = None;
    for v in payload_values(payload) {
        walk(&v, 0, &mut |o| {
            if found.is_none()
                && let (Some(u), Some(m)) = (o.get("usedBudget").and_then(Value::as_f64), o.get("maxBudget").and_then(Value::as_f64))
                && m > 0.0
                && u.is_finite()
                && m.is_finite()
            {
                found = Some((u, m));
            }
        });
    }
    found
}

/// The name of the model the person last used, if the page says so (`initialModelName`).
pub fn initial_model_name(payload: &str) -> Option<String> {
    let mut found = None;
    for v in payload_values(payload) {
        walk(&v, 0, &mut |o| {
            if found.is_none()
                && let Some(n) = o.get("initialModelName").and_then(Value::as_str)
            {
                found = Some(n.to_string());
            }
        });
    }
    found
}

/// Chat messages in a conversation page's payload: objects with an `id`, a `role` of `user` or
/// `assistant` and string `content`. Deduplicated by id, in first-seen order.
pub fn page_messages(payload: &str) -> Vec<(String, String, String)> {
    let mut out: Vec<(String, String, String)> = Vec::new();
    for v in payload_values(payload) {
        walk(&v, 0, &mut |o| {
            let s = |k: &str| o.get(k).and_then(Value::as_str);
            let (Some(id), Some(role), Some(content)) = (s("id"), s("role"), s("content")) else { return };
            if (role == "user" || role == "assistant") && !out.iter().any(|m| m.0 == id) {
                let content = content.strip_prefix("$$").map(|r| format!("${r}")).unwrap_or_else(|| content.to_string());
                out.push((id.to_string(), role.to_string(), content));
            }
        });
    }
    out
}

fn walk(v: &Value, depth: usize, f: &mut dyn FnMut(&serde_json::Map<String, Value>)) {
    if depth > 64 {
        return;
    }
    match v {
        Value::Object(o) => {
            f(o);
            for x in o.values() {
                walk(x, depth + 1, f);
            }
        }
        Value::Array(a) => {
            for x in a {
                walk(x, depth + 1, f);
            }
        }
        _ => {}
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn finds_action_ids_in_minified_js() {
        // Shape observed in app.ais-chat.schule chunks on 2026-11-23.
        let js = r#"let a=(0,f.createServerReference)("401cb40b76f1bad93befce223b14e9683a270cd045",f.callServer,void 0,f.findSourceMapURL,"getChatInfoByInviteCodeAction");var b=(0,g.createServerReference)("60aa11bb22cc33dd44ee55ff66778899aabbccddee",g.callServer,void 0,g.findSourceMapURL,"sendChatMessageAction")"#;
        let ids = action_ids(js);
        assert_eq!(ids.get("sendChatMessageAction").map(String::as_str), Some("60aa11bb22cc33dd44ee55ff66778899aabbccddee"));
        assert_eq!(ids.len(), 2);
    }

    #[test]
    fn chunk_paths_from_html_and_payload() {
        let html = r#"<script src="/_next/static/chunks/0ocyeb5hp4dlt.js" async></script><script>self.__next_f.push([1,"1:I[\"static/chunks/abc-123.js\"]\n"])</script>"#;
        let p = chunk_paths(html);
        assert!(p.contains(&"/_next/static/chunks/0ocyeb5hp4dlt.js".to_string()));
        assert!(p.contains(&"/_next/static/chunks/abc-123.js".to_string()));
    }

    #[test]
    fn messages_from_conversation_page() {
        let row = r#"7:["$","$L8",null,{"initialMessages":[{"id":"a","role":"user","content":"Hallo"},{"id":"b","role":"assistant","content":"$$9 Euro"},{"id":"a","role":"user","content":"dup"},{"id":"s","role":"system","content":"x"}]}]"#;
        let msgs = page_messages(&format!("{row}\n"));
        assert_eq!(msgs, vec![("a".into(), "user".into(), "Hallo".into()), ("b".into(), "assistant".into(), "$9 Euro".into())]);
    }

    #[test]
    fn budget_from_page_payload() {
        let row = r#"9:["$","$L4",null,{"federalState":{},"usedBudget":370.5,"maxBudget":1000}]"#;
        assert_eq!(page_budget(&format!("{row}\n")), Some((370.5, 1000.0)));
        assert_eq!(page_budget("1:{\"maxBudget\":0,\"usedBudget\":1}\n"), None);
        assert_eq!(page_budget(""), None);
    }

    #[test]
    fn models_from_page_payload() {
        let row = r#"5:["$","$L6",null,{"initialModelName":"gpt-5","models":[{"id":"u1","provider":"azure","name":"gpt-5","displayName":"GPT-5","description":"schnell","priceMetadata":{"type":"text"},"supportedImageFormats":["png"],"isDeleted":false},{"id":"u2","provider":"x","name":"img","displayName":"Bild","priceMetadata":{"type":"image"}},{"id":"u3","provider":"x","name":"gone","displayName":"Weg","priceMetadata":{"type":"text"},"isDeleted":true}]}]"#;
        let lit = serde_json::to_string(&format!("{row}\n")).unwrap();
        let html = format!("<script>self.__next_f.push([1,{lit}])</script>");
        let payload = rsc_payload(&html);
        let models = page_models(&payload);
        assert_eq!(models.len(), 2);
        assert_eq!(models[0].display_name, "GPT-5");
        assert!(models[0].vision);
        assert_eq!(models[1].kind, "image");
        assert_eq!(initial_model_name(&payload).as_deref(), Some("gpt-5"));
    }

    #[test]
    fn image_page_models_with_back_references() {
        // Shape of app.ais-chat.schule/image-generation on 2026-11-24: `defaultImageModel` repeats
        // the first model with back-references, and keys sort before `models`.
        let rows = concat!(
            r#"1f:[["$","$L3e","k",{"models":[{"id":"m-img","provider":"bifrost","name":"gpt-image-1.5","displayName":"GPT-Image-1.5","description":"Bilder","priceMetadata":{"type":"image","inputTextTokenPrice":414.5},"supportedImageFormats":["png","jpeg"],"isDeleted":false},{"id":"m-txt","provider":"bifrost","name":"gpt-5","displayName":"GPT-5","priceMetadata":{"type":"text"},"isDeleted":false}],"defaultImageModel":{"id":"m-img","provider":"bifrost","name":"gpt-image-1.5","displayName":"GPT-Image-1.5","description":"Bilder","priceMetadata":"$1f:0:props:models:0:priceMetadata","supportedImageFormats":"$1f:0:props:models:0:supportedImageFormats","isDeleted":false}}]]"#,
            "\n",
            r#"20:{"other":{"id":"m-img","name":"gpt-image-1.5","displayName":"GPT-Image-1.5","priceMetadata":"$1f:0:props:models:0:priceMetadata"},"broken":{"id":"x","name":"x","displayName":"X","priceMetadata":"$99:0:nope"}}"#,
            "\n"
        );
        let models = page_models(rows);
        let img: Vec<_> = models.iter().filter(|m| m.kind == "image").collect();
        assert_eq!(img.len(), 1, "{models:?}");
        assert_eq!(img[0].id, "m-img");
        assert!(img[0].vision);
        assert!(models.iter().any(|m| m.id == "m-txt" && m.kind == "text"));
        // An unresolvable reference falls back to text instead of failing.
        assert!(models.iter().any(|m| m.id == "x" && m.kind == "text"));
    }
}
