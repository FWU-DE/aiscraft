//! `aiscraft-preview --file <page.html> [--title <t>] [--watch-stdin]`
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

use std::borrow::Cow;
use std::io::Read as _;

use tao::dpi::LogicalSize;
use tao::event::{Event, WindowEvent};
use tao::event_loop::{ControlFlow, EventLoopBuilder};
use tao::window::WindowBuilder;
use wry::http::{Response, header::CONTENT_TYPE};
use wry::{NewWindowResponse, WebViewBuilder};

/// The same policy AIS.ChatCraft writes into the page (`engine/src/learn`), sent as a header too.
const CSP: &str = "default-src 'none'; script-src 'unsafe-inline'; style-src 'unsafe-inline'; img-src data: blob:; media-src data: blob:; font-src data:; form-action 'none'; base-uri 'none'";
const MAX_PAGE: u64 = 4 * 1024 * 1024;

/// Where the custom scheme lives: WebView2 maps custom schemes to http(s)://<name>.localhost.
#[cfg(target_os = "windows")]
const START: &str = "http://aiscraft.localhost/";
#[cfg(not(target_os = "windows"))]
const START: &str = "aiscraft://localhost/";

fn arg(args: &[String], name: &str) -> Option<String> {
    args.iter().position(|a| a == name).and_then(|i| args.get(i + 1).cloned())
}

fn fail(msg: &str) -> ! {
    eprintln!("aiscraft-preview: {msg}");
    std::process::exit(2)
}

enum UserEvent {
    ParentGone,
}

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let Some(file) = arg(&args, "--file") else { fail("--file <page.html> fehlt") };
    let title = arg(&args, "--title").unwrap_or_else(|| "Lern-App".into());
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
    if args.iter().any(|a| a == "--watch-stdin") {
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
                .header("Content-Security-Policy", CSP)
                .header("X-Content-Type-Options", "nosniff")
                .body(body)
                .unwrap_or_else(|_| Response::new(Cow::Borrowed(b"".as_slice())))
        })
        .with_url(START)
        .with_navigation_handler(|url| url.starts_with(START))
        .with_new_window_req_handler(|_, _| NewWindowResponse::Deny)
        .with_download_started_handler(|_, _| false)
        .with_devtools(cfg!(debug_assertions));

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
    let _webview = match webview {
        Ok(w) => w,
        Err(e) => fail(&format!("Webview: {e}")),
    };

    event_loop.run(move |event, _, control_flow| {
        *control_flow = ControlFlow::Wait;
        match event {
            Event::WindowEvent { event: WindowEvent::CloseRequested, .. } | Event::UserEvent(UserEvent::ParentGone) => *control_flow = ControlFlow::Exit,
            _ => {}
        }
    });
}
