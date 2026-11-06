//! Drawing helpers shared by the overlay and the settings window: the system font,
//! text centered in a box, `.btn` buttons, `.seg` segmented controls and dragging a
//! window by its title bar. Every helper paints its own box and centers its text
//! with the same rule, so all controls line up the same way.

use std::path::PathBuf;
use std::sync::Arc;

use eframe::egui::{
    self, vec2, Color32, CornerRadius, CursorIcon, FontData, FontDefinitions, FontFamily, FontId, Id, Painter, Pos2, Rect,
    Response, Sense, Stroke, Ui, ViewportCommand,
};

use super::style;

/// Font family of the 600-weight texts (title, player names, setting titles).
pub const SEMIBOLD: &str = "semibold";
/// Text typed in inputs (`#eee` in the Electron CSS).
pub const INPUT_TEXT: Color32 = Color32::from_gray(0xee);

const DOUBLE_CLICK_S: f64 = 0.45;

/// Segoe UI (what the Electron windows use) first, egui's fonts after it for the
/// emoji and icon glyphs, Segoe UI Symbol last. Missing files just keep egui's defaults.
pub fn install_fonts(ctx: &egui::Context) {
    let dir = std::env::var_os("WINDIR").map(PathBuf::from).unwrap_or_else(|| PathBuf::from(r"C:\Windows")).join("Fonts");
    let read = |name: &str| std::fs::read(dir.join(name)).ok().map(|b| Arc::new(FontData::from_owned(b)));
    let mut fonts = FontDefinitions::default();
    let mut regular = fonts.families.get(&FontFamily::Proportional).cloned().unwrap_or_default();
    if let Some(data) = read("segoeui.ttf") {
        fonts.font_data.insert("segoe-ui".into(), data);
        regular.insert(0, "segoe-ui".into());
    }
    if let Some(data) = read("seguisym.ttf") {
        fonts.font_data.insert("segoe-ui-symbol".into(), data);
        regular.push("segoe-ui-symbol".into());
    }
    let mut semibold = regular.clone();
    if let Some(data) = read("seguisb.ttf") {
        fonts.font_data.insert("segoe-ui-semibold".into(), data);
        semibold.insert(0, "segoe-ui-semibold".into());
    }
    fonts.families.insert(FontFamily::Proportional, regular);
    fonts.families.insert(FontFamily::Name(SEMIBOLD.into()), semibold);
    ctx.set_fonts(fonts);
}

pub fn semibold_family() -> FontFamily {
    FontFamily::Name(SEMIBOLD.into())
}

pub fn semibold(size: f32) -> FontId {
    FontId::new(size, semibold_family())
}

fn white(alpha: f32) -> Color32 {
    Color32::from_white_alpha((alpha * 255.0) as u8)
}

/// Inputs and drop-downs like the Electron fields: dark fill, thin light border,
/// 6 px radius, bright text and a blue border while focused. Applies to `ui` and its children.
pub fn field_style(ui: &mut Ui) {
    let v = ui.visuals_mut();
    for w in [&mut v.widgets.inactive, &mut v.widgets.hovered, &mut v.widgets.active, &mut v.widgets.open] {
        w.weak_bg_fill = white(0.06);
        w.bg_fill = white(0.06);
        w.bg_stroke = Stroke::new(1.0_f32, white(0.12));
        w.corner_radius = CornerRadius::same(6);
        w.fg_stroke.color = INPUT_TEXT;
        w.expansion = 0.0;
    }
    v.widgets.hovered.bg_stroke = Stroke::new(1.0_f32, white(0.22));
    v.widgets.hovered.weak_bg_fill = white(0.1);
    v.selection.stroke = Stroke::new(1.0_f32, style::ACCENT);
    v.extreme_bg_color = white(0.06);
    ui.spacing_mut().button_padding = vec2(8.0, 5.0);
}

/// Paints one line of text centered in `rect` (its line box, like CSS flex centering).
pub fn text_in(painter: &Painter, rect: Rect, text: &str, font: FontId, color: Color32) {
    let galley = painter.layout_no_wrap(text.to_owned(), font, color);
    let pos = rect.center() - galley.size() / 2.0;
    painter.galley(Pos2::new(pos.x.round(), pos.y.round()), galley, color);
}

