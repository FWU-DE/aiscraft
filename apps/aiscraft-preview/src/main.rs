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
//! here: `https://host[:port]` or `https://*.host`, never `*`.

use std::borrow::Cow;
use std::io::{BufRead as _, Read as _, Write as _};

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

/// A line from the app, as a JavaScript expression to hand to the view (parsed and written
/// again, so nothing but JSON reaches the page).
fn delivery(line: &str) -> Option<String> {
    if line.len() > MAX_MESSAGE {
        return None;
    }
    let v: serde_json::Value = serde_json::from_str(line).ok()?;
    v.is_object().then(|| format!("window.__aiscraftDeliver({v});"))
}

enum UserEvent {
    ParentGone,
    Deliver(String),
}

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let Some(file) = arg(&args, "--file") else { fail("--file <page.html> fehlt") };
    let title = arg(&args, "--title").unwrap_or_else(|| "Lern-App".into());
    let bridge = args.iter().any(|a| a == "--bridge");
    let connect = all_args(&args, "--connect");
    let resources = all_args(&args, "--resource");
    if !connect.iter().all(|d| domain_ok(d, true)) || !resources.iter().all(|d| domain_ok(d, false)) {
        fail("unzulässige Netzfreigabe");
    }
    let csp: String = if bridge { view_csp(&connect, &resources) } else { CSP.to_string() };
    let page = match std::fs::File::open(&file) {
        Ok(f) => {
            let mut buf = Vec::new();
            if f.take(MAX_PAGE + 1).read_to_end(&mut buf).is_err() || buf.len() as u64 > MAX_PAGE {
                fail("Die Seite lässt sich nicht lesen oder ist größer als 4 MB");
            }
            buf
        }
        Err(e) => fail(&format!("{file}: {e}")),
    };

    let event_loop = EventLoopBuilder::<UserEvent>::with_user_event().build();
    if bridge {
        let proxy = event_loop.create_proxy();
        std::thread::spawn(move || {
            let stdin = std::io::stdin();
            for line in stdin.lock().lines() {
                let Ok(line) = line else { break };
                if let Some(js) = delivery(&line) {
                    let _ = proxy.send_event(UserEvent::Deliver(js));
                }
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

    let mut builder = WebViewBuilder::new()
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
        .with_new_window_req_handler(|_, _| NewWindowResponse::Deny)
        .with_download_started_handler(|_, _| false)
        .with_devtools(cfg!(debug_assertions));
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
        let js = delivery(r#"{"jsonrpc":"2.0","id":1,"result":{}}"#).unwrap();
        assert!(js.starts_with("window.__aiscraftDeliver({") && js.ends_with("});") && js.contains(r#""jsonrpc":"2.0""#), "{js}");
        assert!(delivery("alert(1)").is_none());
        assert!(delivery("[1]").is_none());
        assert!(delivery(r#"{"a":"</script>"}"#).is_some());
    }
}
