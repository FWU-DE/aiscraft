//! Design tokens. Four looks: "Mutig" (night-blue sidebar, bold type), "Glas" (frosted panels on
//! a soft gradient), "Klassisch" (the earlier flat AIS look) and "Omarchy" (colours from the
//! desktop theme, followed live). Each has a light and a dark set; widgets never hard-code
//! colours. The values of Mutig and Glas come from `design/build/themes.py`.

use std::path::PathBuf;
use std::time::{Duration, Instant, SystemTime};

use egui::epaint::Shadow;
use egui::{Color32, CornerRadius, FontData, FontDefinitions, FontFamily, FontId, Margin, Stroke, TextStyle, Visuals};

/// Which design the tokens belong to. Mutig and Glas share one anatomy (sidebar with icons,
/// greeting and cards on the start page, pill actions); Klassisch and Omarchy keep the earlier one.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Look {
    Mutig,
    Glas,
    Classic,
    Omarchy,
}

/// Light, dark, or like the system.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Mode {
    System,
    Light,
    Dark,
}

/// The soft backdrop behind the Glas panels.
#[derive(Debug, Clone, PartialEq)]
pub struct Glass {
    /// Diagonal gradient, top left to bottom right.
    pub backdrop: [Color32; 2],
    /// Soft lights: centre x, y and diameter on a 1280 × 800 frame, blur radius, colour.
    pub lights: [(f32, f32, f32, f32, Color32); 5],
    /// The light 1 px edge along the top of a glass panel.
    pub edge: Color32,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Tokens {
    pub look: Look,
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
    /// Secondary text (answers' lists, sidebar entries).
    pub text2: Color32,
    pub link: Color32,
    /// Accent end colour: Glas draws primary surfaces as a gradient from `accent` to this.
    pub accent_end: Color32,
    /// Tinted accent surface (number circles, chips, icon tiles) and text on it.
    pub accent_soft: Color32,
    pub accent_ink: Color32,
    /// The second accent: coral in Mutig, lavender in Glas (favourites, hints, the active bar).
    pub accent2: Color32,
    pub accent2_soft: Color32,
    pub accent2_ink: Color32,
    pub user_text: Color32,
    /// Recessed surface (formula, segmented control, document backdrop).
    pub sunken: Color32,
    pub card_stroke: Color32,
    /// The chat area.
    pub main: Color32,
    /// The artifact panel.
    pub panel: Color32,
    pub panel_stroke: Color32,
    /// Sidebar: gradient top and bottom, text, secondary text, muted text, lines, active entry,
    /// search field, the "Neuer Chat" button and the glass edge (transparent when none).
    pub side_top: Color32,
    pub side_bottom: Color32,
    pub side_text: Color32,
    pub side_text2: Color32,
    pub side_muted: Color32,
    pub side_line: Color32,
    pub side_active: Color32,
    pub side_active_text: Color32,
    pub side_search: Color32,
    pub side_search_text: Color32,
    pub side_new: Color32,
    pub side_new_text: Color32,
    pub side_stroke: Color32,
    /// Table head and row lines.
    pub th_fill: Color32,
    pub th_text: Color32,
    pub row_line: Color32,
    /// Code block head, text and comments.
    pub code_head: Color32,
    pub code_text: Color32,
    pub code_muted: Color32,
    pub composer: Color32,
    pub composer_stroke: Color32,
    /// Filled neutral chips (model picker, "Quellen", "Erstellen").
    pub chip: Color32,
    pub chip_text: Color32,
    /// Dimming behind dialogs, and the dialog surface.
    pub overlay: Color32,
    pub modal: Color32,
    pub r_card: u8,
    pub r_composer: u8,
    pub r_bubble: u8,
    /// Radius of the floating panels (Glas); 0 where panels sit flush.
    pub r_panel: u8,
    pub card_shadow: Shadow,
    pub composer_shadow: Shadow,
    pub modal_shadow: Shadow,
    pub glass: Option<Glass>,
}

fn hex(s: &str) -> Option<Color32> {
    let s = s.trim().trim_matches('"').trim_start_matches('#');
    if s.len() != 6 || !s.is_ascii() {
        return None;
    }
    let p = |i: usize| s.get(i..i + 2).and_then(|h| u8::from_str_radix(h, 16).ok());
    Some(Color32::from_rgb(p(0)?, p(2)?, p(4)?))
}

/// A colour from a constant `0xRRGGBB`.
const fn rgb(c: u32) -> Color32 {
    Color32::from_rgb((c >> 16) as u8, (c >> 8) as u8, c as u8)
}

/// A colour from a constant `0xRRGGBBAA` (unmultiplied alpha, as in the design files).
fn rgba(c: u32) -> Color32 {
    Color32::from_rgba_unmultiplied((c >> 24) as u8, (c >> 16) as u8, (c >> 8) as u8, c as u8)
}

pub fn mix(a: Color32, b: Color32, t: f32) -> Color32 {
    let l = |x: u8, y: u8| (x as f32 + (y as f32 - x as f32) * t).round().clamp(0.0, 255.0) as u8;
    Color32::from_rgb(l(a.r(), b.r()), l(a.g(), b.g()), l(a.b(), b.b()))
}

/// `c` (possibly translucent) laid over the opaque `under`: the colour a person sees.
pub fn over(c: Color32, under: Color32) -> Color32 {
    let a = c.a() as f32 / 255.0;
    // Color32 is premultiplied.
    let l = |x: u8, y: u8| (x as f32 + y as f32 * (1.0 - a)).round().clamp(0.0, 255.0) as u8;
    Color32::from_rgb(l(c.r(), under.r()), l(c.g(), under.g()), l(c.b(), under.b()))
}

fn luminance(c: Color32) -> f32 {
    (0.2126 * c.r() as f32 + 0.7152 * c.g() as f32 + 0.0722 * c.b() as f32) / 255.0
}

fn shadow(y: i8, blur: u8, color: Color32) -> Shadow {
    Shadow { offset: [0, y], blur, spread: 0, color }
}

impl Tokens {
    /// Klassisch, light: white, the ais.schule blue (`#0e74a0`, a shade darker so text in it
    /// passes WCAG AA on the tinted sidebar too), slate text; card radius 12 and control radius 8
    /// as in AIS.chat (`packages/ui/src/styles/globals.css`).
    pub fn light() -> Self {
        let mut t = Self {
            look: Look::Classic,
            dark: false,
            bg: rgb(0xf7fafc),
            sidebar: rgb(0xedf3f8),
            surface: Color32::WHITE,
            surface_hi: rgb(0xe7eff5),
            border: rgb(0xd3dfe8),
            text: rgb(0x171c20),
            muted: rgb(0x4a5863),
            accent: rgb(0x0e74a0),
            on_accent: Color32::WHITE,
            selection: rgb(0xcfe5f7),
            user_bubble: rgb(0xd9eefe),
            danger: rgb(0xd63329),
            success: rgb(0x1f8a5b),
            warning: rgb(0xb7791f),
            code_bg: rgb(0xedf3f8),
            radius: 12,
            ..Self::base()
        };
        t.derive_flat();
        t
    }

