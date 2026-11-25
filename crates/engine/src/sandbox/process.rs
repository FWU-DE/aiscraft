//! Running a host program for a runtime (`wsl.exe`, `limactl`, `tar`): stdin fed in, both
#![cfg_attr(not(any(windows, target_os = "linux")), allow(dead_code))]
//! streams capped, ended at the time limit, never a console window on Windows.

use std::path::Path;
use std::process::Stdio;
use std::time::Duration;

use aiscraft_core::{Error, Result};
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWriteExt};

use super::runtime::Output;

#[cfg(windows)]
const CREATE_NO_WINDOW: u32 = 0x0800_0000;

/// Run `exe` with `args` and `env`. A program that is not there gives `missing` as the message.
pub(crate) async fn run(exe: &Path, args: &[String], env: &[(&str, String)], stdin: Vec<u8>, timeout: Duration, max: usize, missing: &str) -> Result<Output> {
    let mut cmd = tokio::process::Command::new(exe);
    cmd.args(args).stdin(Stdio::piped()).stdout(Stdio::piped()).stderr(Stdio::piped()).kill_on_drop(true);
    for (k, v) in env {
        cmd.env(k, v);
    }
    #[cfg(windows)]
    cmd.creation_flags(CREATE_NO_WINDOW);
    let mut child =
        cmd.spawn().map_err(|e| if e.kind() == std::io::ErrorKind::NotFound { Error::unsupported(missing) } else { Error::io(exe.display(), &e) })?;
    let mut input = child.stdin.take();
    let out = child.stdout.take();
    let err = child.stderr.take();
    let feed = async move {
        if let Some(i) = input.as_mut() {
            let _ = i.write_all(&stdin).await;
            let _ = i.shutdown().await;
        }
        drop(input);
    };
    let work = async {
        let (_, (o, ocut), (e, ecut)) = tokio::join!(feed, read_capped(out, max), read_capped(err, max));
        let status = child.wait().await;
        (o, ocut || ecut, e, status)
    };
    match tokio::time::timeout(timeout, work).await {
        Ok((stdout, truncated, stderr, status)) => {
            let status = status.map_err(|e| Error::io(exe.display(), &e))?;
            Ok(Output { code: status.code(), stdout, stderr, timed_out: false, truncated })
        }
        // The child is dropped and killed with the future.
        Err(_) => Ok(Output { code: None, timed_out: true, ..Default::default() }),
    }
}

/// Start `exe` with `args` and `env` and keep its stdin and stdout open (stderr dropped). The
/// program ends when the returned pipe is dropped.
pub(crate) fn spawn(exe: &Path, args: &[String], env: &[(&str, String)], missing: &str) -> Result<super::runtime::Pipe> {
    let mut cmd = tokio::process::Command::new(exe);
    cmd.args(args).stdin(Stdio::piped()).stdout(Stdio::piped()).stderr(Stdio::null()).kill_on_drop(true);
    for (k, v) in env {
        cmd.env(k, v);
    }
    #[cfg(windows)]
    cmd.creation_flags(CREATE_NO_WINDOW);
    let mut child =
        cmd.spawn().map_err(|e| if e.kind() == std::io::ErrorKind::NotFound { Error::unsupported(missing) } else { Error::io(exe.display(), &e) })?;
    let (Some(input), Some(output)) = (child.stdin.take(), child.stdout.take()) else {
        return Err(Error::protocol(format!("{}: keine Verbindung zum Programm", exe.display())));
    };
    Ok(super::runtime::Pipe { reader: Box::new(output), writer: Box::new(input), guard: Box::new(child) })
}

async fn read_capped<R: AsyncRead + Unpin>(r: Option<R>, max: usize) -> (Vec<u8>, bool) {
    let Some(mut r) = r else { return (Vec::new(), false) };
    let mut out = Vec::new();
    let mut cut = false;
    let mut buf = vec![0u8; 16 * 1024];
    loop {
        match r.read(&mut buf).await {
            Ok(0) | Err(_) => break,
            Ok(n) => {
                let room = max.saturating_sub(out.len());
                out.extend_from_slice(buf.get(..n.min(room)).unwrap_or(&[]));
                // Keep reading (and dropping), so the program is not stuck on a full pipe.
                cut |= n > room;
            }
        }
    }
    (out, cut)
}

/// Decodes base64 arguments and runs them (`$0` is `aiscraft`): what no quoting rule of a
/// Windows command line or an ssh command string can change.
pub(crate) const DECODER: &str = r#"for a do set -- "$@" "$(printf %s "$a" | base64 -d)"; shift; done; exec "$@""#;

pub(crate) fn b64(s: &str) -> String {
    use base64::Engine as _;
    base64::engine::general_purpose::STANDARD.encode(s.as_bytes())
}
