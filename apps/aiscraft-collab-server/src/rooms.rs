//! Rooms on disk and in memory.
//!
//! `<data>/rooms/<room>/meta.json` holds what the server knows about a room (kind, title, hashes
//! of its tokens and join code, the room token sealed with the code). `state.bin` is a snapshot
//! (one yrs v1 update), `log.bin` the changes since, each as `u32 LE length + v1 update`. Every
//! accepted change is in the log (and synced to disk) before anyone else hears of it; the log is
//! folded into a new snapshot when it grows and when the last person leaves.

use std::collections::HashMap;
use std::fs::{self, File, OpenOptions};
use std::io::{self, Read, Write};
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use tokio::sync::mpsc;

use aiscraft_collab::proto;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Meta {
    pub room: String,
    pub kind: String,
    pub title: String,
    pub created_at: u64,
    pub last_open: u64,
    pub owner_hash: String,
    pub token_hash: String,
    pub code_hash: String,
    /// The room token, XORed with a pad only the join code gives (hex).
    pub token_sealed: String,
}

#[derive(Default)]
pub struct Index {
    pub rooms: HashMap<String, Meta>,
    /// Code hash → room.
    pub codes: HashMap<String, String>,
}

pub fn now_secs() -> u64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0)
}

pub fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

fn unhex(s: &str) -> Option<Vec<u8>> {
    if !s.len().is_multiple_of(2) {
        return None;
    }
    (0..s.len() / 2).map(|i| s.get(2 * i..2 * i + 2).and_then(|h| u8::from_str_radix(h, 16).ok())).collect()
}

pub fn random<const N: usize>() -> io::Result<[u8; N]> {
    let mut b = [0u8; N];
    getrandom::fill(&mut b).map_err(|e| io::Error::other(e.to_string()))?;
    Ok(b)
}

pub fn sha_hex(s: &str) -> String {
    hex(&Sha256::digest(s.as_bytes()))
}

/// Compare secrets in constant time (for equal lengths).
pub fn same(a: &str, b: &str) -> bool {
    a.len() == b.len() && a.bytes().zip(b.bytes()).fold(0u8, |acc, (x, y)| acc | (x ^ y)) == 0
}

/// Rounds of SHA-256 a join code goes through, so a copy of the data directory does not give
/// codes away cheaply.
pub const STRETCH: u32 = 10_000;

/// A join code, stretched: what its room is found by, and the pad its token is sealed with.
pub struct CodeKey {
    pub hash: String,
    pad: [u8; 32],
}

impl CodeKey {
    pub fn new(code: &str) -> CodeKey {
        let mut k: [u8; 32] = Sha256::new().chain_update(b"aiscraft join code\0").chain_update(code.as_bytes()).finalize().into();
        for _ in 0..STRETCH {
            k = Sha256::digest(k).into();
        }
        let part = |what: &[u8]| -> [u8; 32] { Sha256::new().chain_update(what).chain_update(k).finalize().into() };
        CodeKey { hash: hex(&part(b"find\0")), pad: part(b"wrap\0") }
    }

    /// Seal a token of at most 32 bytes.
    pub fn seal(&self, token: &[u8]) -> String {
        hex(&token.iter().zip(self.pad).map(|(b, p)| b ^ p).collect::<Vec<u8>>())
    }

    pub fn open(&self, sealed: &str) -> Option<String> {
        let raw = unhex(sealed)?;
        if raw.len() > 32 {
            return None;
        }
        Some(hex(&raw.iter().zip(self.pad).map(|(b, p)| b ^ p).collect::<Vec<u8>>()))
    }
}

pub fn rooms_dir(data: &Path) -> PathBuf {
    data.join("rooms")
}

pub fn room_dir(data: &Path, room: &str) -> PathBuf {
    rooms_dir(data).join(room)
}

pub fn write_meta(data: &Path, meta: &Meta) -> io::Result<()> {
    let dir = room_dir(data, &meta.room);
    fs::create_dir_all(&dir)?;
    let tmp = dir.join("meta.json.tmp");
    fs::write(&tmp, serde_json::to_vec_pretty(meta).map_err(io::Error::other)?)?;
    fs::rename(tmp, dir.join("meta.json"))
}

/// Every room on disk. Unreadable ones are skipped with a log line.
pub fn load_index(data: &Path) -> io::Result<Index> {
    let mut index = Index::default();
    let dir = rooms_dir(data);
    fs::create_dir_all(&dir)?;
    for entry in fs::read_dir(&dir)?.flatten() {
        let path = entry.path().join("meta.json");
        let meta = fs::read(&path).ok().and_then(|b| serde_json::from_slice::<Meta>(&b).ok());
        match meta {
            Some(m) if proto::valid_room(&m.room) && entry.file_name().to_str() == Some(m.room.as_str()) => {
                index.codes.insert(m.code_hash.clone(), m.room.clone());
                index.rooms.insert(m.room.clone(), m);
            }
            _ => log::warn!("event=skipped dir={}", entry.file_name().to_string_lossy()),
        }
    }
    Ok(index)
}

/// What a peer's writer sends.
pub enum Out {
    Bin(Vec<u8>),
    Close(u16, String),
}

