//! The Fluent (Windows 11) dark and light palettes, and the editor theme
//! made from them.

use gpui::{rgb, rgba, Hsla};

use crate::settings::Settings;

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Palette {
    /// Title bar, tab strip, menu row, status bar.
    pub chrome: Hsla,
    /// The text editing surface.
    pub editor: Hsla,
    /// Menus, dialogs, popups.
    pub popup: Hsla,
    /// Buttons and inputs at rest.
    pub control: Hsla,
    pub control_hover: Hsla,
    pub control_active: Hsla,
    /// Hairline borders.
    pub stroke: Hsla,
    pub text: Hsla,
    pub text_weak: Hsla,
    /// Fluent accent (focus rings, toggles, links).
    pub accent: Hsla,
    /// Text selection background.
    pub selection: Hsla,
    /// Text cursor; always contrasts with `editor`.
    pub caret: Hsla,
    /// Tab hover wash.
    pub tab_hover: Hsla,
    /// Close-button hover (caption bar).
    pub close_hover: Hsla,
    /// Find matches in the editor.
    pub find_match: Hsla,
    /// The selected find match.
    pub find_match_selected: Hsla,
    /// The named colors (`COLOR_NAMES`): a VS Code theme's terminal colors.
    pub named: [Hsla; 6],
    /// Problem underlines and the problem bar: errors and hints.
    pub error: Hsla,
    pub warning: Hsla,
}

/// Colors that plugins can name (tab colors, swatches), so that they follow
/// the theme: the theme's terminal colors.
pub const COLOR_NAMES: [&str; 6] = ["red", "yellow", "green", "cyan", "blue", "magenta"];
/// The VS Code theme keys of `COLOR_NAMES`.
pub const COLOR_KEYS: [&str; 6] = [
    "terminal.ansiRed",
    "terminal.ansiYellow",
    "terminal.ansiGreen",
    "terminal.ansiCyan",
    "terminal.ansiBlue",
    "terminal.ansiMagenta",
];

impl Palette {
    /// A color by name (`COLOR_NAMES`, from the theme) or as "#rrggbb".
    pub fn color(&self, s: &str) -> Option<Hsla> {
        match COLOR_NAMES.iter().position(|n| n.eq_ignore_ascii_case(s.trim())) {
            Some(i) => Some(self.named[i]),
            None => crate::plugins::parse_color(s),
        }
    }
}

/// Whether `s` is a color that `Palette::color` knows.
pub fn is_color(s: &str) -> bool {
    palette(true).color(s).is_some()
}

pub fn palette(dark: bool) -> Palette {
    if dark {
        Palette {
            chrome: rgb(0x202020).into(),
            editor: rgb(0x282828).into(),
            popup: rgb(0x2C2C2C).into(),
            control: rgb(0x2D2D2D).into(),
            control_hover: rgb(0x353535).into(),
            control_active: rgb(0x3A3A3A).into(),
            stroke: rgb(0x3A3A3A).into(),
            text: rgb(0xF0F0F0).into(),
            text_weak: rgb(0x9D9D9D).into(),
            accent: rgb(0x60CDFF).into(),
            selection: rgb(0x265C8C).into(),
            caret: rgb(0x60CDFF).into(),
            tab_hover: rgb(0x2A2A2A).into(),
            close_hover: rgb(0xC42B1C).into(),
            find_match: rgba(0xC9A22766).into(),
            find_match_selected: rgba(0xE8892FAA).into(),
            // VS Code's Dark+ terminal colors
            named: [0xcd3131, 0xe5e510, 0x0dbc79, 0x11a8cd, 0x2472c8, 0xbc3fbc].map(|c| rgb(c).into()),
            // VS Code's Dark+ editorError / editorWarning foregrounds
            error: rgb(0xF14C4C).into(),
            warning: rgb(0xCCA700).into(),
        }
    } else {
        Palette {
            chrome: rgb(0xF3F3F3).into(),
            editor: rgb(0xFFFFFF).into(),
            popup: rgb(0xF9F9F9).into(),
            control: rgb(0xFBFBFB).into(),
            control_hover: rgb(0xF0F0F0).into(),
            control_active: rgb(0xE5E5E5).into(),
            stroke: rgb(0xE0E0E0).into(),
            text: rgb(0x1B1B1B).into(),
            text_weak: rgb(0x606060).into(),
            accent: rgb(0x0067C0).into(),
            selection: rgb(0xA8CEF1).into(),
            caret: rgb(0x0067C0).into(),
            tab_hover: rgb(0xEAEAEA).into(),
            close_hover: rgb(0xC42B1C).into(),
            find_match: rgba(0xFFD54F99).into(),
            find_match_selected: rgba(0xFF9E40AA).into(),
            // VS Code's Light+ terminal colors
            named: [0xcd3131, 0x949800, 0x00bc00, 0x0598bc, 0x0451a5, 0xbc05bc].map(|c| rgb(c).into()),
            // VS Code's Light+ editorError / editorWarning foregrounds
            error: rgb(0xE51400).into(),
            warning: rgb(0xBF8803).into(),
        }
    }
}

/// The editor theme for the palette and the font settings.
pub fn editor_theme(p: &Palette, settings: &Settings) -> codemirror_view::Theme {
    let base = if settings.dark { codemirror_view::Theme::dark() } else { codemirror_view::Theme::light() };
    let mut unfocused = p.selection;
    unfocused.a = 0.6;
    codemirror_view::Theme {
        font_family: settings.editor_font.clone().into(),
        font_features: if settings.ligatures { gpui::FontFeatures::default() } else { gpui::FontFeatures::disable_ligatures() },
        font_size: settings.zoomed_font_size(),
        line_height: 1.4 * settings.line_height,
        text: p.text,
        background: p.editor,
        selection: p.selection,
        selection_unfocused: unfocused,
        caret: p.caret,
        active_line: if settings.dark { rgba(0xffffff08).into() } else { rgba(0x00000008).into() },
        gutter_background: p.editor,
        gutter_text: p.text_weak,
        gutter_active_text: p.text,
        padding_left: 8.0,
        ..base
    }
}
