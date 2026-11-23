# AGENTS.md: guide for AI agents and contributors

**AIS.ChatCraft** is a native desktop client for [AIS.chat](https://app.ais-chat.schule) and other chat
backends (OpenAI-compatible servers, Claude via API, the Claude subscription through the local Claude
CLI). Rust only, egui/eframe, **no webview, no Electron, no Tauri**. It follows the conventions of the
storytold Craft apps (see `~/code/photocraft/AGENTS.md`): never crash, every action is a command,
tests are the gate. Product name in user-facing text: **AIS.ChatCraft** (AIS and AIS.chat are
different things; never shorten to "AIS"). Machine names stay `aiscraft-*`. User-facing text is
German.

Repo: `FWU-DE/aiscraft` (private). Backlog: GitHub issues. AutoCraft (storytold/autocraft) is set up
to triage and implement issues; see `deploy/autocraft/`.

## 1. Workspace map (layers: a crate may only depend on lower ones)

```text
crates/
  core       L0  data model, errors, memory rules (serde only, no I/O)
  rsc        L0  Next.js wire protocol: React Flight parser, server action ids, page payloads
  store      L1  SQLite (FTS5 search, artifacts, settings, usage); schema migrations in Store::init
  backends   L1  AIS.chat (session cookie + server action), OpenAI-compatible, Anthropic, Claude CLI,
                 browser login (DevTools), SSE helpers
  mcp        L1  MCP client (stdio, Streamable HTTP) for connectors
  board      L1  whiteboard: yrs CRDT model + egui widget + mind map layout
  voice      L1  recording, playback, transcription, text to speech (OpenAI, AIS.chat)
  engine     L2  state, command registry, chat/tool loop, artifacts, schedules, connectors, usage
  ui         L3  egui shell (thin: renders state, dispatches commands)
apps/
  aiscraft      desktop app (+ `--control` JSON control channel, token-protected)
  aiscraft-cli  CLI (chat, login, status, search, export, `call`, `mcp` server)
deploy/autocraft  Coolify compose for AutoCraft
```

## 2. Golden rules

1. **Never crash.** No `unwrap`/`expect`/`panic!`/indexing that can panic in non-test code (workspace
   lints deny them), no `unsafe`. Input from servers, files, models and users is hostile: cap sizes,
   use `get()`, return `Err` with a German message a person can act on.
2. **Everything is a command.** New behaviour = a command in `crates/engine/src/commands.rs` (id,
   label, group, params doc, run). The UI, CLI, control channel and MCP server all dispatch by id.
3. **Every command has a functional test.** `every_command_has_a_functional_test` fails when a
   command id does not appear in a test outside the panic hunt. The panic hunt calls every command
   with hostile params and must stay green.
4. **Tests never open windows, start browsers, use the mic or speakers.** In-memory engines refuse
   such commands (`browser_allowed`); exhaustive fuzzers keep a skip list; real-device paths are
   `#[ignore]` tests that run headless. Test servers send `connection: close`.
5. **Never chain `cargo test && git commit`.** Run the tests, read the result, then commit.
6. **Secrets** (session cookies, API keys, connector configs) live in the OS keychain
   (`engine/src/secrets.rs`), never in the database or logs. Image bytes are not stored in the
   database; attachments are copied to `<data>/attachments` and read back per request.
7. **Tools follow Nod's rule** (n8n Nod, Apache-2.0): read-only tools run; anything else needs the
   owner's yes for exactly these arguments, once; the trust check must quote the owner's words and
   the quote is verified; background work refuses; tool output is untrusted data.
8. **Glyphs:** egui's bundled fonts lack ↑ → ⇩ ▾ ◧ ◨ ✓ ✕ ⧉ ＋ 🧠 and more. The kittest harness panics
   on missing glyphs; the app draws boxes (`MissingGlyphPolicy::Tofu`) and loads system fallback
   fonts. Check new UI symbols against the bundled fonts.

## 3. Build, test, look

```sh
cargo test --workspace --exclude aiscraft          # all tests (the app binary has none)
cargo clippy --workspace --all-targets
cargo test -p aiscraft-backends -- --ignored       # live: real Claude CLI, real headless browser
cargo run -p aiscraft-ui --example snapshot -- --demo --out shot.png \
  --script '[["chat.send", {"text": "Hallo"}], ["ui.show", {"what": "settings"}]]'
cargo build --release -p aiscraft -p aiscraft-cli
```

Look at UI changes in the offscreen snapshot (Read the PNG), never by opening the app on someone's
desktop: a new window steals their keyboard focus. `--demo` starts a local OpenAI-compatible server
that streams Markdown and can request a connector tool (prompt containing "Kollegium").

## 4. Keep up with AIS.chat (do this regularly)

AIS.ChatCraft talks to AIS.chat through internals of its Next.js app, not a public API. When AIS.chat
changes, the client breaks or misses features. **At least once per working week, and before every
release**, compare against the AIS.chat repository (`FWU-DE/ais-chat`, public, AGPL) and pull new
features in.

Last compared: **`d496d627c897` (2026-11-23)**. Update this line when you compare.

```sh
gh api repos/FWU-DE/ais-chat/compare/d496d627c897...main --jq '.commits[].commit.message' | head -80
gh api 'repos/FWU-DE/ais-chat/compare/d496d627c897...main' --jq '.files[].filename' | sort -u
```

What to check, in this order (paths in `FWU-DE/ais-chat`):

| What we rely on | Where | Our code |
|---|---|---|
| `sendChatMessageAction(conversationId, messages, modelId, fileIds, …)` and its result `{stream, messageId, error}` | `apps/chat-bot/src/app/api/chat/actions.ts`, `src/types/chat.ts` | `backends/src/aischat.rs` (`action_payload`, `map_action_event`) |
| Stream format: text deltas, `\x1e`-prefixed `ai_activity` events | `src/utils/streaming.ts`, `src/types/ai-activity.ts` (`aiActivityStepSchema`, tool names) | `aischat.rs` (`activity_steps`, `tool_label`) |
| Session cookie name, Auth.js config, 8 h lifetime | `src/auth/cookies.ts`, `src/auth/index.ts` | `SESSION_COOKIE`, `browser_login.rs` |
| Models in the page payload (`llm_model` fields, `priceMetadata.type`) | `packages/shared/src/db/schema.ts`, `llm-model-provider.tsx` | `rsc/src/scrape.rs` (`page_models`) |
| Token points (`usedBudget`, `maxBudget`) | `components/navigation/sidebar/app-sidebar.tsx` | `scrape::page_budget`, `AisChat::quota` |
| Conversation list and page (`/api/conversations`, `/d/<id>`) | `src/app/api/conversations/route.ts`, `(chat-bot)/d/[conversationId]` | `remote_conversations`, `remote_messages` |
| Uploads, speech | `src/app/api/v1/files`, `src/app/api/v1/speech/*` | `AisChat::upload`, `voice` crate (`speak_ais`) |
| Errors (`TokenPointsExceededError`, `ResponsibleAIError`, …) | `packages/ai-core/src/errors*` | `ais_error` |

Adopted since: assistants, characters, learning scenarios (lists via `get…ByFilterAction`, chats
via `sendChatMessageAction` with the id; `aischat.rs` `personas`), image generation
(`generateImageAction`, `generate_image`), links on messages (`ingestWebContentAction` where the
action is reachable, `check_link`) and `create_plot` diagrams (```` ```jsxgraph-json ```` blocks,
drawn by `ui/src/plot.rs`).

**Features AIS.chat has that AIS.ChatCraft does not (yet)** — check whether to adopt them:
creating and editing assistants, characters and scenarios, custom chats (`(chat-bot)/custom`),
image styles and input images, sharing with learners, top-up, new server-side tools in
`src/app/api/chat/tools/`. Open an issue per
feature worth adopting (label `ais-chat-parity`), with the AIS.chat paths and the user-visible goal.

When a protocol detail changes: update the scripted servers in `backends/src/aischat.rs` tests and
`engine/src/tests.rs` (`ais_chat_login_history_and_import`) first, so the tests describe the new
behaviour, then the code.

## 5. Nod parity

Feature parity with n8n Nod is tracked in the issues (Nod: …). Done: memory from the owner's own
words (with provenance, hourly tidy-up), trust check and approvals bound to exact arguments,
connectors (MCP, also for the Claude subscription via the tool bridge), schedules, page watches,
check-ins, usage ledger with monthly limit, history search and recall, learned skills, background
tasks, voice in and out, hints (nudges) with quiet hours, vault with placeholders and TOTP.
Not adopted on purpose: Nod's own sandbox computer and browser (AIS.ChatCraft has no webview and
runs no code), WhatsApp/Telegram channels (out of scope for a school client). Nod's repo (`n8n-io/nod`, Apache-2.0) is a reference for behaviour, not for
code; it is Node/Electron, AIS.ChatCraft is Rust.

## 6. Before you finish

```sh
cargo test --workspace --exclude aiscraft
cargo clippy --workspace --all-targets
```

Then commit with a message that says what changed for people, reference the issue (`Closes #n`),
and push. Rebuild the release binaries if the user runs the app (`target/release/aiscraft`).
