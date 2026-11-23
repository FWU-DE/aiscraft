# aiscraft-collab

Live collaboration for AIS.ChatCraft's yrs artifacts (documents, whiteboards, later more). A
plug-in: the engine builds and runs without it (`aiscraft-engine` feature `collab`, on by
default), and nothing connects anywhere until someone sets a server and shares.

- `crates/collab` (this crate, L1): y-sync messages, join codes, the relay's HTTP API, and
  `Session`, which keeps one artifact in sync with its room and reconnects by state vector.
- `apps/aiscraft-collab-server`: the relay (one binary, rooms on disk, compaction, size caps,
  room tokens, `/healthz`). Deploy: `deploy/collab/` (Dockerfile, Coolify compose).
- Engine commands: `collab.set_server`, `collab.share`, `collab.join`, `collab.leave`,
  `collab.status` (`crates/engine/src/collab.rs`).

## Which relay

| | y-sweet | own relay (figcraft-server pattern) | Loro |
|---|---|---|---|
| CRDT | Yjs/yrs | Yjs/yrs (what documents and boards already are) | Loro's own; documents and boards would have to be migrated |
| Wire format | y-sync over WebSocket | y-sync over WebSocket (y-websocket framing) | Loro sync, no Yjs clients |
| Web clients later | Yjs + y-sweet client | Yjs + y-websocket provider | Loro only |
| Access | server key mints short-lived client tokens: needs a backend that holds the key | join code → room token, owner token ends the room; optional create key | none built in |
| Storage | S3-compatible or a directory | a directory: snapshot + synced log, compacted | none built in |
| Deploy | Rust binary (or Cloudflare) | one small Rust binary, Coolify compose here | own server to write anyway |
| Dependencies | its own yrs version, which must match ours | yrs 0.28 as in the engine, axum | another CRDT next to yrs |

**Decision:** the own relay. y-sweet speaks the same protocol, but its token model assumes a web
backend that holds the server key, which a desktop app handing a code to colleagues does not
have; Loro would mean leaving yrs and Yjs compatibility. Because the relay speaks plain y-sync, a
Yjs client (or y-sweet's protocol code) can still talk to it.

## Protocol

- `POST /rooms` `{room, kind, title}` (optionally `Authorization: Bearer <create key>`) →
  `{room, owner_token, token, code}`. The room id is the artifact id.
- `GET /join/<code>` with `Accept: application/json` → `{room, token, kind, title}`; in a browser
  a page that shows the code, never the token. 20 wrong codes per address and 10 minutes.
- `GET /r/<room>?token=<token or owner token>`: WebSocket, y-sync. Both sides send step 1 (state
  vector) and answer the other's with step 2; then updates. Custom message 101 = people in the
  room. Close codes: 4404 room ended, 4413 too large, 4429 full, 4400 unreadable, 1012 restart.
- `DELETE /rooms/<room>` with `Authorization: Bearer <owner token>` ends the room for everyone.
- `GET /healthz` → `ok`.

The server stores token hashes; the room token is sealed with the join code (stretched with
10 000 rounds of SHA-256), so the code is the only way to it. Every accepted change is in the log
and synced to disk before others get it.
