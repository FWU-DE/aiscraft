use std::collections::BTreeMap;
use std::fmt;
use std::time::Duration;

use serde::{Deserialize, Serialize};

use crate::DEFAULT_TIMEOUT;

/// One connector as the app stores it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ServerConfig {
    /// Stable id; also the prefix of the tool names the models see.
    pub id: String,
    /// Name shown to people and used in error messages.
    pub name: String,
    pub transport: Transport,
    #[serde(default = "default_enabled")]
    pub enabled: bool,
    /// Per-request timeout in seconds; [`DEFAULT_TIMEOUT`] when unset.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub timeout_secs: Option<u64>,
}

fn default_enabled() -> bool {
    true
}

impl ServerConfig {
    /// The per-request timeout, kept between 1 s and 1 h.
    pub fn request_timeout(&self) -> Duration {
        self.timeout_secs.map(|s| Duration::from_secs(s.clamp(1, 3600))).unwrap_or(DEFAULT_TIMEOUT)
    }
}

/// How to reach the server.
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum Transport {
    /// Start a local program and speak newline-delimited JSON-RPC on its stdin/stdout.
    Stdio {
        command: String,
        #[serde(default)]
        args: Vec<String>,
        #[serde(default)]
        env: BTreeMap<String, String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        cwd: Option<String>,
    },
    /// MCP Streamable HTTP endpoint. `headers` are sent with every request (e.g. `Authorization`).
    Http {
        url: String,
        #[serde(default)]
        headers: BTreeMap<String, String>,
    },
}

/// Env values and header values often hold tokens; keep them out of logs.
impl fmt::Debug for Transport {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Transport::Stdio { command, args, env, cwd } => {
                f.debug_struct("Stdio").field("command", command).field("args", args).field("env", &env.keys().collect::<Vec<_>>()).field("cwd", cwd).finish()
            }
            Transport::Http { url, headers } => f.debug_struct("Http").field("url", url).field("headers", &headers.keys().collect::<Vec<_>>()).finish(),
        }
    }
}
