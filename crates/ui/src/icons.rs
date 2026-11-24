//! Line icons from Lucide (ISC licence, `assets/fonts/Lucide-LICENSE.txt`), bundled as a subset
//! of the icon font (`assets/fonts/lucide-icons.txt` lists the icons and how to rebuild it). Each
//! icon is one glyph in the private use area, drawn in the [`crate::theme::ICONS`] family, so it
//! stays sharp at every zoom and takes the text colour.

use egui::{Color32, FontFamily, FontId, RichText};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Icon {
    /// `plus`
    Plus,
    /// `search`
    Search,
    /// `folder`
    Folder,
    /// `file-text`
    File,
    /// `table`
    Table,
    /// `sticky-note`
    Note,
    /// `message-square`
    Chat,
    /// `panel-left`
    Panel,
    /// `sliders-horizontal`
    Sliders,
    /// `sparkles`
    Sparkles,
    /// `paperclip`
    Clip,
    /// `globe`
    Globe,
    /// `chevron-down`
    ChevronDown,
    /// `chevron-right`
    ChevronRight,
    /// `chevron-left`
    ChevronLeft,
    /// `mic`
    Mic,
    /// `arrow-up`
    ArrowUp,
    /// `book-open`
    Book,
    /// `mail`
    Mail,
    /// `clipboard-check`
    Quiz,
    /// `star`
    Star,
    /// `history`
    History,
    /// `pencil`
    Pen,
    /// `chart-pie`
    Pie,
    /// `user`
    User,
    /// `users`
    Users,
    /// `droplet`
    Drop,
    /// `bell`
    Bell,
    /// `share`
    Share,
    /// `ellipsis`
    More,
    /// `copy`
    Copy,
    /// `volume-2`
    Speaker,
    /// `refresh-cw`
    Refresh,
    /// `x`
    X,
    /// `eye`
    Eye,
    /// `download`
    Download,
    /// `upload`
    Upload,
    /// `cloud`
    Cloud,
    /// `plug`
    Plug,
    /// `calendar`
    Calendar,
    /// `lightbulb`
    Bulb,
    /// `lock`
    Lock,
    /// `chart-column`
    Chart,
    /// `brain`
    Memory,
    /// `palette`
    Palette,
    /// `info`
    Info,
    /// `sun`
    Sun,
    /// `moon`
    Moon,
    /// `monitor`
    Monitor,
    /// `check`
    Check,
    /// `circle-help`
    Help,
    /// `square`
    Stop,
    /// `image`
    Image,
    /// `telescope`
    Explore,
    /// `app-window`
    App,
    /// `puzzle`
    Puzzle,
    /// `settings`
    Settings,
    /// `square-arrow-out-up-right`
    External,
    /// `trash-2`
    Trash,
    /// `command`
    Command,
    /// `link`
    Link,
    /// `square-pen`
    Edit,
    /// `pin`
    Pin,
    /// `printer`
    Printer,
    /// `list-checks`
    ListChecks,
    /// `house`
    House,
    /// `wand-sparkles`
    Wand,
}

impl Icon {
    pub const ALL: [Icon; 67] = [
        Icon::Plus,
        Icon::Search,
        Icon::Folder,
        Icon::File,
        Icon::Table,
        Icon::Note,
        Icon::Chat,
        Icon::Panel,
        Icon::Sliders,
        Icon::Sparkles,
        Icon::Clip,
        Icon::Globe,
        Icon::ChevronDown,
        Icon::ChevronRight,
        Icon::ChevronLeft,
        Icon::Mic,
        Icon::ArrowUp,
        Icon::Book,
        Icon::Mail,
        Icon::Quiz,
        Icon::Star,
        Icon::History,
        Icon::Pen,
        Icon::Pie,
        Icon::User,
        Icon::Users,
        Icon::Drop,
        Icon::Bell,
        Icon::Share,
        Icon::More,
        Icon::Copy,
        Icon::Speaker,
        Icon::Refresh,
        Icon::X,
        Icon::Eye,
        Icon::Download,
        Icon::Upload,
        Icon::Cloud,
        Icon::Plug,
        Icon::Calendar,
        Icon::Bulb,
        Icon::Lock,
        Icon::Chart,
        Icon::Memory,
        Icon::Palette,
        Icon::Info,
        Icon::Sun,
        Icon::Moon,
        Icon::Monitor,
        Icon::Check,
        Icon::Help,
        Icon::Stop,
        Icon::Image,
        Icon::Explore,
        Icon::App,
        Icon::Puzzle,
        Icon::Settings,
        Icon::External,
        Icon::Trash,
        Icon::Command,
        Icon::Link,
        Icon::Edit,
        Icon::Pin,
        Icon::Printer,
        Icon::ListChecks,
        Icon::House,
        Icon::Wand,
    ];

