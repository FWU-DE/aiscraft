# AGENTS.md: guide for AI agents and contributors

**AIS.ChatCraft** is a native desktop client for [AIS.chat](https://app.ais-chat.schule) and other chat
backends (OpenAI-compatible servers, Claude via API, the Claude subscription through the local Claude
CLI). Rust only, egui/eframe, **no webview in the main app, no Electron, no Tauri**: HTML previews
(learning apps) run in the separate `aiscraft-preview` process (wry), optional via the app's
`preview` feature. It follows the conventions of the
storytold Craft apps (see `~/code/photocraft/AGENTS.md`): never crash, every action is a command,
tests are the gate. Product name in user-facing text: **AIS.ChatCraft** (AIS and AIS.chat are
different things; never shorten to "AIS"). Machine names stay `aiscraft-*`. User-facing text is
German.

Repo: `FWU-DE/aiscraft` (public since 2026-11-23; never commit code from the `ais-schule` org or secrets). Backlog: GitHub issues. AutoCraft (storytold/autocraft) is set up
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
  collab     L1  plug-in: live collaboration on yrs artifacts (y-sync over WebSocket, join codes);
                 engine feature `collab`, see crates/collab/README.md
  engine     L2  state, command registry, chat/tool loop, artifacts, schedules, connectors, usage
  ui         L3  egui shell (thin: renders state, dispatches commands)
apps/
  aiscraft      desktop app (+ `--control` JSON control channel, token-protected)
  aiscraft-cli  CLI (chat, login, status, search, export, `call`, `mcp` server)
  aiscraft-collab-server  relay for live collaboration (rooms on disk, compaction, /healthz)
  aiscraft-preview  HTML preview of one learning app (wry webview, own process, CSP, no network,
                    no navigation), and connector views (MCP Apps, `--bridge`: JSON-RPC over
                    stdin/stdout, network only to declared domains); needs WebKitGTK 4.1 on Linux
deploy/autocraft  Coolify compose for AutoCraft
deploy/collab     Dockerfile and Coolify compose for the collab relay
docs/craft-libs.md  which shared Craft libraries we use, defer or reject, and why
```

### Shared Craft libraries

Where a crate of [storytold/craft-libs](https://github.com/storytold/craft-libs) does the job, use
it instead of writing our own; check `docs/craft-libs.md` before adding plumbing. In use:
`craft-atomic` (every file write: exports, secrets, tokens, attachments), `craft-jsonrpc` +
`craft-mcp` (`aiscraft-cli mcp`), `craft-math` (LaTeX formulas in `ui/src/math.rs`), `craft-control-protocol` (`aiscraft --control`, requests are
`{"token", "id", "method", "params"}`). All entries in `[workspace.dependencies]` share one pin
(full rev until craft-libs tags a release); update them together and commit `Cargo.lock`.
The egui crates come from the storytold egui fork through `[patch.crates-io]`, like craft-libs
itself; keep that block in step with craft-libs' root `Cargo.toml` when bumping the pin.

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
   owner's yes for exactly these arguments, once, or the owner's own request (one request covers
   its steps); the trust check names the owner message it relies on (`message:N`) and lists each
   explicit term next to the action's value, and code verifies source and terms (`trust.rs`);
   background work refuses; tool output is untrusted data and never owner words.
8. **Glyphs:** egui's bundled fonts lack ↑ → ⇩ ▾ ◧ ◨ ✓ ✕ ⧉ ＋ 🧠 and more. The kittest harness panics
   on missing glyphs; the app draws boxes (`MissingGlyphPolicy::Tofu`) and loads system fallback
   fonts. Check new UI symbols against the bundled fonts.

## 3. Build, test, look

```sh
cargo test --workspace --exclude aiscraft          # all tests (+ `cargo test -p aiscraft`: control channel)
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
| Models in the page payload (`llm_model` fields, `priceMetadata.type`); repeated objects come as back-references (`"$1f:0:props:models:0:priceMetadata"`), e.g. `defaultImageModel` on `/image-generation` | `packages/shared/src/db/schema.ts`, `llm-model-provider.tsx`, `image-generation/page.tsx`, `image-model-provider.tsx` | `rsc/src/scrape.rs` (`page_models`, `resolve_ref`) |
| Favourites: `setFavoriteAction({entityType: assistant\|character\|learningScenario, entityId, isFavorite})`, shipped with the overview pages' JS | `(chat-bot)/actions/favorite-actions.ts`, `packages/shared/src/favorites/favorite-service.ts` | `AisChat::set_favorite`, `persona.favorite` |
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
Not adopted on purpose: Nod's own sandbox computer and browser (AIS.ChatCraft runs no code itself; learning
apps run only in the locked-down preview process or the browser), WhatsApp/Telegram channels (out of scope for a school client). Nod's repo (`n8n-io/nod`, Apache-2.0) is a reference for behaviour, not for
code; it is Node/Electron, AIS.ChatCraft is Rust.

