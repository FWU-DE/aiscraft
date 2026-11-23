# Shared Craft libraries in AIS.ChatCraft

AIS.ChatCraft uses the shared libraries of the storytold Craft apps
([storytold/craft-libs](https://github.com/storytold/craft-libs), MIT OR Apache-2.0, crates
`craft-<name>`) wherever one does the same job as our own code.

## Pin

All craft-libs entries in the root `Cargo.toml` use the same source:
`git = "https://github.com/storytold/craft-libs", rev = "249a41bd9306a1b34330ac94e596a1a8b881378a"`
(main, 2026-11-23). The README and CHANGELOG describe a `v0.1.0` tag, but the repository has no
tags or releases yet (`gh api repos/storytold/craft-libs/tags` is empty), so we pin a full commit
as `docs/versioning.md` allows. Switch to `tag = "vX.Y.Z"` once one exists. Update all entries
together, never mix two revs (duplicate crate instances), run the tests, commit `Cargo.lock`.

craft-libs `main` is on egui 0.37 (the README still says 0.36), the same as ours.

## Map

Fork = needs the storytold egui fork (`[patch.crates-io]`, see below).

| craft-lib | What it does | Our code | Fit | Fork | Decision |
|---|---|---|---|---|---|
| craft-atomic | Crash-safe atomic writes, private (0600) staging, exclusive create | `std::fs::write` in exports, `secrets.rs` tmp+rename, Claude CLI MCP config, control token, attachments | exact, no deps | no | **adopted** |
| craft-mcp + craft-jsonrpc | MCP session (initialize, lifecycle), JSON-RPC envelopes, batches, bounded lines | hand-written loop in `aiscraft-cli mcp` | exact; we keep routing | no | **adopted** |
| craft-mcp-server | Full stdio MCP server with tool schemas and argument checks | `aiscraft-cli mcp` | `check_arguments` rejects every argument a schema doesn't list; our params are free-text docs and partly undocumented (`files`, `links`, `force` …) | no | no (needs typed schemas per command first) |
| crates/mcp (ours) | MCP **client** (stdio, Streamable HTTP) for connectors | — | craft-libs has no MCP client | — | keep ours |
| craft-control-protocol | Authenticated JSON-lines control framing, constant-time token check, bounded lines | `apps/aiscraft --control` | exact; request key is `method` (was `cmd`) | no | **adopted** |
| craft-control | Full control server (hub, waits, cancel, document watch, client) | `apps/aiscraft --control` | built for document apps with a UI-thread request queue; ours dispatches straight to the engine | no | later (if we need waits/cancel) |
| craft-web-api | `window.<app>` for web builds | — | we have no web build | no | no |
| craft-http-url | HTTP URL parsing, redirects, redacted display | reqwest/url already parses | small gain | no | later (redacted URLs in logs) |
| craft-sqlite | SQLite *file format* wire helpers | `crates/store` (rusqlite) | different job | no | no |
| craft-settings | `config.toml` settings, atomic saves, migrations | settings live in SQLite (`store`) | would mean a data migration | no | no |
| craft-dialogs | Non-blocking native file dialogs (rfd), fixes the macOS re-entrancy abort | blocking `rfd::FileDialog` in `ui` (6 call sites) | good, no egui dep; answers arrive on a later frame, so each call site needs state | no | later (after the collab and persona branches merge: touches `conversation.rs`, `artifact_panel.rs`) |
| craft-platform | Dialog lifecycle, XDG appearance (light/dark) | rfd, our Omarchy `ThemeWatcher` | partial | optional | later, with craft-dialogs |
| craft-quit | macOS quit and app menu | — | no unsaved documents, Linux first | no | no |
| craft-update | Release check and self-update | PKGBUILD / `install.sh` | no release feed yet | no | later |
| craft-i18n | Translation catalogs, plurals, locale | German strings in code | UI is German only | no | no |
| craft-font-catalog | Installed fonts on every platform, lookup by name | `fc-match` subprocess in `ui/theme.rs` (Linux only) | works, but the first lookup scans every font: 860 ms for 968 faces here vs 13 ms per `fc-match` | no | later (with a background scan) |
| craft-ui | egui widgets, icons, tabs, docking, theme, `fonts::UiFonts` (suite font stack with system fallback) | `ui/widgets.rs`, `ui/theme.rs` | `UiFonts` could replace our DejaVu fallbacks; widgets would touch sidebar/conversation | builds on stock egui 0.37 too; fork recommended | later (`UiFonts` first) |
| craft-menu, craft-keymap, craft-window | Menu bar, shortcuts, window frame | palette shortcuts in `commands.rs`, eframe frame | would restructure shortcuts and chrome | yes | later |
| craft-markdown, craft-flow | Markdown to and from the craft-flow model | egui_commonmark (render), pulldown-cmark (HTML export) | export path for #38 | no | later (#38) |
| craft-docx, craft-xlsx, craft-pptx, craft-pdf | Office and PDF read/write | — | Word/Excel/PowerPoint/PDF export (#38) | no | later (#38) |
| craft-latex, craft-math | LaTeX documents; equations as one tree (OMML, MathML, LaTeX) | — | parsing for formulas (#36); drawing stays ours | no | later (#36) |
| craft-ical, craft-dav, craft-vcard | iCalendar, CalDAV/CardDAV, vCard | — | calendar connectors (#26) | no | later (#26) |
| craft-ai-core / craft-ai | AI connection/model contract | `backends`, `engine` | AIS.chat is a Next.js server action, not a provider API | no | no |

## egui fork

craft-libs builds against `storytold/egui` (rev `6d518fc7…`) and `storytold/winit` (rev
`a517b384…`) through `[patch.crates-io]`; a git dependency's patches do not reach us, so a
consumer adds the same block. Checked on 2026-11-23: with the block, the workspace builds without
warnings, all tests (kittest included) pass, and the `--demo` snapshot is byte-identical to stock
egui (the chat + settings snapshot differs only in the demo server's port). The switch is its own
commit so it can be dropped. Not verified: interactive input on a real desktop (winit fork).
