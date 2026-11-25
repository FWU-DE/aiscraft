//! `aiscraft-preview --file <page.html> [--title <t>] [--watch-stdin]`
//! `aiscraft-preview --file <view.html> --bridge [--connect <https://host>]… [--resource <https://host>]…`
//!
//! Shows one learning app (a local HTML file AIS.ChatCraft wrote into its data folder) in a
//! single webview window, in its own process, so the main app never links a webview.
//!
//! Locked down: the page is served from a private `aiscraft://` scheme (not `file://`, so it
//! cannot read other local files) with a Content-Security-Policy header that allows inline
//! code only and no network at all; every navigation away from the page and every new window
//! is refused; downloads are refused; developer tools only in debug builds. Each launch has its
//! own scheme name (`aiscraft-<label>`, `aiscraft-view-<label>`, `aiscraft-h5p-<label>`), hence
//! its own origin, in a private profile that keeps nothing (incognito); every permission request
//! (microphone, camera, location, clipboard, notifications …) is refused and WebRTC is gone, so
//! a grant or stored data of one page never reaches another. With
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

/// Takes WebRTC away from the page: a peer connection would reach STUN servers past the policy
/// (`connect-src` does not cover it) and tell them the device's addresses.
const NO_WEBRTC_JS: &str = r#"(() => {
  for (const k of ['RTCPeerConnection', 'webkitRTCPeerConnection', 'RTCDataChannel', 'RTCIceCandidate', 'RTCSessionDescription', 'RTCRtpSender', 'RTCRtpReceiver', 'RTCRtpTransceiver', 'RTCDtlsTransport', 'RTCIceTransport', 'RTCSctpTransport']) {
    try { Object.defineProperty(window, k, { value: undefined, writable: false, configurable: false }); } catch (e) {}
  }
})();"#;

/// WebView2's start arguments: wry's defaults, and WebRTC only through a proxy (there is none),
/// so no UDP to STUN servers even if a page found a way around [`NO_WEBRTC_JS`].
#[cfg(target_os = "windows")]
const BROWSER_ARGS: &str =
    "--disable-features=msWebOOUI,msPdfOOUI,msSmartScreenProtection --webrtc-ip-handling-policy=disable_non_proxied_udp --force-webrtc-ip-handling-policy";

/// The command line, read positionally: a flag that takes a value always takes the next
/// argument, whatever it looks like, so a title like `--bridge` stays a title.
#[derive(Debug, Default, PartialEq)]
struct Args {
    file: Option<String>,
    title: Option<String>,
    h5p: Option<String>,
    player: Option<String>,
    bridge: bool,
    watch_stdin: bool,
    connect: Vec<String>,
    resources: Vec<String>,
}

fn parse_args(args: &[String]) -> Result<Args, String> {
    let mut out = Args::default();
    let mut it = args.iter().skip(1);
    while let Some(a) = it.next() {
        let mut value = || it.next().cloned().ok_or_else(|| format!("{a} ohne Wert"));
        let once = |slot: &mut Option<String>, v: String| if slot.replace(v).is_some() { Err(format!("{a} doppelt")) } else { Ok(()) };
        match a.as_str() {
            "--file" => once(&mut out.file, value()?)?,
            "--title" => once(&mut out.title, value()?)?,
            "--h5p" => once(&mut out.h5p, value()?)?,
            "--player" => once(&mut out.player, value()?)?,
            "--connect" => out.connect.push(value()?),
            "--resource" => out.resources.push(value()?),
            "--bridge" => out.bridge = true,
            "--watch-stdin" => out.watch_stdin = true,
            other => return Err(format!("unbekanntes Argument: {}", other.chars().filter(|c| !c.is_control()).take(40).collect::<String>())),
        }
    }
    if out.connect.len() > MAX_DOMAINS || out.resources.len() > MAX_DOMAINS {
        return Err("zu viele Netzfreigaben".into());
    }
    Ok(out)
}

fn fail(msg: &str) -> ! {
    eprintln!("aiscraft-preview: {msg}");
    std::process::exit(2)
}

/// A label for this launch, new each time: the page's origin carries it, so no two windows
/// (learning apps, H5P, connector views) ever share an origin, its storage or its grants.
fn launch_label() -> String {
    use std::hash::{BuildHasher as _, Hasher as _};
    let mut h = std::collections::hash_map::RandomState::new().build_hasher();
    h.write_u32(std::process::id());
    h.write_u128(std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_nanos()).unwrap_or_default());
    format!("{:016x}", h.finish())
}