    pub fn char(self) -> char {
        match self {
            Icon::Plus => '\u{E13D}',
            Icon::Search => '\u{E151}',
            Icon::Folder => '\u{E0D7}',
            Icon::File => '\u{E0CC}',
            Icon::Table => '\u{E17D}',
            Icon::Note => '\u{E303}',
            Icon::Chat => '\u{E117}',
            Icon::Panel => '\u{E12A}',
            Icon::Sliders => '\u{E29A}',
            Icon::Sparkles => '\u{E412}',
            Icon::Clip => '\u{E12D}',
            Icon::Globe => '\u{E0E8}',
            Icon::ChevronDown => '\u{E06D}',
            Icon::ChevronRight => '\u{E06F}',
            Icon::ChevronLeft => '\u{E06E}',
            Icon::Mic => '\u{E118}',
            Icon::ArrowUp => '\u{E04A}',
            Icon::Book => '\u{E05F}',
            Icon::Mail => '\u{E10F}',
            Icon::Quiz => '\u{E219}',
            Icon::Star => '\u{E176}',
            Icon::History => '\u{E1F5}',
            Icon::Pen => '\u{E1F9}',
            Icon::Pie => '\u{E06B}',
            Icon::User => '\u{E19F}',
            Icon::Users => '\u{E1A4}',
            Icon::Drop => '\u{E0B4}',
            Icon::Bell => '\u{E059}',
            Icon::Share => '\u{E155}',
            Icon::More => '\u{E0B6}',
            Icon::Copy => '\u{E09E}',
            Icon::Speaker => '\u{E1AB}',
            Icon::Refresh => '\u{E145}',
            Icon::X => '\u{E1B2}',
            Icon::Eye => '\u{E0BA}',
            Icon::Download => '\u{E0B2}',
            Icon::Upload => '\u{E19E}',
            Icon::Cloud => '\u{E088}',
            Icon::Plug => '\u{E37F}',
            Icon::Calendar => '\u{E063}',
            Icon::Bulb => '\u{E1C2}',
            Icon::Lock => '\u{E10B}',
            Icon::Chart => '\u{E2A3}',
            Icon::Memory => '\u{E3C6}',
            Icon::Palette => '\u{E1DD}',
            Icon::Info => '\u{E0F9}',
            Icon::Sun => '\u{E178}',
            Icon::Moon => '\u{E11E}',
            Icon::Monitor => '\u{E11D}',
            Icon::Check => '\u{E06C}',
            Icon::Help => '\u{E082}',
            Icon::Stop => '\u{E167}',
            Icon::Image => '\u{E0F6}',
            Icon::Explore => '\u{E5C5}',
            Icon::App => '\u{E426}',
            Icon::Puzzle => '\u{E29C}',
            Icon::Settings => '\u{E154}',
            Icon::External => '\u{E5A4}',
            Icon::Trash => '\u{E18E}',
            Icon::Command => '\u{E09A}',
            Icon::Link => '\u{E102}',
            Icon::Edit => '\u{E172}',
            Icon::Pin => '\u{E259}',
            Icon::Printer => '\u{E141}',
            Icon::ListChecks => '\u{E1D0}',
            Icon::House => '\u{E0F5}',
            Icon::Wand => '\u{E357}',
        }
    }

    /// The icon as text of the given size.
    pub fn text(self, size: f32) -> RichText {
        RichText::new(self.char().to_string()).family(FontFamily::Name(crate::theme::ICONS.into())).size(size)
    }

    pub fn font(size: f32) -> FontId {
        FontId::new(size, FontFamily::Name(crate::theme::ICONS.into()))
    }

    /// Paint the icon centred on `center`.
    pub fn paint(self, painter: &egui::Painter, center: egui::Pos2, size: f32, color: Color32) {
        painter.text(center, egui::Align2::CENTER_CENTER, self.char(), Self::font(size), color);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Every icon is in the bundled font subset (rule 8 in AGENTS.md: a missing glyph would be a
    /// box in the app and a panic in the test harness).
    #[test]
    fn every_icon_is_bundled() {
        let ctx = egui::Context::default();
        ctx.set_fonts(crate::theme::font_definitions(false));
        let mut missing = Vec::new();
        let mut out = ctx.run_ui(Default::default(), |ui| {
            for i in Icon::ALL {
                if !ui.fonts_mut(|f| f.has_glyph(&Icon::font(16.0), i.char())) {
                    missing.push(i);
                }
            }
        });
        out.textures_delta.clear();
        assert!(missing.is_empty(), "missing icons {missing:?}");
    }
}
