//! Design tokens. Colours come from the Omarchy theme when one is active (and follow it live),
//! otherwise from a built-in light or dark set. Widgets never hard-code colours.

use std::path::PathBuf;
use std::time::{Duration, Instant, SystemTime};

use egui::{Color32, CornerRadius, FontData, FontDefinitions, FontFamily, FontId, Margin, Stroke, TextStyle, Visuals};

#[derive(Debug, Clone, PartialEq)]
pub struct Tokens {
    pub dark: bool,
    pub bg: Color32,
    pub sidebar: Color32,
    pub surface: Color32,
    pub surface_hi: Color32,
    pub border: Color32,
    pub text: Color32,
    pub muted: Color32,
    pub accent: Color32,
    pub on_accent: Color32,
    pub selection: Color32,
    pub user_bubble: Color32,
    pub danger: Color32,
    pub success: Color32,
    pub warning: Color32,
    pub code_bg: Color32,
    pub radius: u8,
}

fn hex(s: &str) -> Option<Color32> {
    let s = s.trim().trim_matches('"').trim_start_matches('#');
    if s.len() != 6 || !s.is_ascii() {
        return None;
    }
    let p = |i: usize| s.get(i..i + 2).and_then(|h| u8::from_str_radix(h, 16).ok());
    Some(Color32::from_rgb(p(0)?, p(2)?, p(4)?))
}

fn mix(a: Color32, b: Color32, t: f32) -> Color32 {
    let l = |x: u8, y: u8| (x as f32 + (y as f32 - x as f32) * t).round().clamp(0.0, 255.0) as u8;
    Color32::from_rgb(l(a.r(), b.r()), l(a.g(), b.g()), l(a.b(), b.b()))
}

fn luminance(c: Color32) -> f32 {
    (0.2126 * c.r() as f32 + 0.7152 * c.g() as f32 + 0.0722 * c.b() as f32) / 255.0
}

impl Tokens {
    /// AIS look, light: white, the ais.schule blue (`#107fad`, a shade darker so text in it passes
    /// WCAG AA on the tinted sidebar too), slate text; card radius 12 and control radius 8 as in
    /// AIS.chat (`packages/ui/src/styles/globals.css`: `--radius`, `--radius-enterprise-md`).
    pub fn light() -> Self {
        Self {
            dark: false,
            bg: Color32::WHITE,
            sidebar: Color32::from_rgb(0xee, 0xf5, 0xf8),
            surface: Color32::WHITE,
            surface_hi: Color32::from_rgb(0xe8, 0xf3, 0xf7),
            border: Color32::from_rgb(0xd5, 0xe3, 0xeb),
            text: Color32::from_rgb(0x1e, 0x29, 0x3b),
            muted: Color32::from_rgb(0x5f, 0x6f, 0x82),
            accent: Color32::from_rgb(0x0e, 0x74, 0xa0),
            on_accent: Color32::WHITE,
            selection: Color32::from_rgb(0xcf, 0xe6, 0xf1),
            user_bubble: Color32::from_rgb(0xe7, 0xf2, 0xf7),
            danger: Color32::from_rgb(0xd6, 0x33, 0x29),
            success: Color32::from_rgb(0x1f, 0x8a, 0x5b),
            warning: Color32::from_rgb(0xb7, 0x79, 0x1f),
            code_bg: Color32::from_rgb(0xf1, 0xf5, 0xf9),
            radius: 12,
        }
    }

    /// AIS look, dark: deep slate with a lighter AIS blue for contrast.
    pub fn dark() -> Self {
        Self {
            dark: true,
            bg: Color32::from_rgb(0x0e, 0x16, 0x22),
            sidebar: Color32::from_rgb(0x0a, 0x11, 0x1b),
            surface: Color32::from_rgb(0x15, 0x20, 0x30),
            surface_hi: Color32::from_rgb(0x1d, 0x2b, 0x3e),
            border: Color32::from_rgb(0x26, 0x36, 0x4a),
            text: Color32::from_rgb(0xe6, 0xed, 0xf5),
            muted: Color32::from_rgb(0x94, 0xa3, 0xb8),
            accent: Color32::from_rgb(0x2a, 0xa3, 0xd6),
            on_accent: Color32::from_rgb(0x06, 0x12, 0x1c),
            selection: Color32::from_rgb(0x15, 0x45, 0x62),
            user_bubble: Color32::from_rgb(0x1a, 0x2a, 0x3c),
            danger: Color32::from_rgb(0xf2, 0x44, 0x39),
            success: Color32::from_rgb(0x4c, 0xc0, 0x8a),
            warning: Color32::from_rgb(0xe8, 0xb0, 0x50),
            code_bg: Color32::from_rgb(0x09, 0x0f, 0x18),
            radius: 12,
        }
    }

