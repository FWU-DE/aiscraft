//! stdio transport: a child process speaking newline-delimited JSON-RPC.

use std::collections::{BTreeMap, VecDeque};
use std::process::Stdio as Pipe;
use std::sync::{Arc, Mutex, Weak};
use std::time::Duration;

use aiscraft_core::{Error, Result};
use serde_json::Value;
use tokio::io::{AsyncBufRead, AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::process::{Child, ChildStderr, ChildStdin, ChildStdout, Command};
use tokio::sync::{mpsc, oneshot};

use crate::MAX_MESSAGE_BYTES;
use crate::client::{Conn, Inner, lock, preview};

const STDERR_MAX_BYTES: usize = 64 * 1024;
const STDERR_MAX_LINES: usize = 500;
const STDERR_LINE_MAX: usize = 4 * 1024;
const QUEUE: usize = 64;

pub(crate) struct Stdio {
    tx: Mutex<Option<mpsc::Sender<Vec<u8>>>>,
    /// Held for `kill_on_drop` and to collect the exit status.
    child: tokio::sync::Mutex<Option<Child>>,
    stderr: Arc<Mutex<StderrRing>>,
}

pub(crate) struct Pipes {
    stdin: ChildStdin,
    stdout: ChildStdout,
    stderr: ChildStderr,
    rx: mpsc::Receiver<Vec<u8>>,
    ring: Arc<Mutex<StderrRing>>,
}

pub(crate) fn spawn(command: &str, args: &[String], env: &BTreeMap<String, String>, cwd: Option<&str>) -> Result<(Stdio, Pipes)> {
    let command = command.trim();
    if command.is_empty() {
        return Err(Error::invalid("Für den Konnektor ist kein Programm angegeben."));
    }
    let mut cmd = Command::new(command);
    cmd.args(args).envs(env).stdin(Pipe::piped()).stdout(Pipe::piped()).stderr(Pipe::piped()).kill_on_drop(true);
    if let Some(dir) = cwd.map(str::trim).filter(|d| !d.is_empty()) {
        if !std::path::Path::new(dir).is_dir() {
            return Err(Error::not_found(format!("Der Arbeitsordner „{dir}“ für den Konnektor existiert nicht.")));
        }
        cmd.current_dir(dir);
    }
    let mut child = cmd.spawn().map_err(|e| match e.kind() {
        std::io::ErrorKind::NotFound => Error::not_found(format!("Das Programm „{command}“ wurde nicht gefunden. Ist es installiert und im Suchpfad?")),
        std::io::ErrorKind::PermissionDenied => Error::refused(format!("Das Programm „{command}“ darf nicht ausgeführt werden.")),
        _ => Error::network(format!("Das Programm „{command}“ ließ sich nicht starten: {e}")),
    })?;
    let (Some(stdin), Some(stdout), Some(stderr)) = (child.stdin.take(), child.stdout.take(), child.stderr.take()) else {
        return Err(Error::network("Keine Verbindung zum Konnektor-Prozess (Ein-/Ausgabe fehlt)."));
    };
    let (tx, rx) = mpsc::channel(QUEUE);
    let ring = Arc::new(Mutex::new(StderrRing::default()));
    let conn = Stdio { tx: Mutex::new(Some(tx)), child: tokio::sync::Mutex::new(Some(child)), stderr: Arc::clone(&ring) };
    Ok((conn, Pipes { stdin, stdout, stderr, rx, ring }))
}

/// Start the writer, reader and stderr tasks. They hold only a `Weak`, so dropping the
/// client drops the child (killing it), which ends them.
pub(crate) fn start(rt: &tokio::runtime::Handle, inner: &Arc<Inner>, pipes: Pipes) {
    let Pipes { stdin, stdout, stderr, rx, ring } = pipes;
    let (stderr_done_tx, stderr_done_rx) = oneshot::channel();
    rt.spawn(write_loop(Arc::downgrade(inner), stdin, rx));
    rt.spawn(stderr_loop(inner.name.clone(), stderr, ring, stderr_done_tx));
    rt.spawn(read_loop(Arc::downgrade(inner), stdout, stderr_done_rx));
}

async fn write_loop(inner: Weak<Inner>, mut stdin: ChildStdin, mut rx: mpsc::Receiver<Vec<u8>>) {
    while let Some(mut line) = rx.recv().await {
        line.push(b'\n');
        let written = async {
            stdin.write_all(&line).await?;
            stdin.flush().await
        }
        .await;
        if let Err(e) = written {
            if let Some(inner) = inner.upgrade() {
                let tail = crate::client::stderr_tail_of(&inner, 600);
                inner.fail_all(Error::network(with_tail(format!("Der Konnektor „{}“ nimmt keine Eingaben mehr an ({e}).", inner.name), &tail)));
            }
            return;
        }
    }
    // Sender gone (close or drop): dropping stdin tells the server to exit.
}

async fn read_loop(weak: Weak<Inner>, stdout: ChildStdout, stderr_done: oneshot::Receiver<()>) {
    let mut reader = BufReader::with_capacity(64 * 1024, stdout);
    let mut buf = Vec::new();
    // Ends on EOF or a read error.
    while let Ok(Some(truncated)) = next_line(&mut reader, &mut buf, MAX_MESSAGE_BYTES).await {
        let Some(inner) = weak.upgrade() else { return };
        if truncated {
            log::warn!("MCP „{}“: Nachricht größer als 16 MiB verworfen", inner.name);
            continue;
        }
        let line = buf.trim_ascii();
        if line.is_empty() {
            continue;
        }
        match serde_json::from_slice::<Value>(line) {
            Ok(msg) => inner.dispatch(msg),
            Err(_) => log::warn!("MCP „{}“: Ausgabe ist kein JSON: {}", inner.name, preview(&String::from_utf8_lossy(line), 200)),
        }
    }
    // stdout closed: the server exited or closed it. Give stderr a moment to drain for the message.
    let _ = tokio::time::timeout(Duration::from_millis(300), stderr_done).await;
    let Some(inner) = weak.upgrade() else { return };
    let status = match &inner.conn {
        Conn::Stdio(s) => tokio::time::timeout(Duration::from_millis(500), async {
            let mut child = s.child.lock().await;
            match child.as_mut() {
                Some(c) => c.wait().await.ok(),
                None => None,
            }
        })
        .await
        .ok()
        .flatten(),
        Conn::Http(_) => None,
    };
    let code = status.and_then(|s| s.code()).map(|c| format!(" (Exit-Code {c})")).unwrap_or_default();
    let tail = crate::client::stderr_tail_of(&inner, 600);
    inner.fail_all(Error::network(with_tail(format!("Der Konnektor „{}“ wurde beendet{code}.", inner.name), &tail)));
}

async fn stderr_loop(name: String, stderr: ChildStderr, ring: Arc<Mutex<StderrRing>>, done: oneshot::Sender<()>) {
    let mut reader = BufReader::new(stderr);
    let mut buf = Vec::new();
    while let Ok(Some(truncated)) = next_line(&mut reader, &mut buf, STDERR_LINE_MAX).await {
        let mut line = String::from_utf8_lossy(buf.trim_ascii_end()).into_owned();
        if truncated {
            line.push('…');
        }
        log::debug!("MCP „{name}“ stderr: {line}");
        lock(&ring).push(line);
    }
    let _ = done.send(());
}

fn with_tail(mut msg: String, tail: &str) -> String {
    if !tail.is_empty() {
        msg.push_str(" Letzte Ausgabe: ");
        msg.push_str(tail);
    }
    msg
}

impl Stdio {
    pub(crate) async fn send(&self, bytes: Vec<u8>) -> Result<()> {
        let tx = lock(&self.tx).clone().ok_or_else(closed)?;
        tx.send(bytes).await.map_err(|_| closed())
    }

    pub(crate) fn try_send(&self, bytes: Vec<u8>) {
        if let Some(tx) = lock(&self.tx).as_ref()
            && tx.try_send(bytes).is_err()
        {
            log::debug!("MCP: Warteschlange voll oder geschlossen, Nachricht verworfen");
        }
    }

    pub(crate) fn stderr_tail(&self, max_chars: usize) -> String {
        lock(&self.stderr).tail(max_chars)
    }

    /// Close stdin, give the server two seconds to exit, then kill it.
    pub(crate) async fn close(&self) {
        lock(&self.tx).take();
        let mut child = self.child.lock().await;
        if let Some(c) = child.as_mut()
            && tokio::time::timeout(Duration::from_secs(2), c.wait()).await.is_err()
        {
            let _ = c.start_kill();
            let _ = tokio::time::timeout(Duration::from_secs(2), c.wait()).await;
        }
    }
}

fn closed() -> Error {
    Error::network("Die Verbindung zum Konnektor ist geschlossen.")
}

/// Read one line into `buf` (newline included), keeping at most `cap` bytes of it.
/// `Ok(None)` at end of stream, `Ok(Some(true))` if the line was longer than `cap`.
async fn next_line<R: AsyncBufRead + Unpin>(r: &mut R, buf: &mut Vec<u8>, cap: usize) -> std::io::Result<Option<bool>> {
    buf.clear();
    let mut truncated = false;
    let mut any = false;
    loop {
        let available = r.fill_buf().await?;
        if available.is_empty() {
            return Ok(any.then_some(truncated));
        }
        any = true;
        let (used, found) = match available.iter().position(|&b| b == b'\n') {
            Some(i) => (i + 1, true),
            None => (available.len(), false),
        };
        let room = cap.saturating_sub(buf.len());
        let part = available.get(..used).unwrap_or_default();
        if part.len() > room && !(found && part.len() == room + 1) {
            truncated = true;
        }
        buf.extend_from_slice(part.get(..part.len().min(room)).unwrap_or_default());
        r.consume(used);
        if found {
            return Ok(Some(truncated));
        }
    }
}

/// The last lines a server wrote to stderr, bounded in lines and bytes.
#[derive(Default)]
struct StderrRing {
    lines: VecDeque<String>,
    bytes: usize,
}

impl StderrRing {
    fn push(&mut self, line: String) {
        self.bytes += line.len();
        self.lines.push_back(line);
        while self.bytes > STDERR_MAX_BYTES || self.lines.len() > STDERR_MAX_LINES {
            match self.lines.pop_front() {
                Some(old) => self.bytes -= old.len(),
                None => break,
            }
        }
    }

    /// The newest lines, at most `max_chars` characters, oldest first.
    fn tail(&self, max_chars: usize) -> String {
        let mut picked = Vec::new();
        let mut used = 0;
        for line in self.lines.iter().rev().filter(|l| !l.trim().is_empty()) {
            let n = line.chars().count() + 1;
            if used + n > max_chars {
                if picked.is_empty() {
                    let skip = line.chars().count().saturating_sub(max_chars);
                    picked.push(line.chars().skip(skip).collect());
                }
                break;
            }
            used += n;
            picked.push(line.clone());
        }
        picked.reverse();
        picked.join("\n")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn lines_are_capped() {
        let data: &[u8] = b"short\nthis line is too long\nend";
        let mut r = BufReader::with_capacity(4, data);
        let mut buf = Vec::new();
        assert_eq!(next_line(&mut r, &mut buf, 6).await.unwrap(), Some(false));
        assert_eq!(buf, b"short\n");
        assert_eq!(next_line(&mut r, &mut buf, 6).await.unwrap(), Some(true));
        assert_eq!(buf, b"this l");
        assert_eq!(next_line(&mut r, &mut buf, 6).await.unwrap(), Some(false));
        assert_eq!(buf, b"end");
        assert_eq!(next_line(&mut r, &mut buf, 6).await.unwrap(), None);
    }

    #[test]
    fn stderr_ring_is_bounded() {
        let mut ring = StderrRing::default();
        for i in 0..2000 {
            ring.push(format!("line {i} {}", "x".repeat(100)));
        }
        assert!(ring.lines.len() <= STDERR_MAX_LINES);
        assert!(ring.bytes <= STDERR_MAX_BYTES);
        assert!(ring.tail(1000).ends_with(&format!("line 1999 {}", "x".repeat(100))));
        assert!(ring.tail(1000).chars().count() <= 1000);
        assert_eq!(ring.tail(5).chars().count(), 5);
    }
}
