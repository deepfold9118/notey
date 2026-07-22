//! Fluent (WinUI-inspired) theming for Notey — dark and light variants.

use eframe::egui::{
    self, Color32, CornerRadius, Shadow, Stroke, Visuals,
};

#[derive(Clone, Copy)]
pub struct Palette {
    /// Title bar, tab strip, menu row, status bar.
    pub chrome: Color32,
    /// The text editing surface.
    pub editor: Color32,
    /// Menus, dialogs, popups.
    pub popup: Color32,
    /// Buttons and inputs at rest.
    pub control: Color32,
    pub control_hover: Color32,
    pub control_active: Color32,
    /// Hairline borders.
    pub stroke: Color32,
    pub text: Color32,
    pub text_weak: Color32,
    /// Fluent accent (focus rings, toggles, links).
    pub accent: Color32,
    /// Text selection background.
    pub selection: Color32,
    /// Tab hover wash.
    pub tab_hover: Color32,
    /// Close-button hover (caption bar).
    pub close_hover: Color32,
}

pub fn palette(dark: bool) -> Palette {
    if dark {
        Palette {
            chrome: Color32::from_rgb(0x20, 0x20, 0x20),
            editor: Color32::from_rgb(0x28, 0x28, 0x28),
            popup: Color32::from_rgb(0x2C, 0x2C, 0x2C),
            control: Color32::from_rgb(0x2D, 0x2D, 0x2D),
            control_hover: Color32::from_rgb(0x35, 0x35, 0x35),
            control_active: Color32::from_rgb(0x3A, 0x3A, 0x3A),
            stroke: Color32::from_rgb(0x3A, 0x3A, 0x3A),
            text: Color32::from_rgb(0xF0, 0xF0, 0xF0),
            text_weak: Color32::from_rgb(0x9D, 0x9D, 0x9D),
            accent: Color32::from_rgb(0x60, 0xCD, 0xFF),
            selection: Color32::from_rgb(0x26, 0x5C, 0x8C),
            tab_hover: Color32::from_rgb(0x2A, 0x2A, 0x2A),
            close_hover: Color32::from_rgb(0xC4, 0x2B, 0x1C),
        }
    } else {
        Palette {
            chrome: Color32::from_rgb(0xF3, 0xF3, 0xF3),
            editor: Color32::from_rgb(0xFF, 0xFF, 0xFF),
            popup: Color32::from_rgb(0xF9, 0xF9, 0xF9),
            control: Color32::from_rgb(0xFB, 0xFB, 0xFB),
            control_hover: Color32::from_rgb(0xF0, 0xF0, 0xF0),
            control_active: Color32::from_rgb(0xE5, 0xE5, 0xE5),
            stroke: Color32::from_rgb(0xE0, 0xE0, 0xE0),
            text: Color32::from_rgb(0x1B, 0x1B, 0x1B),
            text_weak: Color32::from_rgb(0x60, 0x60, 0x60),
            accent: Color32::from_rgb(0x00, 0x67, 0xC0),
            selection: Color32::from_rgb(0xA8, 0xCE, 0xF1),
            tab_hover: Color32::from_rgb(0xEA, 0xEA, 0xEA),
            close_hover: Color32::from_rgb(0xC4, 0x2B, 0x1C),
        }
    }
}

/// Apply the Fluent look to every egui widget.
pub fn apply_fluent(ctx: &egui::Context, dark: bool) {
    let p = palette(dark);
    let mut v = if dark {
        Visuals::dark()
    } else {
        Visuals::light()
    };

    v.panel_fill = p.chrome;
    v.window_fill = p.popup;
    v.extreme_bg_color = if dark {
        Color32::from_rgb(0x1F, 0x1F, 0x1F)
    } else {
        Color32::from_rgb(0xFF, 0xFF, 0xFF)
    };
    v.faint_bg_color = p.control;

    v.override_text_color = Some(p.text);
    v.selection.bg_fill = p.selection;
    v.selection.stroke = Stroke::new(1.0, p.accent);
    v.hyperlink_color = p.accent;

    let r4 = CornerRadius::same(4);
    v.widgets.noninteractive.bg_fill = p.chrome;
    v.widgets.noninteractive.weak_bg_fill = p.chrome;
    v.widgets.noninteractive.bg_stroke = Stroke::new(1.0, p.stroke);
    v.widgets.noninteractive.fg_stroke = Stroke::new(1.0, p.text);
    v.widgets.noninteractive.corner_radius = r4;

    v.widgets.inactive.bg_fill = p.control;
    v.widgets.inactive.weak_bg_fill = p.control;
    v.widgets.inactive.bg_stroke = Stroke::new(1.0, p.stroke);
    v.widgets.inactive.fg_stroke = Stroke::new(1.0, p.text);
    v.widgets.inactive.corner_radius = r4;

    v.widgets.hovered.bg_fill = p.control_hover;
    v.widgets.hovered.weak_bg_fill = p.control_hover;
    v.widgets.hovered.bg_stroke = Stroke::new(1.0, p.stroke);
    v.widgets.hovered.fg_stroke = Stroke::new(1.0, p.text);
    v.widgets.hovered.corner_radius = r4;

    v.widgets.active.bg_fill = p.control_active;
    v.widgets.active.weak_bg_fill = p.control_active;
    v.widgets.active.bg_stroke = Stroke::new(1.0, p.stroke);
    v.widgets.active.fg_stroke = Stroke::new(1.0, p.text);
    v.widgets.active.corner_radius = r4;

    v.widgets.open.bg_fill = p.control_active;
    v.widgets.open.weak_bg_fill = p.control_active;
    v.widgets.open.bg_stroke = Stroke::new(1.0, p.stroke);
    v.widgets.open.fg_stroke = Stroke::new(1.0, p.text);
    v.widgets.open.corner_radius = r4;

    v.window_corner_radius = CornerRadius::same(8);
    v.window_stroke = Stroke::new(1.0, p.stroke);
    v.window_shadow = Shadow {
        offset: [0, 8],
        blur: 32,
        spread: 0,
        color: Color32::from_black_alpha(if dark { 96 } else { 48 }),
    };
    v.menu_corner_radius = CornerRadius::same(8);
    v.popup_shadow = Shadow {
        offset: [0, 4],
        blur: 16,
        spread: 0,
        color: Color32::from_black_alpha(if dark { 80 } else { 40 }),
    };

    v.slider_trailing_fill = true;
    v.striped = false;

    let theme = if dark {
        egui::Theme::Dark
    } else {
        egui::Theme::Light
    };
    ctx.set_theme(theme);
    ctx.style_mut_of(theme, |style| {
        style.visuals = v;
        style.spacing.item_spacing = egui::vec2(8.0, 6.0);
        style.spacing.button_padding = egui::vec2(12.0, 5.0);
        style.spacing.menu_margin = egui::Margin::same(6);
        style.spacing.window_margin = egui::Margin::same(14);
        style.spacing.scroll = egui::style::ScrollStyle {
            floating: true,
            bar_width: 8.0,
            floating_allocated_width: 8.0,
            ..egui::style::ScrollStyle::floating()
        };
    });
}
