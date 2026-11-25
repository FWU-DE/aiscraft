//! `--h5p <content dir> --player <player dir>`: H5P content in the real H5P player (#62).
//!
//! The player (h5p-standalone with the H5P core and the content-type libraries) is an optional
//! download AIS.ChatCraft unpacked into its data folder; the content is what AIS.ChatCraft wrote
//! for one H5P artifact. Both are served from the private `aiscraft-h5p` protocol:
//!
//! - `/` the start page (generated here), which loads the player
//! - `/player/…` h5p-standalone (`main.bundle.js`, `frame.bundle.js`, styles, fonts)
//! - `/libraries/<Name>-<major>.<minor>/…` a library: the player's first, then one the
//!   package brought, else the player's newest minor version of the same major version
//! - `/content/…` `h5p.json`, `content/content.json` and the content's pictures and media
//!
//! Nothing else exists. Paths are checked segment by segment (no `..`, no empty or hidden
//! segments, no backslashes, colons or control characters), files must have a known type and
//! must not be links. The policy is the learning apps' `default-src 'none'`, loosened only for
//! this protocol: scripts, styles, pictures, media, fonts, requests (`connect-src`) and the
//! player's own frame come from it; there is still no network at all.

use std::path::{Path, PathBuf};

/// The start of the protocol's name; each launch adds its own label (`aiscraft-h5p-<label>`), so
/// no two windows share an origin (see `launch_scheme` in `main.rs`).
pub const SCHEME: &str = "aiscraft-h5p";
/// Largest file served (videos in content).
const MAX_FILE: u64 = 200 * 1024 * 1024;

/// The policy: nothing but this protocol (and inline code, which the player writes into its
/// frame). No network: no `https:`, no `*`.
pub fn csp(origin: &str) -> String {
    format!(
        "default-src 'none'; script-src 'unsafe-inline' {origin}; style-src 'unsafe-inline' {origin}; img-src {origin} data: blob:; media-src {origin} data: blob:; font-src {origin} data:; connect-src {origin}; frame-src {origin} about:; child-src {origin} about:; object-src 'none'; form-action 'none'; base-uri 'none'"
    )
}

/// Navigation: the start page, and the player's own frame (`about:blank`, `about:srcdoc`).
pub fn navigation_ok(url: &str, origin: &str) -> bool {
    url == "about:blank" || url == "about:srcdoc" || url.strip_prefix(origin).is_some_and(|rest| rest.is_empty() || rest.starts_with('/'))
}

/// What a request path asks for.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Route {
    Index,
    Player(Vec<String>),
    /// Folder (`H5P.MultiChoice-1.16`) and the path inside.
    Library(String, Vec<String>),
    Content(Vec<String>),
}

/// `%XX` decoded; `None` for a broken escape or text that is not UTF-8.
fn percent_decode(s: &str) -> Option<String> {
    let b = s.as_bytes();
    let mut out = Vec::with_capacity(b.len());
    let mut i = 0;
    while i < b.len() {
        match b.get(i) {
            Some(b'%') => {
                let hex = s.get(i + 1..i + 3)?;
                out.push(u8::from_str_radix(hex, 16).ok()?);
                i += 3;
            }
            Some(c) => {
                out.push(*c);
                i += 1;
            }
            None => break,
        }
    }
    String::from_utf8(out).ok()
}

/// A path segment that cannot leave its folder or name something odd.
fn segment_ok(s: &str) -> bool {
    !s.is_empty()
        && s.len() <= 255
        && !s.starts_with('.')
        && !s.chars().any(|c| c.is_control() || matches!(c, '/' | '\\' | ':' | '*' | '?' | '"' | '<' | '>' | '|'))
}

/// `H5P.MultiChoice-1.16` → (`H5P.MultiChoice`, 1, 16).
pub fn library_folder(name: &str) -> Option<(&str, u32, u32)> {
    let (machine, version) = name.rsplit_once('-')?;
    let (major, minor) = version.split_once('.')?;
    let ok = !machine.is_empty() && machine.len() <= 100 && machine.chars().all(|c| c.is_ascii_alphanumeric() || c == '.' || c == '_' || c == '-');
    ok.then_some(())?;
    Some((machine, major.parse().ok()?, minor.parse().ok()?))
}

