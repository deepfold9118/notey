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