/// Width of one line of text.
pub fn text_width(ui: &Ui, text: &str, font: &FontId) -> f32 {
    ui.painter().layout_no_wrap(text.to_owned(), font.clone(), Color32::WHITE).size().x
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum ButtonKind {
    Normal,
    Primary,
    Danger,
}

/// `.btn`: 12 px text, 5×10 px padding, 6 px radius, no border.
pub fn button(ui: &mut Ui, text: &str, kind: ButtonKind, enabled: bool) -> Response {
    let font = FontId::proportional(12.0);
    let size = vec2(text_width(ui, text, &font) + 20.0, 26.0);
    let (rect, resp) = ui.allocate_exact_size(size, if enabled { Sense::click() } else { Sense::hover() });
    let hovered = enabled && resp.hovered();
    let (bg, fg) = match (kind, hovered) {
        (ButtonKind::Primary, false) => (style::ACCENT, Color32::WHITE),
        (ButtonKind::Primary, true) => (Color32::from_rgb(0x25, 0x63, 0xeb), Color32::WHITE),
        (ButtonKind::Danger, true) => (style::CLOSE_HOVER, Color32::WHITE),
        (_, false) => (white(0.08), Color32::from_gray(0xdd)),
        (_, true) => (white(0.14), Color32::from_gray(0xdd)),
    };
    let fade = if enabled { 1.0 } else { 0.5 };
    let p = ui.painter();
    p.rect_filled(rect, 6.0, bg.gamma_multiply(fade));
    text_in(p, rect, text, font, fg.gamma_multiply(fade));
    if enabled {
        resp.on_hover_cursor(CursorIcon::PointingHand)
    } else {
        resp
    }
}

/// `.seg`: a dark rounded track; the active segment is a blue pill with white text.
/// Returns the clicked value (if any) and whether the pointer is over the control.
pub fn segmented<T: Copy + PartialEq>(
    ui: &mut Ui,
    id: Id,
    items: &[(&str, T)],
    current: T,
    pad_x: f32,
    seg_h: f32,
) -> (Option<T>, bool) {
    let font = FontId::proportional(11.0);
    let widths: Vec<f32> = items.iter().map(|(label, _)| text_width(ui, label, &font) + pad_x * 2.0).collect();
    let size = vec2(widths.iter().sum::<f32>() + 4.0, seg_h + 4.0);
    let (track, track_resp) = ui.allocate_exact_size(size, Sense::hover());
    ui.painter().rect_filled(track, 6.0, white(0.07));
    let mut x = track.left() + 2.0;
    let mut clicked = None;
    for (i, ((label, value), w)) in items.iter().zip(&widths).enumerate() {
        let rect = Rect::from_min_size(Pos2::new(x, track.top() + 2.0), vec2(*w, seg_h));
        x += w;
        let resp = ui.interact(rect, id.with(i), Sense::click()).on_hover_cursor(CursorIcon::PointingHand);
        let on = *value == current;
        if on {
            ui.painter().rect_filled(rect, CornerRadius::same(5), style::ACCENT);
        }
        let fg = if on {
            Color32::WHITE
        } else if resp.hovered() {
            Color32::from_gray(0xdd)
        } else {
            style::SEG_TEXT
        };
        text_in(ui.painter(), rect, label, font.clone(), fg);
        if resp.clicked() && !on {
            clicked = Some(*value);
        }
    }
    (clicked, track_resp.contains_pointer())
}

/// A tab with rounded top corners (`.tab` / `.stab`); `extra_w` leaves room for a badge.
pub fn tab_rect(ui: &mut Ui, label: &str, font: &FontId, extra_w: f32) -> (Rect, Response) {
    let size = vec2(text_width(ui, label, font) + extra_w + 20.0, (font.size * 1.33).round() + 10.0);
    ui.allocate_exact_size(size, Sense::click())
}

pub fn tab_fill(ui: &Ui, rect: Rect, alpha: f32) {
    ui.painter().rect_filled(rect, CornerRadius { nw: 6, ne: 6, sw: 0, se: 0 }, white(alpha));
}

pub fn tab_text_color(on: bool, hovered: bool) -> Color32 {
    if on {
        Color32::WHITE
    } else if hovered {
        Color32::from_gray(0xdd)
    } else {
        style::TAB_IDLE
    }
}

/// A title bar like `-webkit-app-region: drag`: pressing anywhere on `bar` that is not
/// a control moves the window; a double click toggles maximize when `maximized` is
/// given. Call it after drawing the bar, with `over_control` from its controls.
pub fn caption_drag(ctx: &egui::Context, id: Id, bar: Rect, over_control: bool, maximized: Option<bool>) {
    let (pressed, pos, now) = ctx.input(|i| (i.pointer.primary_pressed(), i.pointer.interact_pos(), i.time));
    let Some(pos) = pos.filter(|p| pressed && !over_control && bar.contains(*p)) else { return };
    let last: Option<(f64, Pos2)> = ctx.data(|d| d.get_temp(id));
    let double = last.is_some_and(|(t, p)| now - t < DOUBLE_CLICK_S && p.distance(pos) < 6.0);
    match (double, maximized) {
        (true, Some(max)) => {
            ctx.data_mut(|d| d.remove::<(f64, Pos2)>(id));
            ctx.send_viewport_cmd(ViewportCommand::Maximized(!max));
        }
        _ => {
            ctx.data_mut(|d| d.insert_temp(id, (now, pos)));
            ctx.send_viewport_cmd(ViewportCommand::StartDrag);
        }
    }
}