/// The route of a request path (query and fragment already gone), or `None`.
pub fn route(path: &str) -> Option<Route> {
    if path == "/" || path == "/index.html" {
        return Some(Route::Index);
    }
    let rest = path.strip_prefix('/')?;
    if rest.len() > 1024 {
        return None;
    }
    let mut segs = Vec::new();
    for raw in rest.split('/') {
        let s = percent_decode(raw)?;
        if !segment_ok(&s) {
            return None;
        }
        segs.push(s);
    }
    let (head, tail) = segs.split_first()?;
    let tail = tail.to_vec();
    match head.as_str() {
        "player" if !tail.is_empty() => Some(Route::Player(tail)),
        "content" if !tail.is_empty() => Some(Route::Content(tail)),
        "libraries" => {
            let (folder, inner) = tail.split_first()?;
            library_folder(folder)?;
            (!inner.is_empty()).then(|| Route::Library(folder.clone(), inner.to_vec()))
        }
        _ => None,
    }
}

/// The newest folder of `available` with the same machine name and major version as
/// `requested`, when `requested` itself is not there (a package made for 1.20 plays with 1.21).
pub fn fallback_folder(requested: &str, available: &[String]) -> Option<String> {
    let (machine, major, _) = library_folder(requested)?;
    available
        .iter()
        .filter_map(|f| library_folder(f).filter(|(m, ma, _)| *m == machine && *ma == major).map(|(_, _, mi)| (mi, f)))
        .max_by_key(|(mi, _)| *mi)
        .map(|(_, f)| f.clone())
}

fn folders(dir: &Path) -> Vec<String> {
    std::fs::read_dir(dir).into_iter().flatten().flatten().take(2000).filter_map(|e| e.file_name().to_str().map(str::to_string)).collect()
}

/// The file a route names, when it exists and is a regular file (never a link).
pub fn resolve(route: &Route, content: &Path, player: &Path) -> Option<PathBuf> {
    let join = |base: PathBuf, segs: &[String]| segs.iter().fold(base, |p, s| p.join(s));
    let candidates: Vec<PathBuf> = match route {
        Route::Index => return None,
        Route::Player(segs) => vec![join(player.join("player"), segs)],
        Route::Content(segs) => vec![join(content.to_path_buf(), segs)],
        Route::Library(folder, segs) => {
            let mut c = vec![join(player.join("libraries").join(folder), segs), join(content.join("libraries").join(folder), segs)];
            if let Some(f) = fallback_folder(folder, &folders(&player.join("libraries"))) {
                c.push(join(player.join("libraries").join(f), segs));
            }
            c
        }
    };
    candidates.into_iter().find(|p| std::fs::symlink_metadata(p).is_ok_and(|m| m.file_type().is_file() && m.len() <= MAX_FILE))
}

/// Content type by extension; unknown types are not served.
pub fn mime(path: &Path) -> Option<&'static str> {
    let ext = path.extension()?.to_str()?.to_ascii_lowercase();
    Some(match ext.as_str() {
        "js" => "text/javascript; charset=utf-8",
        "css" => "text/css; charset=utf-8",
        "json" => "application/json; charset=utf-8",
        "html" => "text/html; charset=utf-8",
        "txt" | "md" => "text/plain; charset=utf-8",
        "png" => "image/png",
        "jpg" | "jpeg" => "image/jpeg",
        "gif" => "image/gif",
        "svg" => "image/svg+xml",
        "webp" => "image/webp",
        "bmp" => "image/bmp",
        "woff" => "font/woff",
        "woff2" => "font/woff2",
        "ttf" => "font/ttf",
        "otf" => "font/otf",
        "eot" => "application/vnd.ms-fontobject",
        "mp3" => "audio/mpeg",
        "m4a" => "audio/mp4",
        "wav" => "audio/wav",
        "ogg" | "oga" => "audio/ogg",
        "mp4" => "video/mp4",
        "webm" => "video/webm",
        "vtt" | "webvtt" => "text/vtt",
        "xml" => "application/xml",
        _ => return None,
    })
}