    /// Klassisch, dark: deep slate with a lighter AIS blue for contrast.
    pub fn dark() -> Self {
        let mut t = Self {
            look: Look::Classic,
            dark: true,
            bg: rgb(0x0e1418),
            sidebar: rgb(0x171c20),
            surface: rgb(0x1b2025),
            surface_hi: rgb(0x252b2f),
            border: rgb(0x3b4852),
            text: rgb(0xdce3e9),
            muted: rgb(0xbac8d4),
            accent: rgb(0x83cffe),
            on_accent: rgb(0x02344a),
            selection: rgb(0x354957),
            user_bubble: rgb(0x024c6a),
            danger: rgb(0xf24439),
            success: rgb(0x4cc08a),
            warning: rgb(0xe8b050),
            code_bg: rgb(0x070f14),
            radius: 12,
            ..Self::base()
        };
        t.derive_flat();
        t
    }

    /// Neutral defaults every look starts from.
    fn base() -> Self {
        let z = Color32::TRANSPARENT;
        Self {
            look: Look::Classic,
            dark: false,
            bg: z,
            sidebar: z,
            surface: z,
            surface_hi: z,
            border: z,
            text: z,
            muted: z,
            accent: z,
            on_accent: z,
            selection: z,
            user_bubble: z,
            danger: z,
            success: z,
            warning: z,
            code_bg: z,
            radius: 12,
            text2: z,
            link: z,
            accent_end: z,
            accent_soft: z,
            accent_ink: z,
            accent2: z,
            accent2_soft: z,
            accent2_ink: z,
            user_text: z,
            sunken: z,
            card_stroke: z,
            main: z,
            panel: z,
            panel_stroke: z,
            side_top: z,
            side_bottom: z,
            side_text: z,
            side_text2: z,
            side_muted: z,
            side_line: z,
            side_active: z,
            side_active_text: z,
            side_search: z,
            side_search_text: z,
            side_new: z,
            side_new_text: z,
            side_stroke: z,
            th_fill: z,
            th_text: z,
            row_line: z,
            code_head: z,
            code_text: z,
            code_muted: z,
            composer: z,
            composer_stroke: z,
            chip: z,
            chip_text: z,
            overlay: z,
            modal: z,
            r_card: 12,
            r_composer: 18,
            r_bubble: 16,
            r_panel: 0,
            card_shadow: Shadow::NONE,
            composer_shadow: Shadow::NONE,
            modal_shadow: Shadow::NONE,
            glass: None,
        }
    }

    /// The design roles of a flat look (Klassisch, Omarchy), from its base colours.
    fn derive_flat(&mut self) {
        self.text2 = self.text;
        self.link = self.accent;
        self.accent_end = self.accent;
        self.accent_soft = mix(self.bg, self.accent, 0.18);
        self.accent_ink = if self.dark { mix(self.accent, self.text, 0.3) } else { mix(self.accent, self.text, 0.35) };
        self.accent2 = self.warning;
        self.accent2_soft = mix(self.bg, self.warning, 0.18);
        self.accent2_ink = self.text;
        self.user_text = self.text;
        self.sunken = self.surface_hi;
        self.card_stroke = self.border;
        self.main = self.bg;
        self.panel = self.surface;
        self.panel_stroke = self.border;
        self.side_top = self.sidebar;
        self.side_bottom = self.sidebar;
        self.side_text = self.text;
        self.side_text2 = self.text;
        self.side_muted = self.muted;
        self.side_line = self.border;
        self.side_active = self.selection;
        self.side_active_text = self.text;
        self.side_search = self.surface;
        self.side_search_text = self.muted;
        self.side_new = self.surface_hi;
        self.side_new_text = self.text;
        self.side_stroke = Color32::TRANSPARENT;
        self.th_fill = self.surface_hi;
        self.th_text = self.text;
        self.row_line = self.border;
        self.code_head = self.surface_hi;
        self.code_text = self.text;
        self.code_muted = self.muted;
        self.composer = self.surface;
        self.composer_stroke = self.border;
        self.chip = self.surface_hi;
        self.chip_text = self.muted;
        self.overlay = Color32::from_black_alpha(if self.dark { 120 } else { 70 });
        self.modal = self.surface;
        self.r_card = self.radius;
        self.r_composer = self.radius.saturating_add(6);
        self.r_bubble = self.radius.saturating_add(4);
        let shade = if self.dark { Color32::from_black_alpha(90) } else { Color32::from_rgba_unmultiplied(0, 41, 102, 26) };
        self.composer_shadow = shadow(2, 12, Color32::from_black_alpha(if self.dark { 60 } else { 18 }));
        self.modal_shadow = shadow(8, 28, shade);
    }

