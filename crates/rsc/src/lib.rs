//! The Next.js wire protocol, as far as a native client needs it to talk to AIS.chat:
//!
//! - [`flight`]: an incremental React Flight row parser and a decoder for server action
//!   responses whose result carries a `ReadableStream<string>`.
//! - [`encode_action_args`]: the request body for a server action with plain JSON arguments.
//! - [`scrape`]: server action ids from the shipped JS chunks, chunk URLs and the RSC payload
//!   embedded in a page, and the model list in that payload.
//!
//! Everything here is pure and works on bytes and strings; I/O lives in `aiscraft-backends`.

pub mod flight;
pub mod scrape;

use serde_json::Value;

/// Encode plain JSON arguments the way React's `encodeReply` does for a server action: a JSON
/// array, with every string that starts with `$` escaped as `$$`.
pub fn encode_action_args(args: &[Value]) -> String {
    fn esc(v: &Value) -> Value {
        match v {
            Value::String(s) if s.starts_with('$') => Value::String(format!("${s}")),
            Value::Array(a) => Value::Array(a.iter().map(esc).collect()),
            Value::Object(o) => Value::Object(o.iter().map(|(k, v)| (k.clone(), esc(v))).collect()),
            other => other.clone(),
        }
    }
    Value::Array(args.iter().map(esc).collect()).to_string()
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn escapes_dollar_strings_deeply() {
        let body = encode_action_args(&[json!({"a": "$5 kostet", "b": ["x", "$y"], "n": 1})]);
        assert_eq!(body, r#"[{"a":"$$5 kostet","b":["x","$$y"],"n":1}]"#);
    }
}