/// A room someone is in.
pub struct Live {
    pub doc: yrs::Doc,
    pub peers: HashMap<u64, mpsc::Sender<Out>>,
    dir: PathBuf,
    log: Option<File>,
    pub log_bytes: usize,
    pub snapshot_bytes: usize,
}

impl Live {
    /// Load snapshot and log; a torn last log record (a crash mid-write) is dropped.
    pub fn load(dir: PathBuf, max: usize) -> io::Result<Live> {
        fs::create_dir_all(&dir)?;
        let doc = yrs::Doc::new();
        let mut snapshot_bytes = 0;
        if let Ok(state) = read_capped(&dir.join("state.bin"), max.saturating_mul(2)) {
            snapshot_bytes = state.len();
            if !state.is_empty() {
                proto::apply(&doc, &state).map_err(|e| io::Error::other(format!("snapshot: {e}")))?;
            }
        }
        let mut replayed = 0;
        if let Ok(log) = read_capped(&dir.join("log.bin"), max.saturating_mul(2)) {
            let mut rest = log.as_slice();
            while let Some((len, tail)) = rest.split_first_chunk::<4>().map(|(l, t)| (u32::from_le_bytes(*l) as usize, t)) {
                let Some(rec) = tail.get(..len) else { break };
                if proto::apply(&doc, rec).is_err() {
                    break;
                }
                replayed += 1;
                rest = tail.get(len..).unwrap_or(&[]);
            }
        }
        let mut live = Live { doc, peers: HashMap::new(), dir, log: None, log_bytes: 0, snapshot_bytes };
        if replayed > 0 || live.dir.join("log.bin").exists() {
            live.compact()?;
        }
        Ok(live)
    }

    pub fn size(&self) -> usize {
        self.snapshot_bytes + self.log_bytes
    }

    /// Append an accepted change and sync it to disk.
    pub fn append(&mut self, delta: &[u8]) -> io::Result<()> {
        if self.log.is_none() {
            self.log = Some(OpenOptions::new().create(true).append(true).open(self.dir.join("log.bin"))?);
        }
        let len = u32::try_from(delta.len()).map_err(|_| io::Error::other("update too large"))?;
        if let Some(f) = self.log.as_mut() {
            let mut rec = Vec::with_capacity(delta.len() + 4);
            rec.extend_from_slice(&len.to_le_bytes());
            rec.extend_from_slice(delta);
            f.write_all(&rec)?;
            f.sync_data()?;
        }
        self.log_bytes += delta.len() + 4;
        Ok(())
    }

    /// Fold the log into a new snapshot.
    pub fn compact(&mut self) -> io::Result<()> {
        let full = proto::full_state(&self.doc);
        let tmp = self.dir.join("state.bin.tmp");
        {
            let mut f = File::create(&tmp)?;
            f.write_all(&full)?;
            f.sync_all()?;
        }
        fs::rename(&tmp, self.dir.join("state.bin"))?;
        self.log = None;
        match fs::remove_file(self.dir.join("log.bin")) {
            Ok(()) => {}
            Err(e) if e.kind() == io::ErrorKind::NotFound => {}
            Err(e) => return Err(e),
        }
        self.log_bytes = 0;
        self.snapshot_bytes = full.len();
        Ok(())
    }
}

fn read_capped(path: &Path, max: usize) -> io::Result<Vec<u8>> {
    let f = File::open(path)?;
    let mut out = Vec::new();
    f.take(max as u64 + 1).read_to_end(&mut out)?;
    if out.len() > max {
        return Err(io::Error::other("file too large"));
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use yrs::{GetString, ReadTxn, Text, Transact};

    #[test]
    fn codes_seal_tokens() {
        let k = CodeKey::new("ABCDEFGHJK");
        let token = random::<24>().unwrap();
        let sealed = k.seal(&token);
        assert_eq!(k.open(&sealed), Some(hex(&token)));
        assert_ne!(CodeKey::new("ABCDEFGHJ0").open(&sealed), Some(hex(&token)));
        assert!(k.open("zz").is_none());
        assert!(same("abc", "abc") && !same("abc", "abd") && !same("ab", "abc"));
    }

    #[test]
    fn log_survives_a_torn_tail_and_compacts() {
        let dir = std::env::temp_dir().join(format!("aiscraft-collab-rooms-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        let mut live = Live::load(dir.clone(), 1 << 20).unwrap();
        let src = yrs::Doc::new();
        let text = src.get_or_insert_text("content");
        let mut sent = Vec::new();
        for w in ["Eins ", "Zwei "] {
            let before = src.transact().state_vector();
            text.push(&mut src.transact_mut(), w);
            let u = src.transact().encode_state_as_update_v1(&before);
            let delta = proto::apply(&live.doc, &u).unwrap().unwrap();
            live.append(&delta).unwrap();
            sent.push(delta);
        }
        drop(live);
        // A crash mid-write leaves half a record.
        OpenOptions::new().append(true).open(dir.join("log.bin")).unwrap().write_all(&[200, 0, 0, 0, 1, 2]).unwrap();
        let live = Live::load(dir.clone(), 1 << 20).unwrap();
        assert_eq!(live.doc.get_or_insert_text("content").get_string(&live.doc.transact()), "Eins Zwei ");
        assert!(!dir.join("log.bin").exists(), "compacted on load");
        assert!(live.snapshot_bytes > 0);
        let _ = fs::remove_dir_all(&dir);
    }
}