    /// "Mutig", light: a night-blue sidebar, a calm light work area, Barlow Bold titles.
    pub fn mutig_light() -> Self {
        let navy = rgba(0x0b2a45ff);
        Self {
            look: Look::Mutig,
            dark: false,
            bg: rgb(0xf3f6f9),
            sidebar: rgb(0x0c2e4e),
            surface: Color32::WHITE,
            surface_hi: rgb(0xeef3f7),
            border: rgb(0xdce4ec),
            text: rgb(0x0d1b2a),
            muted: rgb(0x566578),
            accent: rgb(0x107fad),
            on_accent: Color32::WHITE,
            selection: rgb(0xdff0f8),
            user_bubble: rgb(0xdceef8),
            danger: rgb(0xc8352b),
            success: rgb(0x1f9d6b),
            warning: rgb(0xa86412),
            code_bg: rgb(0x0c2238),
            radius: 16,
            text2: rgb(0x33445a),
            link: rgb(0x0b6e98),
            accent_end: rgb(0x107fad),
            accent_soft: rgb(0xdff0f8),
            accent_ink: rgb(0x0a5c80),
            accent2: rgb(0xf0663f),
            accent2_soft: rgb(0xfde7df),
            accent2_ink: rgb(0xa93a18),
            user_text: rgb(0x0d1b2a),
            sunken: rgb(0xeaf0f5),
            card_stroke: rgb(0xdfe6ee),
            main: rgb(0xf3f6f9),
            panel: Color32::WHITE,
            panel_stroke: rgb(0xdce4ec),
            side_top: rgb(0x0c2e4e),
            side_bottom: rgb(0x081f37),
            side_text: rgb(0xf3f8fc),
            side_text2: rgb(0xc9d8e6),
            side_muted: rgb(0x8fafcc),
            side_line: rgb(0x1d3e60),
            side_active: rgb(0x17507d),
            side_active_text: Color32::WHITE,
            side_search: rgb(0x132f4e),
            side_search_text: rgb(0xa9c0d6),
            side_new: rgb(0x107fad),
            side_new_text: Color32::WHITE,
            side_stroke: Color32::TRANSPARENT,
            th_fill: rgb(0x0c2e4e),
            th_text: Color32::WHITE,
            row_line: rgb(0xe3e9ef),
            code_head: rgb(0x14304c),
            code_text: rgb(0xe6edf5),
            code_muted: rgb(0x94a9be),
            composer: Color32::WHITE,
            composer_stroke: rgb(0xd5dfe9),
            chip: rgb(0xeef3f7),
            chip_text: rgb(0x24364b),
            overlay: rgba(0x06182b8c),
            modal: Color32::WHITE,
            r_card: 16,
            r_composer: 22,
            r_bubble: 20,
            r_panel: 0,
            card_shadow: shadow(6, 18, navy.gamma_multiply(0x10 as f32 / 255.0)),
            composer_shadow: shadow(14, 36, navy.gamma_multiply(0x1f as f32 / 255.0)),
            modal_shadow: shadow(24, 64, navy.gamma_multiply(0x40 as f32 / 255.0)),
            glass: None,
        }
    }

    /// "Mutig", dark: the sidebar a shade darker than the work area, the AIS blue lighter.
    pub fn mutig_dark() -> Self {
        Self {
            dark: true,
            bg: rgb(0x0d1723),
            sidebar: rgb(0x071b30),
            surface: rgb(0x142234),
            surface_hi: rgb(0x1d2f44),
            border: rgb(0x22364d),
            text: rgb(0xe8eef5),
            muted: rgb(0x8e9fb2),
            accent: rgb(0x38a8db),
            on_accent: rgb(0x04131f),
            selection: rgb(0x113a53),
            user_bubble: rgb(0x16405c),
            danger: rgb(0xff7a6e),
            success: rgb(0x3ccb91),
            warning: rgb(0xf0b860),
            code_bg: rgb(0x081422),
            text2: rgb(0xbdcad8),
            link: rgb(0x7cc8ec),
            accent_end: rgb(0x38a8db),
            accent_soft: rgb(0x113a53),
            accent_ink: rgb(0x9ad7f2),
            accent2: rgb(0xff8a66),
            accent2_soft: rgb(0x3d2219),
            accent2_ink: rgb(0xffb59c),
            user_text: rgb(0xeef5fb),
            sunken: rgb(0x1a2a3e),
            card_stroke: rgb(0x22364d),
            main: rgb(0x0d1723),
            panel: rgb(0x132030),
            panel_stroke: rgb(0x22344a),
            side_top: rgb(0x071b30),
            side_bottom: rgb(0x041222),
            side_text: rgb(0xeef4fa),
            side_text2: rgb(0xc2d2e2),
            side_muted: rgb(0x87a3bf),
            side_line: rgb(0x14304c),
            side_active: rgb(0x123f63),
            side_search: rgb(0x0d2741),
            side_search_text: rgb(0x9db5cc),
            side_new: rgb(0x38a8db),
            side_new_text: rgb(0x04131f),
            th_fill: rgb(0x1b3a57),
            row_line: rgb(0x22364d),
            code_head: rgb(0x0f2033),
            composer: rgb(0x142234),
            composer_stroke: rgb(0x26405a),
            chip: rgb(0x1d2f44),
            chip_text: rgb(0xd4dee8),
            modal: rgb(0x142234),
            card_shadow: Shadow::NONE,
            composer_shadow: shadow(14, 36, Color32::from_black_alpha(0x4d)),
            modal_shadow: shadow(24, 64, Color32::from_black_alpha(0x66)),
            ..Self::mutig_light()
        }
    }

