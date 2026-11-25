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
| craft-update | Release check and self-update | — | `check_for_update` reads FWU-DE/aiscraft's GitHub releases (published, stable only; drafts and pre-releases skipped), links and assets checked; its egui `StartupPrompt` is a modal with ArtCraft branding, so the hint is our own (`ui/src/update.rs`) | no (without feature `egui`) | **adopted, check only** (see below) |
| craft-i18n | Translation catalogs, plurals, locale | German strings in code | UI is German only | no | no |
| craft-font-catalog | Installed fonts on every platform, lookup by name | `fc-match` subprocess in `ui/theme.rs` (Linux only) | works, but the first lookup scans every font: 860 ms for 968 faces here vs 13 ms per `fc-match` | no | later (with a background scan) |
| craft-text | Hyphenation (Liang patterns, German through hypher), bidi, line breaking | — | the German hyphenation points as syllables for the readability of an answer (`readability.rs`, #48); only the `hyphen` + `german` features | no | **adopted** (#48) |
| craft-ui | egui widgets, icons, tabs, docking, theme, `fonts::UiFonts` (suite font stack with system fallback) | `ui/widgets.rs`, `ui/theme.rs` | `UiFonts` could replace our DejaVu fallbacks; widgets would touch sidebar/conversation | builds on stock egui 0.37 too; fork recommended | later (`UiFonts` first) |
| craft-menu, craft-keymap, craft-window | Menu bar, shortcuts, window frame | palette shortcuts in `commands.rs`, eframe frame | would restructure shortcuts and chrome | yes | later |
| craft-markdown + craft-docx | Markdown to a craft-wordmodel `Document`; DOCX written from it | — | exact: `import(md)` → `write(doc)` | no | **adopted** (#38: document export as Word, `document_file_create`) |
| craft-xlsx | SpreadsheetML read and **write** (`Document::write`, a new workbook from fields) | — | exact: sheets, shared strings, styles, formulas, frozen header | no | **adopted** (#38: `spreadsheet_create`) |
| craft-pptx | PresentationML read and **write** (`export`), `Plain` master | — | exact: text boxes, bullets, tables, notes | no | **adopted** (#38: `slides_create`) |
| craft-opc | The OOXML ZIP package | — | tests open our .docx/.xlsx/.pptx with it | no | **adopted** (tests) |
| craft-zip | ZIP from untrusted files: entry-count guard, budgeted reads, duplicate detection, deterministic writer | — | exact: `.h5p` packages are zips (#59) | no | **adopted** (`learn/h5p.rs`: H5P packages read and written; entry names and file types checked by us) |
| craft-pdf | PDF read/render (hayro), write (Krilla), Standard-14 metrics, object layer (`cos`) | — | `write` needs the vendored Krilla through a `[patch.crates-io]` entry, an embedded font and our own text layout; feature `fonts` alone has the Standard-14 metrics and WinAnsi encoding without dependencies | no | **adopted, features `fonts`, `cos` and `raster`** (see below; `cos` builds the page subset of #47; `raster` draws the worksheet print preview and needs craft-libs' vendored hayro and vello through `[patch.crates-io]` git entries at the same pin); the interpreter behind `raster` also reads the text of PDFs for project knowledge (#32, `attach/pdf_text.rs`: a text device over `craft_pdf::interpret_page`, glyph positions with kurbo) |
| craft-flow | The flow model behind craft-markdown | pulldown-cmark (HTML export, PDF layout) | comes with craft-markdown | no | indirect |
| craft-math | Equations as one tree (OMML, MathML, LaTeX math, UnicodeMath), bounded readers | — | `math_latex::parse_latex` reads the formulas in answers and documents (#36); layout and drawing are ours (`ui/src/math.rs`) | no | **adopted** |
| craft-latex | Whole LaTeX documents to and from the craft-flow model | — | answers hold formulas, not `.tex` documents | no | later (LaTeX export with #38) |
| craft-ical, craft-vcard | iCalendar content lines and dates, vCard contacts | — | `.ics` → a readable list of events, `.vcf` → contacts as Markdown before an AIS.chat upload (`attach/docs.rs`, #45); `.vcf` class lists for „Namen schützen“ (`privacy/detect.rs`, #55) ; dates from an answer written as `.ics` (content lines, folding, escaping, `write_vtimezone`; the `VEVENT`s are ours, `learn/calendar.rs`, #51) | no | **adopted** (attachments, calendar files; calendar connectors later, #26) |
| craft-dav | CalDAV/CardDAV | — | calendar connectors (#26) | no | later (#26) |
| craft-doc-binary, craft-rtf (`doc`), craft-epub + craft-html (`flow`), craft-mime | Word 97-2003, RTF, EPUB, HTML, e-mail and mbox readers | — | `.doc`, `.rtf`, `.epub`, `.eml`, `.mbox` → word model → craft-markdown before an AIS.chat upload (`attach/docs.rs`, #45); password-protected `.doc` and DRM e-books get a message, not decrypted | no | **adopted** |
| craft-xls, craft-ods, craft-csv | Excel 97-2003 and OpenDocument spreadsheet readers, CSV quoting | — | `.xls` → CSV (one sheet) or Markdown tables (#45); one sheet of `.xls`/`.xlsx`/`.ods` as CSV (#47, `attach/sheets.rs`); class lists from CSV (`craft_csv::decode_text`, `sniff_delimiter`, `records`) and sheets for „Namen schützen“ (#55, `privacy/mod.rs`) | no | **adopted** |
| craft-ppt-binary | PowerPoint 97-2003 reader into the craft-pptx model | — | `.ppt` → Markdown; some slides of `.ppt`/`.pptx` (#47, `attach/slides.rs`) | no | **adopted** |
| craft-image-codecs (+ craft-heif, `avif-decode`), craft-metadata | Bounded image decode/encode, HEIC and AVIF decoders; EXIF/XMP reading | — | photos for AIS.chat decoded upright, scaled to 1000 px (our box filter: the crate has no resize), encoded as JPEG/PNG without metadata; `has_location_metadata` and EXIF GPS say "Standortdaten entfernt" (#50, `attach/image.rs`). HEIC behind the engine feature `heif` (default on; HEVC patents are a distributor's call) | no | **adopted** |
| craft-ai-core / craft-ai | AI connection/model contract | `backends`, `engine` | AIS.chat is a Next.js server action, not a provider API | no | no |

## egui fork

craft-libs builds against `storytold/egui` (rev `6d518fc7…`) and `storytold/winit` (rev
`a517b384…`) through `[patch.crates-io]`; a git dependency's patches do not reach us, so a
consumer adds the same block. Checked on 2026-11-23: with the block, the workspace builds without
warnings, all tests (kittest included) pass, and the `--demo` snapshot is byte-identical to stock
egui (the chat + settings snapshot differs only in the demo server's port). The switch is its own
commit so it can be dropped. Not verified: interactive input on a real desktop (winit fork).

## PDF (#38)

Markdown becomes PDF in `crates/engine/src/office/pdf.rs`, a small writer of our own (about 600
lines with tests): pulldown-cmark events, greedy line breaking with craft-pdf's Standard-14
metrics (`craft_pdf::fonts::Std14::text_width`), WinAnsi text (`craft_pdf::fonts::win_ansi`), A4
pages with Helvetica and Courier, which every viewer has, so no font is embedded. craft-pdf's
`write` feature (Krilla) was not used: it requires the vendored Krilla patch in our workspace,
a bundled font file and still leaves pagination and line breaking to the app. Limit: characters
outside WinAnsi (emoji, Greek, CJK) print as a stand-in or `?`. Switch to `write` with an
embedded Unicode font when that matters.

## Update check

`crates/engine/src/update.rs`: `craft_update::check_for_update(&APP, &Http, CARGO_PKG_VERSION)`
on a blocking task, at start (5 s after, at most once in 24 h, silent on errors) and with
`app.check_update`. The answer (last check, newest release, skipped version) is stored under the
settings key `update`; `update_check` in `Settings` switches it off. Engines in memory and
`cfg!(test)` builds never ask GitHub; tests hand them release JSON (`Engine::set_update_feed`),
which goes through craft-update's `parse_github_release_list` and `version::offers_update`, the
same as `check_for_update` (its own HTTP client is HTTPS-only outside craft-update's tests, so a
local scripted server can't stand in). `App` id `aischatcraft` (the release asset stem),
`bundle_id_override` `ai.uds.aischatcraft`. craft-update brings its own HTTP stack (ureq, rustls
with ring, platform verifier) next to reqwest.

Release packages (#58) come from craft-common-actions' shared `release.yml`
(`.github/workflows/release.yml` calls it on tags `v*`; our `packaging/` scripts build them) and
carry the names `pick_asset` reads, `aischatcraft-<version>-<os>-<arch>.<ext>`: AppImage, `.deb`,
`.rpm`, Flatpak and `.tar.gz` for Linux (x86_64, aarch64), a universal `.dmg` for macOS, MSI and
`…-windows-<arch>-portable.zip` (with `portable.txt`) for Windows x64 and arm64, and
`SHA256SUMS.txt`. `app.update_open` picks this computer's package with `pick_asset`. `main` runs
`self_update::finish` first (`aiscraft_engine::finish_update_helper`), so the restart helper works
once the self-update is switched on.

Not adopted yet: the self-update itself (`self_update::prepare_update`, `Updater`). Missing: signed
Windows builds (unsigned copies get no update offered; the signing secrets go into the repo's
`release` environment), the portable data folder beside the exe when `portable.txt` is there, and
dialogs without ArtCraft branding.

Worksheets (`engine/src/learn/print.rs`) print through the same writer: the worksheet becomes
Markdown (answer lines as escaped underscores, `( )`/`[ ]` instead of ballot boxes, which
WinAnsi lacks) and then PDF or, through craft-markdown + craft-docx, Word.

## HTML preview (not a craft-lib)

Learning apps are shown by `apps/aiscraft-preview`, a separate program on `wry` 0.57 + `tao`
0.37 (one window, one webview; WebKitGTK 4.1 on Linux). craft-libs has no webview crate, and
the main app links none on purpose.

## Page subsets (#47)

`attach/pdf.rs` opens the PDF with `craft_pdf::cos`, collects the page tree's leaves (with the
inherited `Resources`, `MediaBox`, `CropBox`, `Rotate`), points the catalog at a new flat page
list of the chosen pages and writes a full, garbage-collected copy. Outlines, named
destinations, forms, structure tree, page labels and link/widget annotations are left out,
because they can reach pages that were not chosen; the result is reopened and must hold exactly
the chosen number of `/Page` objects, else the subset is refused. No reader patches are needed.
