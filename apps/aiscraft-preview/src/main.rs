//! `aiscraft-preview --file <page.html> [--title <t>] [--watch-stdin]`
//! `aiscraft-preview --file <view.html> --bridge [--connect <https://host>]… [--resource <https://host>]…`
//!
//! Shows one learning app (a local HTML file AIS.ChatCraft wrote into its data folder) in a
//! single webview window, in its own process, so the main app never links a webview.
//!
//! Locked down: the page is served from a private `aiscraft://` scheme (not `file://`, so it
//! cannot read other local files) with a Content-Security-Policy header that allows inline
//! code only and no network at all; every navigation away from the page and every new window
//! is refused; downloads are refused; developer tools only in debug builds. With
//! `--watch-stdin` the window closes when stdin reaches its end, i.e. when AIS.ChatCraft,
//! which holds the other end of the pipe, exits.
//!
//! `--bridge` shows a connector's view (MCP Apps, #74). The view talks JSON-RPC with
//! `window.parent.postMessage`; here the page is the top window, so a small script stands in for
//! the parent: what the view posts goes to stdout as one JSON line, and each JSON line on stdin
//! is delivered to the view as a `message` event. AIS.ChatCraft answers on the other end and
//! applies its approval rules there. The network stays closed except for the domains given with
//! `--connect` (requests) and `--resource` (scripts, styles, images, fonts), each checked again
//! here: `https://host[:port]` or `https://*.host`, never `*`. Lines `{"aiscraft/window": …}`
//! from the app steer the window instead (the size a view asked for, within caps; the small
//! window on top for the `pip` display mode) and never reach the view.
// No console window next to the preview on Windows (#58); stdin/stdout stay usable, the app
// talks to the preview through pipes it creates itself.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod h5p;

use std::borrow::Cow;
use std::io::{BufRead as _, Read as _, Write as _};
use std::path::PathBuf;

use tao::dpi::LogicalSize;
use tao::event::{Event, WindowEvent};
use tao::event_loop::{ControlFlow, EventLoopBuilder};
use tao::window::WindowBuilder;
use wry::http::{Response, header::CONTENT_TYPE};
use wry::{NewWindowResponse, WebViewBuilder};

/// The same policy AIS.ChatCraft writes into the page (`engine/src/learn`), sent as a header too.
const CSP: &str = "default-src 'none'; script-src 'unsafe-inline'; style-src 'unsafe-inline'; img-src data: blob:; media-src data: blob:; font-src data:; form-action 'none'; base-uri 'none'";
const MAX_PAGE: u64 = 4 * 1024 * 1024;
/// Longest message between view and app.
const MAX_MESSAGE: usize = 1024 * 1024;
const MAX_DOMAINS: usize = 20;

/// Stands in for `window.parent` of a view: posts go to the app, the app's answers come back as
/// `message` events whose source is this stand-in.
const BRIDGE_JS: &str = r#"(() => {
  const host = { postMessage: (m) => { try { window.ipc.postMessage(typeof m === 'string' ? m : JSON.stringify(m)); } catch (e) {} } };
  try { Object.defineProperty(window, 'parent', { get: () => host, configurable: false }); } catch (e) {}
  Object.defineProperty(window, '__aiscraftDeliver', { value: (m) => {
    const e = new MessageEvent('message', { data: m, origin: window.location.origin });
    try { Object.defineProperty(e, 'source', { value: host }); } catch (_) {}
    window.dispatchEvent(e);
  }, writable: false, configurable: false });
})();"#;

/// Where the custom scheme lives: WebView2 maps custom schemes to http(s)://<name>.localhost.
#[cfg(target_os = "windows")]
const START: &str = "http://aiscraft.localhost/";
#[cfg(not(target_os = "windows"))]
const START: &str = "aiscraft://localhost/";

fn arg(args: &[String], name: &str) -> Option<String> {
    args.iter().position(|a| a == name).and_then(|i| args.get(i + 1).cloned())
}

fn all_args(args: &[String], name: &str) -> Vec<String> {
    args.iter().enumerate().filter(|(_, a)| *a == name).filter_map(|(i, _)| args.get(i + 1).cloned()).take(MAX_DOMAINS).collect()
}

fn fail(msg: &str) -> ! {
    eprintln!("aiscraft-preview: {msg}");
    std::process::exit(2)
}