    /// "Glas", light: frosted, translucent panels over a soft gradient with five lights.
    pub fn glas_light() -> Self {
        let shade = |a: u8| Color32::from_rgba_unmultiplied(0x1b, 0x3a, 0x5a, a);
        Self {
            look: Look::Glas,
            dark: false,
            bg: rgb(0xe8ecf7),
            sidebar: rgba(0xffffff7a),
            surface: rgba(0xffffffb8),
            surface_hi: rgb(0xeef4f9),
            border: rgba(0x1b3a5a1f),
            text: rgb(0x0e1e2e),
            muted: rgb(0x4a5d71),
            accent: rgb(0x107fad),
            on_accent: Color32::WHITE,
            selection: rgba(0xd3ecf8cc),
            user_bubble: rgba(0xcfe8f6e6),
            danger: rgb(0xc0352b),
            success: rgb(0x16875a),
            warning: rgb(0x9a5f12),
            code_bg: rgba(0x0f2236e6),
            radius: 18,
            text2: rgb(0x2f4256),
            link: rgb(0x0b6a93),
            accent_end: rgb(0x0a5e86),
            accent_soft: rgba(0xd3ecf8cc),
            accent_ink: rgb(0x0a5577),
            accent2: rgb(0x7b5ce5),
            accent2_soft: rgba(0xe9e2fccc),
            accent2_ink: rgb(0x5434c4),
            user_text: rgb(0x0e1e2e),
            sunken: rgba(0xe9f0f7b3),
            card_stroke: rgba(0xfffffff2),
            main: rgba(0xffffffa6),
            panel: rgba(0xffffffc4),
            panel_stroke: rgba(0xffffffe6),
            side_top: rgba(0xffffff7a),
            side_bottom: rgba(0xffffff7a),
            side_text: rgb(0x0e1e2e),
            side_text2: rgb(0x2f4256),
            side_muted: rgb(0x4a5d71),
            side_line: rgba(0xffffff99),
            side_active: rgba(0xffffffd9),
            side_active_text: rgb(0x0a4f70),
            side_search: rgba(0xffffff99),
            side_search_text: rgb(0x4a5d71),
            side_new: rgb(0x107fad),
            side_new_text: Color32::WHITE,
            side_stroke: rgba(0xffffffcc),
            th_fill: rgb(0xe2eef7),
            th_text: rgb(0x0a4563),
            row_line: rgba(0x1b3a5a17),
            code_head: rgba(0xffffff14),
            code_text: rgb(0xe6edf5),
            code_muted: rgb(0x9db1c5),
            composer: rgba(0xffffffe6),
            composer_stroke: Color32::WHITE,
            chip: rgb(0xeef4f9),
            chip_text: rgb(0x24364b),
            overlay: rgba(0xc9dceb99),
            modal: rgba(0xffffffeb),
            r_card: 18,
            r_composer: 24,
            r_bubble: 20,
            r_panel: 24,
            card_shadow: shadow(6, 18, shade(0x12)),
            composer_shadow: shadow(20, 44, shade(0x29)),
            modal_shadow: shadow(30, 70, shade(0x33)),
            glass: Some(Glass {
                backdrop: [rgb(0xe4eef7), rgb(0xeee9f7)],
                lights: [
                    (150.0, 110.0, 620.0, 150.0, rgb(0x6fc3ec)),
                    (1080.0, 60.0, 640.0, 160.0, rgb(0xb9a8f4)),
                    (1210.0, 730.0, 620.0, 160.0, rgb(0x8ee0d2)),
                    (400.0, 750.0, 560.0, 150.0, rgb(0xffcbae)),
                    (640.0, 330.0, 420.0, 140.0, rgb(0x9fd6f2)),
                ],
                edge: rgba(0xffffffcc),
            }),
        }
    }

    /// "Glas", dark: deep blue glass, the lights darker and more saturated.
    pub fn glas_dark() -> Self {
        Self {
            dark: true,
            bg: rgb(0x0b1224),
            sidebar: rgba(0x0b1626a6),
            surface: rgba(0xffffff0f),
            surface_hi: rgba(0xffffff14),
            border: rgba(0xffffff1a),
            text: rgb(0xeef4fa),
            muted: rgb(0x9aabbd),
            on_accent: rgb(0x04131f),
            selection: rgba(0x38a8db2e),
            user_bubble: rgba(0x2a7fb04d),
            danger: rgb(0xff7a6e),
            success: rgb(0x3ccb91),
            warning: rgb(0xf0b860),
            code_bg: rgba(0x050c16b3),
            text2: rgb(0xc7d3e0),
            link: rgb(0x86cff0),
            accent: rgb(0x5cc3ee),
            accent_end: rgb(0x2a93c6),
            accent_soft: rgba(0x38a8db2e),
            accent_ink: rgb(0xa6ddf5),
            accent2: rgb(0xa48bff),
            accent2_soft: rgba(0xa48bff2e),
            accent2_ink: rgb(0xcdbeff),
            user_text: rgb(0xf2f8fd),
            sunken: rgba(0xffffff0d),
            card_stroke: rgba(0xffffff1a),
            main: rgba(0x0d1a2bb8),
            panel: rgba(0x0d1a2bcc),
            panel_stroke: rgba(0xffffff1f),
            side_top: rgba(0x0b1626a6),
            side_bottom: rgba(0x0b1626a6),
            side_text: rgb(0xeef4fa),
            side_text2: rgb(0xc7d3e0),
            side_muted: rgb(0x9aabbd),
            side_line: rgba(0xffffff1a),
            side_active: rgba(0xffffff1f),
            side_active_text: Color32::WHITE,
            side_search: rgba(0xffffff14),
            side_search_text: rgb(0xa9b8c8),
            side_new: rgb(0x38a8db),
            side_new_text: rgb(0x04131f),
            side_stroke: rgba(0xffffff24),
            th_fill: rgba(0xffffff14),
            th_text: rgb(0xeef4fa),
            row_line: rgba(0xffffff14),
            code_head: rgba(0xffffff0d),
            composer: rgba(0x13243adb),
            composer_stroke: rgba(0xffffff2e),
            chip: rgba(0xffffff14),
            chip_text: rgb(0xdce6f0),
            overlay: rgba(0x02060eb3),
            modal: rgba(0x101e31f2),
            card_shadow: Shadow::NONE,
            composer_shadow: shadow(20, 44, Color32::from_black_alpha(0x5c)),
            modal_shadow: shadow(30, 70, Color32::from_black_alpha(0x80)),
            glass: Some(Glass {
                backdrop: [rgb(0x07111f), rgb(0x11132e)],
                lights: [
                    (140.0, 100.0, 640.0, 160.0, rgb(0x0f6a97)),
                    (1100.0, 80.0, 640.0, 170.0, rgb(0x4a3aa0)),
                    (1220.0, 730.0, 600.0, 170.0, rgb(0x126e66)),
                    (360.0, 760.0, 560.0, 160.0, rgb(0x6b3352)),
                    (650.0, 370.0, 420.0, 150.0, rgb(0x0e4f7a)),
                ],
                edge: rgba(0xffffff24),
            }),
            ..Self::glas_light()
        }
    }