Last compared with Nod's releases: **nod-v0.9.6 (2026-11-24)**; issues #63–#69. Check `gh release list -R n8n-io/nod` for newer ones and update this line.

## 6. Non-functional requirements and quality gates

Budgets a person notices. Each one has a gate, a test that fails when the budget breaks. Gates
count requests, file reads and pages, never wall-clock time, so they are stable in CI. A fix for
a slow path comes with such a gate.

| Requirement | Budget | Gate |
|---|---|---|
| Start-up shows what the last run knew | Persona list (favourites) and AIS.chat action ids come from `<data>/cache/<backend>.{personas,actions}.json` before any request; a broken cache file is ignored | `persona_list_and_action_ids_survive_a_restart` (engine) |
| Finding AIS.chat action ids | Each JS chunk loaded at most once per run, even when pages share it; with remembered ids no page or chunk is loaded | `action_discovery_loads_each_chunk_once_and_remembered_ids_load_none` (backends) |
| Persona lists | The three lists and the session load in parallel | none yet (code review) |
| Pictures in Entdecken load as cards come into view | Opening the page asks only for the pictures of the cards in view and about a row ahead (`PICTURE_LOOKAHEAD`); jumping over cards asks nothing for them; a picture is asked for once per run; at most 6 asks are open at once, 2 of them for cards not yet in view (`ui/src/explore.rs`), the engine downloads 4 at once | `explore_asks_only_for_pictures_in_view` (ui, `tests/explore_pictures.rs`) |
| Errors a person can fix are named | A full disk (`StorageFull`, `QuotaExceeded`, ENOSPC/EDQUOT, `SQLITE_FULL`) gives `aiscraft_core::DISK_FULL` with the details, classified from error codes, never from text; every engine file write goes through `engine/src/disk.rs`. A stopped local service (Ollama, LM Studio, Claude CLI, `aiscraft-preview`, a connector) is named with host:port or exit code and the next step | `every_engine_file_write_goes_through_disk`, `a_full_disk_while_saving_an_artifact_is_named_and_keeps_the_old_file` (engine), `a_store_that_cannot_grow_says_the_disk_is_full` (store), `a_stopped_local_server_is_named_with_host_and_port`, `claude_cli_that_ends_mid_answer_says_so` (backends) |
| Names stay on the device („Namen schützen“, #55) | With protection on, no class-list name, e-mail, phone, IBAN or street address is in any request body the backend receives (message, attached text, memory, system prompt, later rounds, single calls); the mapping is not in the state agents read nor in the JSON export | `no_real_name_reaches_the_backend`, `single_calls_are_protected_and_translated_back` (engine, `privacy/tests.rs`) |
| One request, one question at most (#63) | A batch of steps from one request asks nothing; a call nobody asked for asks once; one trust check per call that needs a yes, none for reads; the check's request holds the owner's messages with the answers they replied to and never a tool result | `one_request_covers_every_step` (engine, `trust_tests.rs`) |
| Own rules (#22) | An allowing rule runs what it names without a question; a prohibition refuses without a question, also against a covering request, and also with the trust check off; background work stays closed; the check's request carries the connector's rules with their kind | `own_rules_allow_forbid_and_prohibitions_win` (engine, `trust_tests.rs`) |
| One approval, one result (#24) | An approved or covered action is claimed in the store before it runs; a second answer, a claim from another window or a repeat in the same request does not run it; a call cut off midway is uncertain, never retried, and asks next time; after a restart running claims become uncertain | `one_approval_one_result`, `a_call_cut_off_midway_is_uncertain_and_never_retried`, `a_covered_action_runs_once_per_request`, `running_actions_become_uncertain_after_a_restart` (engine), `a_claim_is_taken_once_and_running_claims_become_uncertain` (store) |
| Action log (#23) | Every tool action is logged with target, short arguments and why (read, request with source and words, rule, approval, refusal); model calls with tokens and cost; no vault value in the log or the CSV; capped at 5000 entries | `action_log_says_what_ran_and_why` (engine), `action_log_keeps_the_newest_entries` (store) |
| Panels keep their size | In every main view (start page, chat with a long Markdown answer, chat + document, H5P, learning app, whiteboard, worksheet, the link field, every settings tab, the chat import dialog, Entdecken, the welcome) at 100 % (1280×800) and 150 % (1333×889), in Modern, Glas and Omarchy: no panel grows from frame to frame, sidebar and artifact panel keep their default width, no control sticks out of the window or across a panel edge, no two controls of one row lie on each other. Rows with trailing buttons use `widgets::field_then_buttons` (buttons first, the field takes the rest), never `available_width() - N` | `layout_gate_modern`, `layout_gate_glas`, `layout_gate_omarchy` (ui, `tests/layout_gate.rs`; `AisCraftApp::panel_layout`) |
| UI tests don't depend on load or on each other | No process-wide state the UI reads per frame: each app has its own Omarchy theme source (`theme::OmarchyTheme`, tests pass theirs through `common::app`), and toasts and the theme watcher count egui's input time, not the wall clock. `cargo test -p aiscraft-ui --test ui_flows --no-run`, then `seq 1 16 \| xargs -P 8` over the test binary (and `layout_gate`): 0 failures | `each_app_reads_its_own_omarchy_theme` (ui, `tests/ui_flows.rs`); the stress run before a change to the UI tests |
| Never crash on hostile input | Caps on every size, no panics | workspace lints, panic hunt |
| Tests stay off the desktop | No windows, browser, mic or speakers | rule 2.4, `browser_allowed` |

Live check against the real AIS.chat (signed in): cold, with no cache, `persona.refresh` takes
about 25 s; warm, with cached ids, about 3 s. The app shows the cached list at once. Measure with
`time aiscraft-cli call persona.refresh '{"backend":"ais-chat"}'`; delete `<data>/cache/` for the
cold number. If the warm number grows past 5 s, look for a missing cache before blaming AIS.chat.

## 7. Before you finish

```sh
cargo test --workspace --exclude aiscraft
cargo clippy --workspace --all-targets
```

Then commit with a message that says what changed for people, reference the issue (`Closes #n`),
and push. If the user runs the app, reinstall it with `packaging/install.sh` (binaries in
`~/.local/bin`, launcher entry and icons under `~/.local/share`) and restart it.

Packaging: `packaging/install.sh` (per user, no sudo), `packaging/PKGBUILD` (Arch), the launcher
entry `packaging/linux/ai.uds.aischatcraft.desktop` and icons in `packaging/icons/` (the AIS.chat logo
from FWU-DE/ais-chat on a white tile, rendered with `rsvg-convert`).

Releases: set `[workspace.package] version` in `Cargo.toml`, commit, push a tag `v<version>`.
`.github/workflows/release.yml` checks that tag against the version and calls the Craft suite's
shared release workflow (`storytold/craft-common-actions`, `release.yml`), which runs our
`packaging/linux/package.sh`, `packaging/linux/flatpak-bundle.sh`, `packaging/macos/package.sh` and
`packaging/windows/package.ps1` and creates a draft release with `SHA256SUMS.txt`. Package names
follow craft-update (`aischatcraft-<version>-<os>-<arch>.<ext>`); `update.rs` finds them with
`pick_asset`. Windows (Azure Trusted Signing or a PFX) and macOS (Developer ID, notarization) are
signed once their secrets are in the repository's `release` environment; the shared workflow signs
Linux packages only on a `release` branch, so tag builds stay unsigned there. "Run workflow" on
Release with dry-run builds the packages and publishes nothing.

For now (owner's decision, 2026-11-25) releases have no macOS package and no signatures: our
workflow always calls the shared one in dry-run mode (no macOS, whose `macos-15-xlarge` runner is
paid for, no signing, no draft), and its own job `publish` drafts the release with the suite's
`draft-release` action and publishes it on a tag. Once the runner is allowed and the certificates
are in the `release` environment, go back to the shared workflow's own draft (see the comment in
`release.yml`).