/// A declared domain: `https://host[:port]`, `https://*.host` (and `wss://` for connections).
fn domain_ok(d: &str, connect: bool) -> bool {
    let rest = match d.split_once("://") {
        Some(("https", r)) => r,
        Some(("wss", r)) if connect => r,
        _ => return false,
    };
    let host = rest.strip_prefix("*.").unwrap_or(rest);
    let (host, port) = match host.rsplit_once(':') {
        Some((h, p)) => (h, Some(p)),
        None => (host, None),
    };
    let port_ok = port.is_none_or(|p| !p.is_empty() && p.len() <= 5 && p.chars().all(|c| c.is_ascii_digit()));
    let labels: Vec<&str> = host.split('.').collect();
    port_ok
        && d.len() <= 253
        && labels.len() >= 2
        && labels
            .iter()
            .all(|l| !l.is_empty() && l.len() <= 63 && l.chars().all(|c| c.is_ascii_alphanumeric() || c == '-') && !l.starts_with('-') && !l.ends_with('-'))
}

/// The policy of a view: inline code, and the network only to the declared domains.
fn view_csp(connect: &[String], resources: &[String]) -> String {
    let res = if resources.is_empty() { String::new() } else { format!(" {}", resources.join(" ")) };
    let conn = if connect.is_empty() { "'none'".to_string() } else { connect.join(" ") };
    format!(
        "default-src 'none'; script-src 'unsafe-inline'{res}; style-src 'unsafe-inline'{res}; img-src data: blob:{res}; media-src data: blob:{res}; font-src data:{res}; connect-src {conn}; frame-src 'none'; object-src 'none'; form-action 'none'; base-uri 'none'"
    )
}

/// Lines from the app that steer the window; never delivered to the view. Only the app writes
/// to stdin, so a view cannot send them itself.
const WINDOW_KEY: &str = "aiscraft/window";
/// The window of a view stays within these sizes (logical pixels); the app caps them too.
const MIN_SIZE: (f64, f64) = (240.0, 160.0);
const MAX_SIZE: (f64, f64) = (1600.0, 1200.0);
/// The normal window and the small one on top (`pip`).
const NORMAL_SIZE: (f64, f64) = (1100.0, 800.0);
const PIP_SIZE: (f64, f64) = (420.0, 320.0);

#[derive(Debug, PartialEq)]
enum Steer {
    /// Resize to (width, height), within the caps.
    Size(f64, f64),
    /// `true`: small and on top; `false`: the normal window.
    Pip(bool),
}

#[derive(Debug, PartialEq)]
enum Incoming {
    /// A JavaScript expression to hand to the view.
    Deliver(String),
    Window(Steer),
}

/// A line from the app: a window instruction, or a message for the view as a JavaScript
/// expression (parsed and written again, so nothing but JSON reaches the page).
fn incoming(line: &str) -> Option<Incoming> {
    if line.len() > MAX_MESSAGE {
        return None;
    }
    let v: serde_json::Value = serde_json::from_str(line).ok()?;
    if let Some(w) = v.get(WINDOW_KEY) {
        if let Some(mode) = w.get("mode").and_then(serde_json::Value::as_str) {
            return match mode {
                "pip" => Some(Incoming::Window(Steer::Pip(true))),
                "fullscreen" => Some(Incoming::Window(Steer::Pip(false))),
                _ => None,
            };
        }
        let dim = |k: &str, min: f64, max: f64| w.get(k).and_then(serde_json::Value::as_f64).filter(|x| x.is_finite() && *x > 0.0).map(|x| x.clamp(min, max));
        return Some(Incoming::Window(Steer::Size(dim("width", MIN_SIZE.0, MAX_SIZE.0)?, dim("height", MIN_SIZE.1, MAX_SIZE.1)?)));
    }
    v.is_object().then(|| Incoming::Deliver(format!("window.__aiscraftDeliver({v});")))
}

enum UserEvent {
    ParentGone,
    Deliver(String),
    Window(Steer),
}

fn read_page(file: &str) -> Vec<u8> {
    match std::fs::File::open(file) {
        Ok(f) => {
            let mut buf = Vec::new();
            if f.take(MAX_PAGE + 1).read_to_end(&mut buf).is_err() || buf.len() as u64 > MAX_PAGE {
                fail("Die Seite lässt sich nicht lesen oder ist größer als 4 MB");
            }
            buf
        }
        Err(e) => fail(&format!("{file}: {e}")),
    }
}