    /// Glas without translucency ("Transparenz: Aus"): every surface laid over the backdrop's
    /// middle colour once, so it reads the same but nothing shines through.
    pub fn solid(mut self) -> Self {
        let Some(g) = self.glass.as_ref() else { return self };
        let under = mix(g.backdrop[0], g.backdrop[1], 0.5);
        for c in [
            &mut self.sidebar,
            &mut self.surface,
            &mut self.surface_hi,
            &mut self.selection,
            &mut self.user_bubble,
            &mut self.code_bg,
            &mut self.accent_soft,
            &mut self.accent2_soft,
            &mut self.sunken,
            &mut self.main,
            &mut self.panel,
            &mut self.side_top,
            &mut self.side_bottom,
            &mut self.side_active,
            &mut self.side_search,
            &mut self.th_fill,
            &mut self.composer,
            &mut self.chip,
            &mut self.modal,
        ] {
            *c = over(*c, under);
        }
        self
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
        let mut t = Self {
            look: Look::Omarchy,
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
            ..Self::base()
        };
        t.derive_flat();
        if let Some(m) = get("magenta") {
            t.accent2 = m;
            t.accent2_soft = mix(bg, m, 0.25);
        }
        Some(t)
    }

    /// The tokens inside the sidebar: its own text, muted and line colours (Mutig's sidebar is
    /// dark in the light scheme too), so plain widgets there stay readable.
    pub fn for_sidebar(&self) -> Tokens {
        if !self.designed() {
            return self.clone();
        }
        let mut t = self.clone();
        t.text = self.side_text2;
        t.muted = self.side_muted;
        t.border = self.side_line;
        t.surface = self.side_search;
        t.surface_hi = self.side_search;
        t.selection = self.side_active;
        t.bg = self.side_bottom;
        t.main = self.side_bottom;
        t.dark = self.dark || self.look == Look::Mutig;
        t
    }

    /// Mutig and Glas: the new anatomy (icons, greeting, cards, pills).
    pub fn designed(&self) -> bool {
        matches!(self.look, Look::Mutig | Look::Glas)
    }

    /// Corners of buttons, fields and menus (AIS.chat: `--radius` 0.5rem); cards use `radius`.
    pub fn control_radius(&self) -> u8 {
        if self.designed() { 12 } else { self.radius.saturating_sub(4) }
    }

    /// The syntax theme for code blocks: one whose colours stay readable on `code_bg`
    /// (`base16-ocean.light` has strings at about 1.9:1 on a light background). Mutig and Glas
    /// put code on a dark block in both schemes.
    pub fn code_theme(&self) -> String {
        if self.designed() {
            let c = self.opaque(self.code_bg);
            format!("aiscraft-{:02x}{:02x}{:02x}", c.r(), c.g(), c.b())
        } else if self.dark {
            "base16-ocean.dark".into()
        } else {
            "InspiredGitHub".into()
        }
    }

    /// Mutig and Glas: a syntax theme (tmTheme) on the code block's own background, with the
    /// designs' colours for comments, keywords, strings and numbers.
    pub fn code_theme_xml(&self) -> Option<String> {
        if !self.designed() {
            return None;
        }
        let h = |c: Color32| format!("#{:02X}{:02X}{:02X}", c.r(), c.g(), c.b());
        let rule = |scope: &str, color: &str| {
            format!("<dict><key>scope</key><string>{scope}</string><key>settings</key><dict><key>foreground</key><string>{color}</string></dict></dict>")
        };
        let rules = [
            rule("comment, punctuation.definition.comment", &h(self.code_muted)),
            rule("keyword, storage, support.function, entity.name.function, variable.function", "#7DD3FC"),
            rule("string, punctuation.definition.string", "#FDBA74"),
            rule("constant.numeric, constant.language, constant.character", "#C4B5FD"),
            rule("entity.name.tag, entity.name.type, support.type, support.class", "#86EFAC"),
        ]
        .join("");
        Some(format!(
            "<?xml version=\"1.0\" encoding=\"UTF-8\"?><plist version=\"1.0\"><dict><key>name</key><string>{name}</string><key>settings</key><array><dict><key>settings</key><dict><key>background</key><string>{bg}</string><key>foreground</key><string>{fg}</string><key>caret</key><string>{fg}</string><key>selection</key><string>#2A4A6B</string></dict></dict>{rules}</array></dict></plist>",
            name = self.code_theme(),
            bg = h(self.opaque(self.code_bg)),
            fg = h(self.code_text),
        ))
    }