    /// Tokens from an Omarchy `colors.toml` (`key = "#rrggbb"` lines).
    pub fn from_omarchy(colors: &str) -> Option<Self> {
        let get = |k: &str| {
            colors.lines().find_map(|l| {
                let (key, v) = l.split_once('=')?;
                if key.trim() != k {
                    return None;
                }
                let v = v.trim();
                hex(v.strip_prefix('"').and_then(|r| r.split('"').next()).unwrap_or(v))
            })
        };
        let bg = get("background")?;
        let fg = get("foreground")?;
        let accent = get("accent").or_else(|| get("orange")).unwrap_or(fg);
        let dark = colors.lines().any(|l| l.replace(' ', "") == "mode=\"dark\"") || luminance(bg) < 0.5;
        let base = if dark { Self::dark() } else { Self::light() };
        let on_accent = if luminance(accent) > 0.55 { Color32::from_rgb(0x10, 0x10, 0x10) } else { Color32::WHITE };
        Some(Self {
            dark,
            bg,
            sidebar: get("dark_background").unwrap_or_else(|| mix(bg, Color32::BLACK, 0.12)),
            surface: get("lighter_background").unwrap_or_else(|| mix(bg, fg, 0.06)),
            surface_hi: mix(get("lighter_background").unwrap_or(bg), fg, 0.08),
            border: mix(bg, fg, 0.16),
            text: fg,
            muted: get("muted").or_else(|| get("dark_foreground")).unwrap_or_else(|| mix(fg, bg, 0.4)),
            accent,
            on_accent,
            selection: get("selection").unwrap_or_else(|| mix(bg, accent, 0.3)),
            user_bubble: mix(get("lighter_background").unwrap_or(bg), fg, 0.04),
            danger: get("red").unwrap_or(base.danger),
            success: get("green").unwrap_or(base.success),
            warning: get("yellow").unwrap_or(base.warning),
            code_bg: get("darker_background").unwrap_or_else(|| mix(bg, Color32::BLACK, 0.2)),
            radius: 12,
        })
    }

    /// Corners of buttons, fields and menus (AIS.chat: `--radius` 0.5rem); cards use `radius`.
    pub fn control_radius(&self) -> u8 {
        self.radius.saturating_sub(4)
    }

    /// The syntax theme for code blocks: one whose colours stay readable on `code_bg`
    /// (`base16-ocean.light` has strings at about 1.9:1 on a light background).
    pub fn code_theme(&self) -> &'static str {
        if self.dark { "base16-ocean.dark" } else { "InspiredGitHub" }
    }

    pub fn visuals(&self) -> Visuals {
        let mut v = if self.dark { Visuals::dark() } else { Visuals::light() };
        v.panel_fill = self.bg;
        v.window_fill = self.surface;
        v.extreme_bg_color = self.surface;
        v.faint_bg_color = self.surface_hi;
        v.code_bg_color = self.code_bg;
        v.hyperlink_color = self.accent;
        v.override_text_color = Some(self.text);
        v.selection.bg_fill = self.selection;
        v.selection.stroke = Stroke::new(1.0, self.accent);
        v.window_stroke = Stroke::new(1.0, self.border);
        v.window_corner_radius = CornerRadius::same(self.radius.saturating_add(4));
        v.menu_corner_radius = CornerRadius::same(self.control_radius());
        v.error_fg_color = self.danger;
        v.warn_fg_color = self.warning;
        for (w, fill) in [
            (&mut v.widgets.noninteractive, self.bg),
            (&mut v.widgets.inactive, self.surface_hi),
            (&mut v.widgets.hovered, mix(self.surface_hi, self.text, 0.08)),
            (&mut v.widgets.active, mix(self.surface_hi, self.text, 0.14)),
            (&mut v.widgets.open, self.surface_hi),
        ] {
            w.corner_radius = CornerRadius::same(self.control_radius());
            w.bg_fill = fill;
            w.weak_bg_fill = fill;
            w.fg_stroke = Stroke::new(1.0, self.text);
        }
        v.widgets.noninteractive.bg_stroke = Stroke::new(1.0, self.border);
        v.widgets.inactive.bg_stroke = Stroke::NONE;
        v.widgets.hovered.bg_stroke = Stroke::new(1.0, self.border);
        v
    }

    pub fn apply(&self, ctx: &egui::Context, zoom: f32) {
        ctx.set_visuals(self.visuals());
        ctx.global_style_mut(|s| {
            s.spacing.item_spacing = egui::vec2(8.0, 6.0);
            s.spacing.button_padding = egui::vec2(10.0, 5.0);
            s.spacing.menu_margin = Margin::same(6);
            s.spacing.window_margin = Margin::same(16);
            s.spacing.interact_size.y = 26.0;
            s.text_styles = [
                (TextStyle::Small, FontId::new(12.0, FontFamily::Proportional)),
                (TextStyle::Body, FontId::new(15.0, FontFamily::Proportional)),
                (TextStyle::Button, FontId::new(14.0, FontFamily::Proportional)),
                (TextStyle::Heading, FontId::new(22.0, FontFamily::Proportional)),
                (TextStyle::Monospace, FontId::new(13.5, FontFamily::Monospace)),
            ]
            .into();
        });
        if (ctx.zoom_factor() - zoom).abs() > f32::EPSILON {
            ctx.set_zoom_factor(zoom);
        }
    }
}