fn main() {
    let args: Vec<String> = std::env::args().collect();
    // `--h5p <content dir> --player <player dir>`: H5P content in the H5P player (#62).
    let h5p_dirs = arg(&args, "--h5p").zip(arg(&args, "--player")).map(|(c, p)| (PathBuf::from(c), PathBuf::from(p)));
    if let Some((content, player)) = &h5p_dirs
        && (!content.join("h5p.json").is_file() || !player.join("player").join("main.bundle.js").is_file())
    {
        fail("H5P-Inhalt oder H5P-Player fehlt");
    }
    let file = match (&h5p_dirs, arg(&args, "--file")) {
        (Some(_), _) => String::new(),
        (None, Some(f)) => f,
        (None, None) => fail("--file <page.html> fehlt"),
    };
    let title = arg(&args, "--title").unwrap_or_else(|| if h5p_dirs.is_some() { "H5P".into() } else { "Lern-App".into() });
    let bridge = h5p_dirs.is_none() && args.iter().any(|a| a == "--bridge");
    let connect = all_args(&args, "--connect");
    let resources = all_args(&args, "--resource");
    if !connect.iter().all(|d| domain_ok(d, true)) || !resources.iter().all(|d| domain_ok(d, false)) {
        fail("unzulässige Netzfreigabe");
    }
    let csp: String = if bridge { view_csp(&connect, &resources) } else { CSP.to_string() };
    let page = if h5p_dirs.is_some() { Vec::new() } else { read_page(&file) };

    let event_loop = EventLoopBuilder::<UserEvent>::with_user_event().build();
    if bridge {
        let proxy = event_loop.create_proxy();
        std::thread::spawn(move || {
            let stdin = std::io::stdin();
            for line in stdin.lock().lines() {
                let Ok(line) = line else { break };
                let ev = match incoming(&line) {
                    Some(Incoming::Deliver(js)) => UserEvent::Deliver(js),
                    Some(Incoming::Window(s)) => UserEvent::Window(s),
                    None => continue,
                };
                let _ = proxy.send_event(ev);
            }
            let _ = proxy.send_event(UserEvent::ParentGone);
        });
    } else if args.iter().any(|a| a == "--watch-stdin") {
        let proxy = event_loop.create_proxy();
        std::thread::spawn(move || {
            let mut sink = [0u8; 256];
            let mut stdin = std::io::stdin();
            while matches!(stdin.read(&mut sink), Ok(n) if n > 0) {}
            let _ = proxy.send_event(UserEvent::ParentGone);
        });
    }
    let window = match WindowBuilder::new()
        .with_title(format!("{} – Vorschau (AIS.ChatCraft)", title.chars().take(80).collect::<String>()))
        .with_inner_size(LogicalSize::new(1100.0, 800.0))
        .build(&event_loop)
    {
        Ok(w) => w,
        Err(e) => fail(&format!("Fenster: {e}")),
    };

    let builder = WebViewBuilder::new()
        .with_new_window_req_handler(|_, _| NewWindowResponse::Deny)
        .with_download_started_handler(|_, _| false)
        .with_devtools(cfg!(debug_assertions));
    let mut builder = if let Some((content, player)) = h5p_dirs {
        let origin = h5p::origin();
        let policy = h5p::csp(origin);
        let page_title = title.clone();
        builder
            .with_custom_protocol(h5p::SCHEME.into(), move |_id, request| {
                let (status, body, mime) = h5p::respond(request.uri().path(), &content, &player, &page_title);
                Response::builder()
                    .status(status)
                    .header(CONTENT_TYPE, mime)
                    .header("Content-Security-Policy", policy.as_str())
                    .header("X-Content-Type-Options", "nosniff")
                    .body(Cow::Owned(body))
                    .unwrap_or_else(|_| Response::new(Cow::Borrowed(b"".as_slice())))
            })
            .with_url(format!("{origin}/"))
            .with_navigation_handler(move |url| h5p::navigation_ok(&url, origin))
    } else {
        builder
            .with_custom_protocol("aiscraft".into(), move |_id, request| {
                // One page; anything else the page asks for does not exist.
                let path = request.uri().path();
                let (status, body, mime): (u16, Cow<'static, [u8]>, &str) = if path == "/" || path == "/index.html" {
                    (200, Cow::Owned(page.clone()), "text/html; charset=utf-8")
                } else {
                    (404, Cow::Borrowed(b"".as_slice()), "text/plain")
                };
                Response::builder()
                    .status(status)
                    .header(CONTENT_TYPE, mime)
                    .header("Content-Security-Policy", csp.as_str())
                    .header("X-Content-Type-Options", "nosniff")
                    .body(body)
                    .unwrap_or_else(|_| Response::new(Cow::Borrowed(b"".as_slice())))
            })
            .with_url(START)
            .with_navigation_handler(|url| url.starts_with(START))
    };
    if bridge {
        builder = builder.with_initialization_script(BRIDGE_JS).with_ipc_handler(|req| {
            // One message per line; JSON.stringify never writes a raw line break.
            let body = req.body();
            if body.len() <= MAX_MESSAGE && !body.contains('\n') && !body.contains('\r') {
                let mut out = std::io::stdout().lock();
                let _ = writeln!(out, "{body}");
                let _ = out.flush();
            }
        });
    }

    #[cfg(any(target_os = "windows", target_os = "macos"))]
    let webview = builder.build(&window);
    #[cfg(not(any(target_os = "windows", target_os = "macos")))]
    let webview = {
        use tao::platform::unix::WindowExtUnix;
        use wry::WebViewBuilderExtUnix;
        match window.default_vbox() {
            Some(vbox) => builder.build_gtk(vbox),
            None => fail("Fenster ohne GTK-Container"),
        }
    };
    let webview = match webview {
        Ok(w) => w,
        Err(e) => fail(&format!("Webview: {e}")),
    };

    event_loop.run(move |event, _, control_flow| {
        *control_flow = ControlFlow::Wait;
        match event {
            Event::WindowEvent { event: WindowEvent::CloseRequested, .. } | Event::UserEvent(UserEvent::ParentGone) => *control_flow = ControlFlow::Exit,
            Event::UserEvent(UserEvent::Deliver(js)) => {
                let _ = webview.evaluate_script(&js);
            }
            Event::UserEvent(UserEvent::Window(Steer::Size(w, h))) => window.set_inner_size(LogicalSize::new(w, h)),
            Event::UserEvent(UserEvent::Window(Steer::Pip(pip))) => {
                window.set_always_on_top(pip);
                let (w, h) = if pip { PIP_SIZE } else { NORMAL_SIZE };
                window.set_inner_size(LogicalSize::new(w, h));
            }
            _ => {}
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn domains_and_policy() {
        assert!(domain_ok("https://api.example.org", true) && domain_ok("https://*.example.org:8443", false));
        for bad in ["*", "https://*", "http://example.org", "data:", "https://example.org/x", "'self'", "https://a b.org"] {
            assert!(!domain_ok(bad, true), "{bad}");
        }
        let p = view_csp(&[], &[]);
        assert!(p.contains("connect-src 'none'") && p.contains("default-src 'none'") && !p.contains('*'));
    }

    #[test]
    fn only_json_objects_reach_the_view() {
        let Some(Incoming::Deliver(js)) = incoming(r#"{"jsonrpc":"2.0","id":1,"result":{}}"#) else { panic!("a message for the view") };
        assert!(js.starts_with("window.__aiscraftDeliver({") && js.ends_with("});") && js.contains(r#""jsonrpc":"2.0""#), "{js}");
        assert!(incoming("alert(1)").is_none());
        assert!(incoming("[1]").is_none());
        assert!(matches!(incoming(r#"{"a":"</script>"}"#), Some(Incoming::Deliver(_))));
    }

    #[test]
    fn window_lines_steer_the_window_within_caps_and_never_reach_the_view() {
        assert_eq!(incoming(r#"{"aiscraft/window":{"width":500,"height":400}}"#), Some(Incoming::Window(Steer::Size(500.0, 400.0))));
        assert_eq!(incoming(r#"{"aiscraft/window":{"width":99999,"height":1}}"#), Some(Incoming::Window(Steer::Size(MAX_SIZE.0, MIN_SIZE.1))));
        assert_eq!(incoming(r#"{"aiscraft/window":{"mode":"pip"}}"#), Some(Incoming::Window(Steer::Pip(true))));
        assert_eq!(incoming(r#"{"aiscraft/window":{"mode":"fullscreen"}}"#), Some(Incoming::Window(Steer::Pip(false))));
        for bad in [
            r#"{"aiscraft/window":{"mode":"inline"}}"#,
            r#"{"aiscraft/window":{"width":"x","height":2}}"#,
            r#"{"aiscraft/window":{"width":-5,"height":200}}"#,
            r#"{"aiscraft/window":1}"#,
        ] {
            assert!(incoming(bad).is_none(), "{bad}");
        }
    }
}