    /// An opaque stand-in for a translucent colour (menus and tooltips must not show what lies
    /// behind them).
    pub fn opaque(&self, c: Color32) -> Color32 {
        let under = self.glass.as_ref().map(|g| mix(g.backdrop[0], g.backdrop[1], 0.5)).unwrap_or(self.bg);
        over(c, under)
    }

    pub fn visuals(&self) -> Visuals {
        let mut v = if self.dark { Visuals::dark() } else { Visuals::light() };
        let designed = self.designed();
        v.panel_fill = if designed { self.opaque(self.main) } else { self.bg };
        v.window_fill = if designed { self.opaque(self.modal) } else { self.surface };
        v.extreme_bg_color = if designed { self.opaque(self.surface) } else { self.surface };
        v.faint_bg_color = self.opaque(self.surface_hi);
        v.code_bg_color = if designed { self.opaque(self.sunken) } else { self.code_bg };
        v.hyperlink_color = self.link;
        v.override_text_color = Some(self.text);
        v.selection.bg_fill = self.opaque(self.selection);
        v.selection.stroke = Stroke::new(1.0, self.accent);
        v.window_corner_radius = CornerRadius::same(if designed { 24 } else { self.radius.saturating_add(4) });
        v.menu_corner_radius = CornerRadius::same(self.control_radius());
        let shade = if self.dark { Color32::from_black_alpha(90) } else { Color32::from_rgba_unmultiplied(0, 41, 102, 26) };
        v.popup_shadow = Shadow { offset: [0, 4], blur: 12, spread: 0, color: shade };
        v.window_shadow = if designed { self.modal_shadow } else { Shadow { offset: [0, 8], blur: 28, spread: 0, color: shade } };
        v.window_stroke = Stroke::NONE;
        v.error_fg_color = self.danger;
        v.warn_fg_color = self.warning;
        let rest = self.opaque(self.surface_hi);
        for (w, fill) in [
            (&mut v.widgets.noninteractive, self.opaque(self.main)),
            (&mut v.widgets.inactive, rest),
            // State layers: hover 8 %, pressed 10 % of the text colour.
            (&mut v.widgets.hovered, mix(rest, self.text, 0.08)),
            (&mut v.widgets.active, mix(rest, self.text, 0.10)),
            (&mut v.widgets.open, rest),
        ] {
            w.corner_radius = CornerRadius::same(self.control_radius());
            w.bg_fill = fill;
            w.weak_bg_fill = fill;
            w.fg_stroke = Stroke::new(1.0, self.text);
        }
        v.widgets.noninteractive.bg_stroke = Stroke::new(1.0, if designed { self.opaque(self.border) } else { self.border });
        v.widgets.inactive.bg_stroke = Stroke::NONE;
        v.widgets.hovered.bg_stroke = Stroke::NONE;
        let r = if designed { 12 } else { 10 };
        v.widgets.inactive.corner_radius = CornerRadius::same(r);
        v.widgets.hovered.corner_radius = CornerRadius::same(r);
        v.widgets.active.corner_radius = CornerRadius::same(r);
        v
    }