/// The scheme of this launch: `<base>-<label>` (lower case letters, digits and dashes, so it is
/// also a valid host label for WebView2).
fn launch_scheme(base: &str, label: &str) -> String {
    format!("{base}-{label}")
}

/// Where a custom scheme lives: WebView2 maps custom schemes to `http://<name>.localhost`.
fn origin_of(scheme: &str) -> String {
    if cfg!(target_os = "windows") { format!("http://{scheme}.localhost") } else { format!("{scheme}://localhost") }
}

/// Navigation stays on the page's own origin (and its path), never a look-alike host.
fn same_origin(url: &str, origin: &str) -> bool {
    url.strip_prefix(origin).is_some_and(|rest| rest.is_empty() || rest.starts_with('/'))
}

/// Every permission a page asks for (microphone, camera, location, clipboard, notifications …)
/// is refused, without asking the person: nothing in a preview needs one.
fn permission(_kind: wry::PermissionKind) -> wry::PermissionResponse {
    wry::PermissionResponse::Deny
}

/// A declared domain: the engine's rules (`aiscraft_core::net`), checked here again.
fn domain_ok(d: &str, connect: bool) -> bool {
    aiscraft_core::net::view_domain_ok(d, connect)
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
    let raw: Vec<String> = std::env::args().collect();
    let args = parse_args(&raw).unwrap_or_else(|e| fail(&e));
    // `--h5p <content dir> --player <player dir>`: H5P content in the H5P player (#62).
    let h5p_dirs = args.h5p.clone().zip(args.player.clone()).map(|(c, p)| (PathBuf::from(c), PathBuf::from(p)));
    if let Some((content, player)) = &h5p_dirs
        && (!content.join("h5p.json").is_file() || !player.join("player").join("main.bundle.js").is_file())
    {
        fail("H5P-Inhalt oder H5P-Player fehlt");
    }
    let file = match (&h5p_dirs, &args.file) {
        (Some(_), _) => String::new(),
        (None, Some(f)) => f.clone(),
        (None, None) => fail("--file <page.html> fehlt"),
    };
    let title = args.title.clone().unwrap_or_else(|| if h5p_dirs.is_some() { "H5P".into() } else { "Lern-App".into() });
    let bridge = h5p_dirs.is_none() && args.bridge;
    if !args.connect.iter().all(|d| domain_ok(d, true)) || !args.resources.iter().all(|d| domain_ok(d, false)) {
        fail("unzulässige Netzfreigabe");
    }
    let csp: String = if bridge { view_csp(&args.connect, &args.resources) } else { CSP.to_string() };
    let page = if h5p_dirs.is_some() { Vec::new() } else { read_page(&file) };
    // This launch's own origin: storage and grants of one window never reach another.
    let label = launch_label();

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
    } else if args.watch_stdin {
        let proxy = event_loop.create_proxy();
        std::thread::spawn(move || {
            let mut sink = [0u8; 256];
            let mut stdin = std::io::stdin();
            while matches!(stdin.read(&mut sink), Ok(n) if n > 0) {}
            let _ = proxy.send_event(UserEvent::ParentGone);
        });
    }
    let window = match WindowBuilder::new()
        .with_title(format!("{} – Vorschau (AIS.ChatCraft)", title.chars().filter(|c| !c.is_control()).take(80).collect::<String>()))
        .with_inner_size(LogicalSize::new(1100.0, 800.0))
        .build(&event_loop)
    {
        Ok(w) => w,
        Err(e) => fail(&format!("Fenster: {e}")),
    };

    // No profile that outlives the window (no storage, no remembered grants), no permission
    // ever, no clipboard access, no WebRTC.
    let builder = WebViewBuilder::new()
        .with_incognito(true)
        .with_permission_handler(permission)
        .with_clipboard(false)
        .with_initialization_script(NO_WEBRTC_JS)
        .with_new_window_req_handler(|_, _| NewWindowResponse::Deny)
        .with_download_started_handler(|_, _| false)
        .with_devtools(cfg!(debug_assertions));
    #[cfg(target_os = "windows")]
    let builder = {
        use wry::WebViewBuilderExtWindows as _;
        builder.with_additional_browser_args(BROWSER_ARGS)
    };
    let mut builder = if let Some((content, player)) = h5p_dirs {
        let scheme = launch_scheme(h5p::SCHEME, &label);
        let origin = origin_of(&scheme);
        let policy = h5p::csp(&origin);
        let page_title = title.clone();
        let start = format!("{origin}/");
        builder
            .with_custom_protocol(scheme, move |_id, request| {
                let (status, body, mime) = h5p::respond(request.uri().path(), &content, &player, &page_title);
                Response::builder()
                    .status(status)
                    .header(CONTENT_TYPE, mime)
                    .header("Content-Security-Policy", policy.as_str())
                    .header("X-Content-Type-Options", "nosniff")
                    .body(Cow::Owned(body))
                    .unwrap_or_else(|_| Response::new(Cow::Borrowed(b"".as_slice())))
            })
            .with_url(start)
            .with_navigation_handler(move |url| h5p::navigation_ok(&url, &origin))
    } else {
        // Learning apps and connector views get different names, on top of the launch label.
        let scheme = launch_scheme(if bridge { "aiscraft-view" } else { "aiscraft" }, &label);
        let origin = origin_of(&scheme);
        let start = format!("{origin}/");
        builder
            .with_custom_protocol(scheme, move |_id, request| {
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
            .with_url(start)
            .with_navigation_handler(move |url| same_origin(&url, &origin))
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

    fn args(a: &[&str]) -> Result<Args, String> {
        let v: Vec<String> = std::iter::once("aiscraft-preview").chain(a.iter().copied()).map(String::from).collect();
        parse_args(&v)
    }

    #[test]
    fn a_flag_takes_its_value_whatever_it_looks_like() {
        // A model-made title that looks like flags stays the title.
        let a = args(&["--file", "a.html", "--title", "--bridge", "--watch-stdin"]).unwrap();
        assert_eq!(a.title.as_deref(), Some("--bridge"));
        assert!(!a.bridge && a.watch_stdin && a.connect.is_empty());
        let a = args(&["--file", "a.html", "--title", "--connect", "--watch-stdin"]).unwrap();
        assert_eq!(a.title.as_deref(), Some("--connect"));
        assert!(a.connect.is_empty() && a.watch_stdin);
        let a = args(&["--file", "v.html", "--title", "Kalender", "--bridge", "--connect", "https://api.example.org", "--resource", "https://cdn.example.org"])
            .unwrap();
        assert!(a.bridge && a.connect == ["https://api.example.org"] && a.resources == ["https://cdn.example.org"]);
        for bad in [&["--file"][..], &["--file", "a", "--file", "b"], &["--title", "a", "--title", "b"], &["--file", "a", "--unbekannt"], &["a.html"]] {
            assert!(args(bad).is_err(), "{bad:?}");
        }
        let many: Vec<&str> = std::iter::repeat_n(["--connect", "https://a.example.org"], MAX_DOMAINS + 1).flatten().collect();
        assert!(args(&many).is_err());
    }

    #[test]
    fn every_launch_has_its_own_origin_and_no_permission() {
        let (a, b) = (launch_label(), launch_label());
        assert_ne!(a, b);
        assert!(a.len() == 16 && a.chars().all(|c| c.is_ascii_hexdigit() && !c.is_ascii_uppercase()));
        let app = origin_of(&launch_scheme("aiscraft", &a));
        let view = origin_of(&launch_scheme("aiscraft-view", &a));
        let later = origin_of(&launch_scheme("aiscraft-view", &b));
        assert!(app != view && view != later && app.contains(&a));
        // Navigation stays on the origin; a look-alike host is somewhere else.
        assert!(same_origin(&format!("{view}/"), &view) && same_origin(&view, &view));
        assert!(!same_origin(&format!("{view}.evil.org/"), &view) && !same_origin(&format!("{later}/"), &view) && !same_origin("https://example.org/", &view));
        use wry::{PermissionKind as K, PermissionResponse as R};
        for k in [K::Microphone, K::Camera, K::Geolocation, K::Notifications, K::ClipboardRead, K::DisplayCapture, K::Midi, K::Sensors, K::Other] {
            assert_eq!(permission(k), R::Deny, "{k:?}");
        }
        assert!(NO_WEBRTC_JS.contains("'RTCPeerConnection'") && NO_WEBRTC_JS.contains("configurable: false"));
    }

    #[test]
    fn domains_and_policy() {
        assert!(domain_ok("https://api.example.org", true) && domain_ok("https://*.example.org:8443", false));
        for bad in [
            "*",
            "https://*",
            "http://example.org",
            "data:",
            "https://example.org/x",
            "'self'",
            "https://a b.org",
            "https://127.0.0.1",
            "https://[::1]",
            "https://localhost",
            "https://x.localhost",
            "https://nas.local",
            "https://*.github.io",
            "https://*.co.uk",
        ] {
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