/// Which tokens to use for a `theme` setting ("system", "light", "dark", "omarchy").
pub fn resolve(setting: &str, system_dark: bool) -> Tokens {
    match setting {
        "light" => Tokens::light(),
        "dark" => Tokens::dark(),
        "omarchy" => omarchy_colors().and_then(|c| Tokens::from_omarchy(&c)).unwrap_or_else(|| if system_dark { Tokens::dark() } else { Tokens::light() }),
        _ => {
            if system_dark {
                Tokens::dark()
            } else {
                Tokens::light()
            }
        }
    }
}

fn omarchy_dir() -> Option<PathBuf> {
    let home = std::env::var_os("HOME")?;
    let dir = PathBuf::from(home).join(".local/state/omarchy/current/theme");
    dir.is_dir().then_some(dir)
}

fn omarchy_colors() -> Option<String> {
    std::fs::read_to_string(omarchy_dir()?.join("colors.toml")).ok()
}

/// Notices when the desktop theme changes, checking at most every two seconds.
pub struct ThemeWatcher {
    last_check: Instant,
    stamp: Option<SystemTime>,
}

impl ThemeWatcher {
    pub fn new() -> Self {
        Self { last_check: Instant::now(), stamp: Self::stamp() }
    }

    fn stamp() -> Option<SystemTime> {
        let dir = omarchy_dir()?;
        std::fs::metadata(dir.join("colors.toml")).and_then(|m| m.modified()).ok()
    }

    pub fn changed(&mut self) -> bool {
        if self.last_check.elapsed() < Duration::from_secs(2) {
            return false;
        }
        self.last_check = Instant::now();
        let now = Self::stamp();
        let changed = now != self.stamp;
        self.stamp = now;
        changed
    }
}

impl Default for ThemeWatcher {
    fn default() -> Self {
        Self::new()
    }
}

/// Use the system's UI and monospace fonts when fontconfig finds them; egui's built-in fonts stay
/// as fallbacks (and for the icons), so everything works without them.
pub fn install_fonts(ctx: &egui::Context) {
    let mut fonts = FontDefinitions::default();
    let mut add = |name: &str, family: FontFamily, query: &str, first: bool| {
        let Some(path) = fc_match(query) else { return };
        let Ok(bytes) = std::fs::read(&path) else { return };
        fonts.font_data.insert(name.to_string(), std::sync::Arc::new(FontData::from_owned(bytes)));
        if let Some(list) = fonts.families.get_mut(&family) {
            if first { list.insert(0, name.to_string()) } else { list.push(name.to_string()) }
        }
    };
    add("ui", FontFamily::Proportional, "Noto Sans:style=Regular", true);
    add("mono", FontFamily::Monospace, "JetBrains Mono:style=Regular", true);
    // Broad symbol coverage (arrows, maths, box drawing) for whatever an answer contains.
    add("symbols", FontFamily::Proportional, "DejaVu Sans:style=Book", false);
    add("symbols-mono", FontFamily::Monospace, "DejaVu Sans Mono:style=Book", false);
    ctx.set_fonts(fonts);
    // Answers can contain any character. A glyph no font has must become a box, never a panic
    // (the test harness defaults to panicking).
    ctx.set_missing_glyph_policy(egui::epaint::text::MissingGlyphPolicy::Tofu);
}

fn fc_match(query: &str) -> Option<PathBuf> {
    let out = std::process::Command::new("fc-match").args(["-f", "%{file}", query]).output().ok()?;
    let path = PathBuf::from(String::from_utf8_lossy(&out.stdout).trim());
    let ext = path.extension()?.to_str()?.to_ascii_lowercase();
    (out.status.success() && matches!(ext.as_str(), "ttf" | "otf") && path.is_file()).then_some(path)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_omarchy_colors() {
        let t = Tokens::from_omarchy("mode = \"dark\"\naccent = \"#e96429\"\nbackground = \"#1d1a18\"\nforeground = \"#fff3e9\"\nred = \"#f55050\"").unwrap();
        assert!(t.dark);
        assert_eq!(t.accent, Color32::from_rgb(0xe9, 0x64, 0x29));
        assert_eq!(t.danger, Color32::from_rgb(0xf5, 0x50, 0x50));
        assert!(Tokens::from_omarchy("nothing here").is_none());
        assert!(Tokens::from_omarchy("background = \"#zzzzzz\"\nforeground = \"#ffffff\"").is_none());
    }
}