fn escape(s: &str) -> String {
    s.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;").replace('"', "&quot;")
}

/// The start page: loads h5p-standalone and shows the content, or says why not.
pub fn index_page(title: &str) -> String {
    let title = escape(&title.chars().take(80).collect::<String>());
    format!(
        r#"<!doctype html>
<html lang="de"><head><meta charset="utf-8"><title>{title}</title>
<style>body{{margin:0;padding:16px;background:#fff;font-family:system-ui,sans-serif}}#fehler{{color:#a40000}}</style>
<script src="/player/main.bundle.js"></script></head>
<body><div id="h5p-container"></div><p id="fehler"></p>
<script>
(function () {{
  var fail = function (e) {{ document.getElementById('fehler').textContent = 'Der H5P-Player konnte den Inhalt nicht zeigen: ' + e; }};
  try {{
    new H5PStandalone.H5P(document.getElementById('h5p-container'), {{
      h5pJsonPath: '/content', contentJsonPath: '/content/content', librariesPath: '/libraries',
      frameJs: '/player/frame.bundle.js', frameCss: '/player/styles/h5p.css',
      frame: false, copyright: false, export: false, embed: false, icon: false, fullScreen: false, saveFreq: false
    }}).catch(fail);
  }} catch (e) {{ fail(e); }}
}})();
</script></body></html>"#
    )
}

/// The answer to one request: status, body, content type.
pub fn respond(path: &str, content: &Path, player: &Path, title: &str) -> (u16, Vec<u8>, &'static str) {
    let not_found = (404, Vec::new(), "text/plain");
    let Some(r) = route(path) else { return not_found };
    if r == Route::Index {
        return (200, index_page(title).into_bytes(), "text/html; charset=utf-8");
    }
    let Some(file) = resolve(&r, content, player) else { return not_found };
    let Some(m) = mime(&file) else { return not_found };
    match std::fs::read(&file) {
        Ok(b) => (200, b, m),
        Err(_) => not_found,
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)]
    use super::*;

    #[test]
    fn paths_stay_inside_their_folders() {
        assert_eq!(route("/"), Some(Route::Index));
        assert_eq!(route("/player/main.bundle.js"), Some(Route::Player(vec!["main.bundle.js".into()])));
        assert_eq!(route("/content/content.json"), Some(Route::Content(vec!["content.json".into()])));
        assert_eq!(route("/content/images/igel%20klein.png"), Some(Route::Content(vec!["images".into(), "igel klein.png".into()])));
        assert_eq!(
            route("/libraries/H5P.MultiChoice-1.16/js/multichoice.js"),
            Some(Route::Library("H5P.MultiChoice-1.16".into(), vec!["js".into(), "multichoice.js".into()]))
        );
        for bad in [
            "/../secret",
            "/content/../../etc/passwd",
            "/content/%2e%2e/x.json",
            "/content/a%2fb.json",
            "/content/a%5cb.json",
            "/content//x.json",
            "/content/.hidden",
            "/content/C:x.json",
            "/content/%00.json",
            "/content/%zz",
            "/libraries/evil/library.json",
            "/libraries/../player/main.bundle.js",
            "/libraries/H5P.X-1.2",
            "/player",
            "/other/x.js",
            "x.js",
            "",
        ] {
            assert_eq!(route(bad), None, "{bad}");
        }
    }

    #[test]
    fn library_versions_fall_back_to_the_newest_minor() {
        let have = vec!["H5P.QuestionSet-1.21".to_string(), "H5P.QuestionSet-1.19".into(), "H5P.QuestionSet-2.0".into(), "FontAwesome-4.5".into()];
        assert_eq!(fallback_folder("H5P.QuestionSet-1.20", &have), Some("H5P.QuestionSet-1.21".into()));
        assert_eq!(fallback_folder("H5P.QuestionSet-3.0", &have), None);
        assert_eq!(fallback_folder("H5P.Blanks-1.14", &have), None);
        assert_eq!(library_folder("H5P.Blanks-1.14"), Some(("H5P.Blanks", 1, 14)));
        assert_eq!(library_folder("H5P.Blanks"), None);
        assert_eq!(library_folder("x y-1.2"), None);
    }

    #[test]
    fn policy_allows_the_protocol_and_no_network() {
        let o = "aiscraft-h5p://localhost";
        let p = csp(o);
        assert!(p.starts_with("default-src 'none'"));
        for d in ["script-src", "style-src", "img-src", "font-src", "connect-src", "media-src"] {
            let part = p.split(';').map(str::trim).find(|x| x.starts_with(d)).unwrap();
            assert!(part.contains(o), "{part}");
        }
        assert!(!p.contains("https:") && !p.contains('*') && !p.contains("unsafe-eval"), "{p}");
        assert!(p.contains("form-action 'none'") && p.contains("base-uri 'none'") && p.contains("object-src 'none'"));
        assert!(navigation_ok("aiscraft-h5p://localhost/", o) && navigation_ok("about:blank", o));
        assert!(!navigation_ok("https://h5p.org/", o) && !navigation_ok("aiscraft-h5p://localhost.evil.org/", o) && !navigation_ok("file:///etc/passwd", o));
    }

    #[test]
    fn files_are_served_from_their_folders_only() {
        let root = std::env::temp_dir().join(format!("aiscraft-preview-h5p-{}", std::process::id()));
        let (content, player) = (root.join("content"), root.join("player-dir"));
        std::fs::create_dir_all(content.join("content")).unwrap();
        std::fs::create_dir_all(player.join("player/styles")).unwrap();
        std::fs::create_dir_all(player.join("libraries/H5P.QuestionSet-1.21")).unwrap();
        std::fs::create_dir_all(content.join("libraries/H5P.Own-1.0")).unwrap();
        std::fs::write(content.join("h5p.json"), b"{}").unwrap();
        std::fs::write(content.join("content/content.json"), b"{\"a\":1}").unwrap();
        std::fs::write(content.join("content/run.exe"), b"MZ").unwrap();
        std::fs::write(player.join("player/main.bundle.js"), b"/* js */").unwrap();
        std::fs::write(player.join("libraries/H5P.QuestionSet-1.21/library.json"), b"{\"v\":21}").unwrap();
        std::fs::write(content.join("libraries/H5P.Own-1.0/library.json"), b"{\"own\":1}").unwrap();
        std::fs::write(root.join("secret.json"), b"secret").unwrap();

        let get = |p: &str| respond(p, &content, &player, "Igel <Quiz>");
        let (s, body, m) = get("/");
        assert_eq!((s, m), (200, "text/html; charset=utf-8"));
        let page = String::from_utf8(body).unwrap();
        assert!(page.contains("/player/main.bundle.js") && page.contains("Igel &lt;Quiz&gt;") && !page.contains("<Quiz>"));
        assert!(!page.contains("http"), "the page names no host");
        assert_eq!(get("/content/h5p.json").1, b"{}");
        assert_eq!(get("/content/content/content.json").2, "application/json; charset=utf-8");
        assert_eq!(get("/player/main.bundle.js").2, "text/javascript; charset=utf-8");
        assert_eq!(get("/libraries/H5P.QuestionSet-1.20/library.json").1, b"{\"v\":21}", "newer minor");
        assert_eq!(get("/libraries/H5P.Own-1.0/library.json").1, b"{\"own\":1}", "brought by the package");
        assert_eq!(get("/content/content/run.exe").0, 404, "unknown type");
        assert_eq!(get("/content/../secret.json").0, 404);
        assert_eq!(get("/content/%2e%2e/secret.json").0, 404);
        assert_eq!(get("/player/../../secret.json").0, 404);
        assert_eq!(get("/content/missing.json").0, 404);
        let _ = std::fs::remove_dir_all(root);
    }
}