    pub fn apply(&self, ctx: &egui::Context, zoom: f32) {
        ctx.set_visuals(self.visuals());
        let designed = self.designed();
        ctx.global_style_mut(|s| {
            s.spacing.item_spacing = egui::vec2(8.0, 6.0);
            s.spacing.button_padding = egui::vec2(12.0, 6.0);
            s.spacing.menu_margin = Margin::same(8);
            s.spacing.window_margin = Margin::same(16);
            s.spacing.interact_size.y = 30.0;
            // Type scale on Barlow (design/README.md): body 15.5 (16 in Klassisch), UI 14.5,
            // small 13, titles in Bold (SemiBold in Klassisch). install_fonts registers the
            // families before the first apply.
            let medium = FontFamily::Name(MEDIUM.into());
            let heading = FontFamily::Name(if designed { BOLD } else { SEMIBOLD }.into());
            let button = FontFamily::Name(if designed { SEMIBOLD } else { MEDIUM }.into());
            s.text_styles = [
                (TextStyle::Small, FontId::new(13.0, FontFamily::Proportional)),
                (TextStyle::Body, FontId::new(if designed { 15.5 } else { 16.0 }, FontFamily::Proportional)),
                (TextStyle::Button, FontId::new(14.5, if designed { button } else { medium })),
                (TextStyle::Heading, FontId::new(if designed { 23.0 } else { 24.0 }, heading)),
                (TextStyle::Monospace, FontId::new(13.5, FontFamily::Monospace)),
            ]
            .into();
        });
        if (ctx.zoom_factor() - zoom).abs() > f32::EPSILON {
            ctx.set_zoom_factor(zoom);
        }
    }
}

/// The `theme` setting split into look and mode. Older values ("system", "light", "dark") are
/// Klassisch; unknown values fall back to Mutig like the system.
pub fn parse(setting: &str) -> (Look, Mode) {
    let (look, mode) = setting.split_once('-').unwrap_or((setting, ""));
    let mode = match mode {
        "light" => Mode::Light,
        "dark" => Mode::Dark,
        _ => Mode::System,
    };
    match look {
        "system" => (Look::Classic, Mode::System),
        "light" => (Look::Classic, Mode::Light),
        "dark" => (Look::Classic, Mode::Dark),
        "classic" => (Look::Classic, mode),
        "omarchy" => (Look::Omarchy, Mode::System),
        "glas" => (Look::Glas, mode),
        _ => (Look::Mutig, mode),
    }
}

/// The `theme` setting for a look and mode (the inverse of [`parse`]).
pub fn setting(look: Look, mode: Mode) -> String {
    let suffix = match mode {
        Mode::System => "",
        Mode::Light => "-light",
        Mode::Dark => "-dark",
    };
    match look {
        Look::Classic => match mode {
            Mode::System => "system".into(),
            Mode::Light => "light".into(),
            Mode::Dark => "dark".into(),
        },
        Look::Omarchy => "omarchy".into(),
        Look::Mutig => format!("mutig{suffix}"),
        Look::Glas => format!("glas{suffix}"),
    }
}

/// The tokens of one look in one scheme.
pub fn tokens(look: Look, dark: bool) -> Tokens {
    match (look, dark) {
        (Look::Mutig, false) => Tokens::mutig_light(),
        (Look::Mutig, true) => Tokens::mutig_dark(),
        (Look::Glas, false) => Tokens::glas_light(),
        (Look::Glas, true) => Tokens::glas_dark(),
        (Look::Classic | Look::Omarchy, false) => Tokens::light(),
        (Look::Classic | Look::Omarchy, true) => Tokens::dark(),
    }
}

/// Which tokens to use for a `theme` setting (see [`parse`]); `solid` turns Glas' translucency
/// off (setting `transparency` = "off").
pub fn resolve(setting: &str, system_dark: bool, solid: bool) -> Tokens {
    let (look, mode) = parse(setting);
    let dark = match mode {
        Mode::System => system_dark,
        Mode::Light => false,
        Mode::Dark => true,
    };
    let t = match look {
        Look::Omarchy => omarchy_colors().and_then(|c| Tokens::from_omarchy(&c)).unwrap_or_else(|| tokens(Look::Classic, dark)),
        look => tokens(look, dark),
    };
    if solid { t.solid() } else { t }
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

/// Barlow in four weights, the Lucide icons, and the system's fonts where fontconfig finds them;
/// egui's built-in fonts stay as fallbacks, so everything works without them.
pub fn install_fonts(ctx: &egui::Context) {
    ctx.set_fonts(font_definitions(true));
    // Answers can contain any character. A glyph no font has must become a box, never a panic
    // (the test harness defaults to panicking).
    ctx.set_missing_glyph_policy(egui::epaint::text::MissingGlyphPolicy::Tofu);
}

/// The font set; `system` adds fonts found through fontconfig (tests leave them out).
pub fn font_definitions(system: bool) -> FontDefinitions {
    let mut fonts = FontDefinitions::default();
    // Barlow (OFL, the font of AIS.chat and ais.schule); every family falls back to the system
    // UI font and egui's bundled fonts for symbols Barlow lacks (arrows, ✓, ⬆ …).
    let fallback: Vec<String> = fonts.families.get(&FontFamily::Proportional).cloned().unwrap_or_default();
    for (name, bytes) in [
        ("barlow", &include_bytes!("../assets/fonts/Barlow-Regular.ttf")[..]),
        ("barlow-medium", &include_bytes!("../assets/fonts/Barlow-Medium.ttf")[..]),
        ("barlow-semibold", &include_bytes!("../assets/fonts/Barlow-SemiBold.ttf")[..]),
        ("barlow-bold", &include_bytes!("../assets/fonts/Barlow-Bold.ttf")[..]),
        // Lucide (ISC), only the icons the app uses (assets/fonts/lucide-icons.txt).
        ("lucide", &include_bytes!("../assets/fonts/Lucide.ttf")[..]),
    ] {
        fonts.font_data.insert(name.to_string(), std::sync::Arc::new(FontData::from_static(bytes)));
    }
    for (family, first) in [(MEDIUM, "barlow-medium"), (SEMIBOLD, "barlow-semibold"), (BOLD, "barlow-bold"), (ICONS, "lucide")] {
        let mut list = vec![first.to_string()];
        list.extend(fallback.iter().cloned());
        fonts.families.insert(FontFamily::Name(family.into()), list);
    }
    if let Some(list) = fonts.families.get_mut(&FontFamily::Proportional) {
        list.insert(0, "barlow".to_string());
    }
    if !system {
        return fonts;
    }
    // The system UI font only fills in glyphs Barlow lacks (₂, Greek, …), right after Barlow.
    if let Some(path) = fc_match("Noto Sans:style=Regular")
        && let Ok(bytes) = std::fs::read(&path)
    {
        fonts.font_data.insert("ui".to_string(), std::sync::Arc::new(FontData::from_owned(bytes)));
        for fam in [FontFamily::Proportional, FontFamily::Name(MEDIUM.into()), FontFamily::Name(SEMIBOLD.into()), FontFamily::Name(BOLD.into())] {
            if let Some(list) = fonts.families.get_mut(&fam) {
                list.insert(1.min(list.len()), "ui".to_string());
            }
        }
    }
    let mut add = |name: &str, family: FontFamily, query: &str, first: bool| {
        let Some(path) = fc_match(query) else { return };
        let Ok(bytes) = std::fs::read(&path) else { return };
        fonts.font_data.insert(name.to_string(), std::sync::Arc::new(FontData::from_owned(bytes)));
        if let Some(list) = fonts.families.get_mut(&family) {
            if first { list.insert(0, name.to_string()) } else { list.push(name.to_string()) }
        }
    };
    add("mono", FontFamily::Monospace, "JetBrains Mono:style=Regular", true);
    // Broad symbol coverage (arrows, maths, box drawing) for whatever an answer contains.
    add("symbols", FontFamily::Proportional, "DejaVu Sans:style=Book", false);
    add("symbols-mono", FontFamily::Monospace, "DejaVu Sans Mono:style=Book", false);
    fonts
}

/// Font family names for heavier Barlow cuts (egui has no font weights, only families) and the
/// icon font.
pub const MEDIUM: &str = "medium";
pub const SEMIBOLD: &str = "semibold";
pub const BOLD: &str = "bold";
pub const ICONS: &str = "icons";

/// Text in the semibold cut (names, the main button).
pub fn semibold(text: impl Into<String>) -> egui::RichText {
    egui::RichText::new(text).family(FontFamily::Name(SEMIBOLD.into()))
}

/// Text in the medium cut (labels, chips).
pub fn medium(text: impl Into<String>) -> egui::RichText {
    egui::RichText::new(text).family(FontFamily::Name(MEDIUM.into()))
}

/// Text in the bold cut (titles, table heads).
pub fn bold(text: impl Into<String>) -> egui::RichText {
    egui::RichText::new(text).family(FontFamily::Name(BOLD.into()))
}

/// Titles: Bold in Mutig and Glas, SemiBold in Klassisch and Omarchy.
pub fn title(t: &Tokens, text: impl Into<String>) -> egui::RichText {
    if t.designed() { bold(text) } else { semibold(text) }
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
        assert_eq!(t.look, Look::Omarchy);
        assert_eq!(t.accent, Color32::from_rgb(0xe9, 0x64, 0x29));
        assert_eq!(t.danger, Color32::from_rgb(0xf5, 0x50, 0x50));
        assert!(Tokens::from_omarchy("nothing here").is_none());
        assert!(Tokens::from_omarchy("background = \"#zzzzzz\"\nforeground = \"#ffffff\"").is_none());
    }

    #[test]
    fn theme_settings_round_trip() {
        for look in [Look::Mutig, Look::Glas, Look::Classic] {
            for mode in [Mode::System, Mode::Light, Mode::Dark] {
                assert_eq!(parse(&setting(look, mode)), (look, mode), "{look:?} {mode:?}");
            }
        }
        assert_eq!(parse("omarchy"), (Look::Omarchy, Mode::System));
        assert_eq!(parse("system"), (Look::Classic, Mode::System));
        assert_eq!(parse("dark"), (Look::Classic, Mode::Dark));
        assert_eq!(parse("whatever"), (Look::Mutig, Mode::System));
        assert_eq!(parse(""), (Look::Mutig, Mode::System));
        let t = resolve("glas-dark", false, false);
        assert!(t.dark && t.look == Look::Glas && t.glass.is_some());
        assert!(!resolve("mutig", false, false).dark);
        assert!(resolve("mutig", true, false).dark);
    }

    /// The design values from design/build/themes.py arrive unchanged.
    #[test]
    fn design_values() {
        let m = Tokens::mutig_light();
        assert_eq!(m.accent, rgb(0x107fad));
        assert_eq!((m.side_top, m.side_bottom), (rgb(0x0c2e4e), rgb(0x081f37)));
        assert_eq!(m.accent2, rgb(0xf0663f));
        assert_eq!(Tokens::mutig_dark().accent, rgb(0x38a8db));
        let g = Tokens::glas_light();
        assert_eq!(g.main, Color32::from_rgba_unmultiplied(255, 255, 255, 0xa6));
        assert_eq!(g.accent2, rgb(0x7b5ce5));
        assert_eq!(g.r_panel, 24);
    }

    /// The generated syntax theme loads (it is parsed by syntect at runtime).
    #[test]
    fn code_theme_parses() {
        for t in [Tokens::mutig_light(), Tokens::glas_dark()] {
            let xml = t.code_theme_xml().unwrap();
            let mut md = egui_commonmark::CommonMarkCache::default();
            md.add_syntax_theme_from_bytes(t.code_theme(), xml.as_bytes()).unwrap();
        }
        assert!(Tokens::light().code_theme_xml().is_none());
    }

    /// "Transparenz: Aus" leaves no translucent surface.
    #[test]
    fn solid_glass_is_opaque() {
        for t in [Tokens::glas_light().solid(), Tokens::glas_dark().solid()] {
            for c in [t.main, t.panel, t.sidebar, t.surface, t.composer, t.modal, t.chip, t.side_active] {
                assert_eq!(c.a(), 255);
            }
        }
        assert_eq!(over(Color32::from_rgba_unmultiplied(255, 255, 255, 0), Color32::BLACK), Color32::BLACK);
        assert_eq!(over(Color32::WHITE, Color32::BLACK), Color32::WHITE);
    }

    /// Text on the accent and body text stay readable (WCAG AA, 4.5:1) in every look.
    #[test]
    fn contrast_holds() {
        fn lin(c: u8) -> f32 {
            let c = c as f32 / 255.0;
            if c <= 0.03928 { c / 12.92 } else { ((c + 0.055) / 1.055).powf(2.4) }
        }
        fn rel(c: Color32) -> f32 {
            0.2126 * lin(c.r()) + 0.7152 * lin(c.g()) + 0.0722 * lin(c.b())
        }
        fn ratio(a: Color32, b: Color32) -> f32 {
            let (x, y) = (rel(a) + 0.05, rel(b) + 0.05);
            if x > y { x / y } else { y / x }
        }
        for t in [Tokens::mutig_light(), Tokens::mutig_dark(), Tokens::glas_light(), Tokens::glas_dark(), Tokens::light(), Tokens::dark()] {
            let main = t.opaque(t.main);
            assert!(ratio(t.text, main) >= 4.5, "{:?} text", t.look);
            assert!(ratio(t.muted, main) >= 4.5, "{:?} muted {}", t.look, ratio(t.muted, main));
            assert!(ratio(t.on_accent, t.accent) >= 4.5 - 0.01, "{:?} on accent {}", t.look, ratio(t.on_accent, t.accent));
            assert!(ratio(t.side_text2, t.opaque(t.side_bottom)) >= 4.5, "{:?} sidebar", t.look);
            assert!(ratio(t.th_text, t.opaque(t.th_fill)) >= 4.5, "{:?} table head", t.look);
        }
    }
}
