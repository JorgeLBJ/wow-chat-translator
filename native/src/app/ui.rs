//! The overlay window drawing (port of `app/src/renderer/overlay.html` + `overlay.js`).
//! Only drawing and input here; state rules live in `messages`, `style`, `shared`.

use std::sync::Arc;
use std::time::{Duration, Instant};

use eframe::egui::{
    self, pos2, vec2, Align, Align2, Color32, CornerRadius, CursorIcon, FontId, Frame, Id, LayerId, Layout, Margin,
    Order, Rect, Response, RichText, Sense, Shape, Stroke, StrokeKind, TextStyle, Ui, ViewportCommand,
};
use egui_twemoji::EmojiLabel;

use crate::core::pipeline::Status;
use crate::core::prefs::Bounds;

use super::messages::{badge, can_translate, shows_translation, BadgeKind, Mode, Row, Tab};
use super::shared::Shared;
use super::style::{self, channel_label, class_color, tab_color};

const BOUNDS_SAVE_DELAY: Duration = Duration::from_millis(600);
const PAUSED_NOTE: &str = "⏸ En pausa · clic en un mensaje para traducir";
const EMPTY: &str = "Sin mensajes todavía. Aparecerán aquí en cuanto alguien escriba en el chat.";

fn white(alpha: f32) -> Color32 {
    Color32::from_white_alpha((alpha * 255.0) as u8)
}

fn with_alpha(c: Color32, alpha: u8) -> Color32 {
    Color32::from_rgba_unmultiplied(c.r(), c.g(), c.b(), alpha)
}

pub struct OverlayApp {
    shared: Arc<Shared>,
    mode: Mode,
    started_hidden: bool,
    first_frame: bool,
    scroll_to_bottom: bool,
    opacity: f32,
    last_bounds: Option<Bounds>,
    bounds_dirty_since: Option<Instant>,
}

impl OverlayApp {
    pub fn new(shared: Arc<Shared>, started_hidden: bool) -> Self {
        let opacity = shared.config().prefs.opacity;
        Self {
            shared,
            mode: Mode::Translated,
            started_hidden,
            first_frame: true,
            scroll_to_bottom: true,
            opacity,
            last_bounds: None,
            bounds_dirty_since: None,
        }
    }

    fn persist_bounds(&mut self, ctx: &egui::Context) {
        let vp = ctx.input(|i| i.viewport().clone());
        if vp.maximized == Some(true) || vp.minimized == Some(true) {
            return;
        }
        if let (Some(outer), Some(inner)) = (vp.outer_rect, vp.inner_rect) {
            let b = Bounds {
                x: Some(outer.min.x.round() as i32),
                y: Some(outer.min.y.round() as i32),
                width: Some(inner.width().round() as i32),
                height: Some(inner.height().round() as i32),
            };
            if self.last_bounds != Some(b) {
                self.last_bounds = Some(b);
                self.bounds_dirty_since = Some(Instant::now());
            }
        }
        if let Some(since) = self.bounds_dirty_since {
            if since.elapsed() >= BOUNDS_SAVE_DELAY {
                self.bounds_dirty_since = None;
                self.shared.config().overlay_bounds = self.last_bounds;
                self.shared.save_config();
            } else {
                ctx.request_repaint_after(BOUNDS_SAVE_DELAY);
            }
        }
    }

    // Title bar ------------------------------------------------------------------------

    fn title_bar(&mut self, ui: &mut Ui, maximized: bool) {
        let ctx = ui.ctx().clone();
        // Empty parts of the bar move the window; widgets added later sit on top.
        let bar = ui.interact(ui.max_rect(), Id::new("titlebar-drag"), Sense::click_and_drag());
        if bar.drag_started_by(egui::PointerButton::Primary) {
            ctx.send_viewport_cmd(ViewportCommand::StartDrag);
        }
        if bar.double_clicked() {
            ctx.send_viewport_cmd(ViewportCommand::Maximized(!maximized));
        }

        let reader = self.shared.status().reader;
        let paused = self.shared.config().prefs.translation_paused;
        let on_top = self.shared.config().prefs.always_on_top;

        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 6.0;
            ui.label(RichText::new("WoW Chat Translator").size(12.5).color(style::TEXT));
            ui.label(RichText::new(reader.title()).size(11.0).color(style::MUTED));

            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                ui.spacing_mut().item_spacing.x = 4.0;
                if icon_button(ui, "🗙", style::ICON, None, style::CLOSE_HOVER, "Ocultar (sigue en la bandeja)").clicked() {
                    self.shared.hide_by_user();
                }
                let (max_icon, max_tip) = if maximized { ("🗗", "Restaurar") } else { ("🗖", "Maximizar") };
                if icon_button(ui, max_icon, style::ICON, None, white(0.1), max_tip).clicked() {
                    ctx.send_viewport_cmd(ViewportCommand::Maximized(!maximized));
                }
                if icon_button(ui, "🗕", style::ICON, None, white(0.1), "Minimizar").clicked() {
                    ctx.send_viewport_cmd(ViewportCommand::Minimized(true));
                }
                separator(ui);
                if icon_button(ui, "⚙", style::ICON, None, white(0.1), "Ajustes").clicked() {
                    self.shared.status().transient =
                        Some(("Ajustes: próximamente".into(), Instant::now() + Duration::from_secs(3)));
                }
                if pin_button(ui, on_top).clicked() {
                    let next = !on_top;
                    self.shared.config().prefs.always_on_top = next;
                    self.shared.save_config();
                    ctx.send_viewport_cmd(ViewportCommand::WindowLevel(if next {
                        egui::WindowLevel::AlwaysOnTop
                    } else {
                        egui::WindowLevel::Normal
                    }));
                }
                let (pause_icon, pause_tip, fg, bg) = if paused {
                    ("▶", "Reanudar traducción", style::AMBER, Some(style::AMBER_BG))
                } else {
                    ("⏸", "Pausar traducción", style::ICON, None)
                };
                if icon_button(ui, pause_icon, fg, bg, white(0.1), pause_tip).clicked() {
                    self.shared.set_paused(!paused);
                }
                separator(ui);
                self.mode_toggle(ui);
            });
        });
    }

    fn mode_toggle(&mut self, ui: &mut Ui) {
        Frame::new().fill(white(0.07)).corner_radius(CornerRadius::same(6)).inner_margin(Margin::same(2)).show(ui, |ui| {
            ui.spacing_mut().item_spacing.x = 0.0;
            // Right-to-left parent: add "Original" first so "Traducido" ends up on the left.
            ui.with_layout(Layout::left_to_right(Align::Center), |ui| {
                for (label, mode) in [("Traducido", Mode::Translated), ("Original", Mode::Original)] {
                    let on = self.mode == mode;
                    let (fg, bg) = if on { (Color32::WHITE, style::ACCENT) } else { (style::SEG_TEXT, Color32::TRANSPARENT) };
                    if chip(ui, label, 11.0, fg, bg, vec2(9.0, 2.0), 5, 18.0, Sense::click()).clicked() && !on {
                        self.mode = mode;
                        self.scroll_to_bottom = true;
                    }
                }
            });
        });
    }

    // Tabs -------------------------------------------------------------------------------

    fn tabs(&mut self, ui: &mut Ui) {
        let mut clicked = None;
        {
            let model = self.shared.model();
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = 2.0;
                for tab in Tab::ALL {
                    let on = model.list.active() == tab;
                    let resp = tab_button(ui, tab.label(), on, model.list.unread_badge(tab));
                    if resp.clicked() {
                        clicked = Some(tab);
                    }
                }
            });
        }
        if let Some(tab) = clicked {
            self.shared.model().list.set_active(tab);
            self.scroll_to_bottom = true;
        }
    }

    // Messages ---------------------------------------------------------------------------

    fn messages(&mut self, ui: &mut Ui, font_size: f32) {
        let mut clicked = None;
        {
            let model = self.shared.model();
            let mut area = egui::ScrollArea::vertical().stick_to_bottom(true).auto_shrink([false, false]);
            if std::mem::take(&mut self.scroll_to_bottom) {
                area = area.vertical_scroll_offset(f32::MAX);
            }
            area.show(ui, |ui| {
                ui.style_mut().text_styles.insert(TextStyle::Body, FontId::proportional(font_size));
                ui.spacing_mut().item_spacing.y = 0.0;
                let mut any = false;
                for row in model.list.visible() {
                    any = true;
                    if message_row(ui, row, self.mode, font_size).clicked() {
                        clicked = Some(row.item.id);
                    }
                }
                if !any {
                    ui.add_space(30.0);
                    ui.vertical_centered(|ui| {
                        ui.label(RichText::new(EMPTY).size(12.0).color(style::DIM));
                    });
                }
            });
        }
        if let Some(id) = clicked {
            self.shared.translate_by_id(id);
        }
    }

    // Footer -----------------------------------------------------------------------------

    fn footer(&mut self, ui: &mut Ui) {
        let (reader, text, translator, transient) = {
            let mut s = self.shared.status();
            if s.transient.as_ref().is_some_and(|(_, until)| Instant::now() >= *until) {
                s.transient = None;
            }
            (s.reader, s.text.clone(), s.translator.clone(), s.transient.clone())
        };
        if let Some((_, until)) = &transient {
            ui.ctx().request_repaint_after(until.saturating_duration_since(Instant::now()));
        }
        let paused = self.shared.config().prefs.translation_paused;
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 10.0;
            let (dot, _) = ui.allocate_exact_size(vec2(7.0, 7.0), Sense::hover());
            ui.painter().circle_filled(dot.center(), 3.5, reader.dot());
            ui.add_space(-5.0);
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                let resp = ui.add(egui::Slider::new(&mut self.opacity, 0.4..=1.0).show_value(false));
                if resp.changed() {
                    self.shared.hwnd().set_opacity(self.opacity);
                }
                if resp.drag_stopped() || (resp.changed() && !resp.dragged()) {
                    self.shared.config().prefs.opacity = self.opacity;
                    self.shared.save_config();
                }
                ui.label(RichText::new("Opacidad").size(11.0).color(style::MUTED));
                ui.with_layout(Layout::left_to_right(Align::Center), |ui| {
                    ui.add(
                        egui::Label::new(
                            RichText::new(format!("{} · {}", reader.footer(), translator)).size(11.0).color(style::MUTED),
                        )
                        .truncate(),
                    );
                    if paused {
                        ui.add(egui::Label::new(RichText::new(PAUSED_NOTE).size(11.0).color(style::AMBER)).truncate());
                    }
                    let shown = transient.map(|(t, _)| t).unwrap_or(text);
                    if !shown.is_empty() {
                        ui.add(egui::Label::new(RichText::new(shown).size(11.0).color(style::AMBER)).truncate());
                    }
                });
            });
        });
    }

    fn chrome(&self, ctx: &egui::Context, maximized: bool) {
        let screen = ctx.content_rect();
        let painter = ctx.layer_painter(LayerId::new(Order::Foreground, Id::new("window-border")));
        painter.rect_stroke(screen, 0.0, Stroke::new(1.0_f32, style::BORDER), StrokeKind::Inside);
        if maximized {
            return;
        }
        egui::Area::new(Id::new("resize-grip"))
            .order(Order::Foreground)
            .anchor(Align2::RIGHT_BOTTOM, vec2(0.0, 0.0))
            .show(ctx, |ui| {
                let (rect, resp) = ui.allocate_exact_size(vec2(16.0, 16.0), Sense::drag());
                let stroke = Stroke::new(1.0_f32, white(0.35));
                for k in [0.5_f32, 0.75] {
                    let off = rect.width() * k;
                    ui.painter().line_segment([pos2(rect.max.x - off, rect.max.y - 1.0), pos2(rect.max.x - 1.0, rect.max.y - off)], stroke);
                }
                let resp = resp.on_hover_cursor(CursorIcon::ResizeNwSe);
                if resp.drag_started() {
                    ctx.send_viewport_cmd(ViewportCommand::BeginResize(egui::ResizeDirection::SouthEast));
                }
            });
    }
}

impl eframe::App for OverlayApp {
    fn clear_color(&self, _visuals: &egui::Visuals) -> [f32; 4] {
        style::BG.to_normalized_gamma_f32()
    }

    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        if std::mem::take(&mut self.first_frame) && self.started_hidden {
            // eframe shows the window after its first frame; --hidden keeps it in the tray.
            self.shared.hwnd().hide();
        }
        if ctx.input(|i| i.viewport().close_requested()) {
            // ✕ and Alt+F4 only hide; quitting is done from the tray menu.
            ctx.send_viewport_cmd(ViewportCommand::CancelClose);
            self.shared.hide_by_user();
        }
        self.persist_bounds(ctx);
        let maximized = ctx.input(|i| i.viewport().maximized).unwrap_or(false);
        let font_size = self.shared.config().prefs.font_size.clamp(9.0, 28.0);

        egui::TopBottomPanel::top("titlebar")
            .show_separator_line(false)
            .frame(Frame::new().fill(white(0.03)).inner_margin(Margin { left: 12, right: 6, top: 6, bottom: 6 }))
            .show(ctx, |ui| self.title_bar(ui, maximized));
        egui::TopBottomPanel::top("tabs")
            .show_separator_line(false)
            .frame(Frame::new().fill(style::BG).inner_margin(Margin { left: 10, right: 10, top: 6, bottom: 0 }))
            .show(ctx, |ui| {
                hline(ui, ui.max_rect().top() - 6.0);
                self.tabs(ui);
            });
        egui::TopBottomPanel::bottom("footer")
            .show_separator_line(false)
            .frame(Frame::new().fill(style::BG).inner_margin(Margin { left: 12, right: 24, top: 5, bottom: 6 }))
            .show(ctx, |ui| {
                hline(ui, ui.max_rect().top() - 5.0);
                self.footer(ui);
            });
        egui::CentralPanel::default().frame(Frame::new().fill(style::BG).inner_margin(Margin::same(6))).show(ctx, |ui| {
            hline(ui, ui.max_rect().top() - 6.0);
            self.messages(ui, font_size);
        });
        self.chrome(ctx, maximized);
    }
}

// Widgets ----------------------------------------------------------------------------------

/// A 1 px line across the panel at `y` (the CSS border-bottom/top lines).
fn hline(ui: &Ui, y: f32) {
    let r = ui.ctx().content_rect();
    ui.painter().clone().with_layer_id(LayerId::new(Order::Foreground, Id::new("lines"))).hline(
        r.x_range(),
        y,
        Stroke::new(1.0_f32, white(0.07)),
    );
}

fn separator(ui: &mut Ui) {
    let (rect, _) = ui.allocate_exact_size(vec2(1.0, 16.0), Sense::hover());
    ui.painter().rect_filled(rect, 0.0, white(0.12));
}

/// The title bar's square icon buttons (`.ib`).
fn icon_button(ui: &mut Ui, icon: &str, fg: Color32, bg: Option<Color32>, hover_bg: Color32, tip: &str) -> Response {
    let (rect, resp) = ui.allocate_exact_size(vec2(28.0, 24.0), Sense::click());
    let hovered = resp.hovered();
    if let Some(fill) = if hovered { Some(hover_bg) } else { bg } {
        ui.painter().rect_filled(rect, 5.0, fill);
    }
    let color = if hovered && bg.is_none() { Color32::WHITE } else { fg };
    ui.painter().text(rect.center(), Align2::CENTER_CENTER, icon, FontId::proportional(13.0), color);
    resp.on_hover_text(tip)
}

/// 📌: blue and tinted when on; grey, faded and struck through when off.
fn pin_button(ui: &mut Ui, on: bool) -> Response {
    let (rect, resp) = ui.allocate_exact_size(vec2(28.0, 24.0), Sense::click());
    let p = ui.painter();
    if on {
        p.rect_filled(rect, 5.0, with_alpha(style::ACCENT, 56));
        p.text(rect.center(), Align2::CENTER_CENTER, "📌", FontId::proportional(13.0), style::ACCENT_LIGHT);
    } else {
        if resp.hovered() {
            p.rect_filled(rect, 5.0, white(0.1));
        }
        let alpha = if resp.hovered() { 205 } else { 115 };
        p.text(rect.center(), Align2::CENTER_CENTER, "📌", FontId::proportional(13.0), Color32::from_gray(160).gamma_multiply(alpha as f32 / 255.0));
        let c = rect.center();
        p.line_segment([pos2(c.x - 8.0, c.y + 6.0), pos2(c.x + 8.0, c.y - 6.0)], Stroke::new(2.0_f32, Color32::from_rgb(0xcb, 0xd5, 0xe1)));
    }
    let tip = if on { "Siempre encima: activado" } else { "Siempre encima: desactivado" };
    resp.on_hover_text(tip)
}

/// Text in a rounded box; `height` is the allocated row height (the box is centered).
#[allow(clippy::too_many_arguments)]
fn chip(ui: &mut Ui, text: &str, size: f32, fg: Color32, bg: Color32, pad: egui::Vec2, radius: u8, height: f32, sense: Sense) -> Response {
    let galley = ui.painter().layout_no_wrap(text.to_owned(), FontId::proportional(size), fg);
    let box_size = galley.size() + pad * 2.0;
    let (rect, resp) = ui.allocate_exact_size(vec2(box_size.x, height.max(box_size.y)), sense);
    let chip_rect = Rect::from_center_size(rect.center(), box_size);
    if bg != Color32::TRANSPARENT {
        ui.painter().rect_filled(chip_rect, radius as f32, bg);
    }
    ui.painter().galley(chip_rect.center() - galley.size() / 2.0, galley, fg);
    resp
}

fn tab_button(ui: &mut Ui, label: &str, on: bool, count: Option<u32>) -> Response {
    let font = FontId::proportional(11.5);
    let label_galley = ui.painter().layout_no_wrap(label.to_owned(), font, Color32::WHITE);
    let count_galley = count.map(|n| ui.painter().layout_no_wrap(n.to_string(), FontId::proportional(9.5), Color32::WHITE));
    let count_w = count_galley.as_ref().map_or(0.0, |g| g.size().x + 10.0 + 4.0);
    let size = vec2(label_galley.size().x + count_w + 20.0, label_galley.size().y + 10.0);
    let (rect, resp) = ui.allocate_exact_size(size, Sense::click());
    let p = ui.painter();
    if on {
        p.rect_filled(rect, CornerRadius { nw: 6, ne: 6, sw: 0, se: 0 }, white(0.06));
    }
    let color = if on {
        Color32::WHITE
    } else if resp.hovered() {
        Color32::from_gray(0xdd)
    } else {
        style::TAB_IDLE
    };
    let text_pos = pos2(rect.left() + 10.0, rect.center().y - label_galley.size().y / 2.0);
    let label_w = label_galley.size().x;
    p.galley(text_pos, label_galley, color);
    if let Some(g) = count_galley {
        let pill = Rect::from_min_size(
            pos2(text_pos.x + label_w + 4.0, rect.center().y - g.size().y / 2.0),
            vec2(g.size().x + 10.0, g.size().y),
        );
        p.rect_filled(pill, 8.0, style::ACCENT);
        p.galley(pill.center() - g.size() / 2.0, g, Color32::WHITE);
    }
    resp.on_hover_cursor(CursorIcon::PointingHand)
}

/// One chat line: time, channel tag, player, badge, text (with colored emoji).
fn message_row(ui: &mut Ui, row: &Row, mode: Mode, fs: f32) -> Response {
    let item = &row.item;
    let show_t = shows_translation(item, mode);
    let clickable = can_translate(item);
    let bg_slot = ui.painter().add(Shape::Noop);
    let body_h = fs * 1.35;

    let inner = Frame::new().inner_margin(Margin::symmetric(8, 3)).show(ui, |ui| {
        ui.set_width(ui.available_width());
        ui.horizontal_top(|ui| {
            ui.spacing_mut().item_spacing.x = 6.0;
            let (time_rect, _) = ui.allocate_exact_size(vec2(34.0, body_h), Sense::hover());
            ui.painter().text(
                pos2(time_rect.left(), time_rect.center().y),
                Align2::LEFT_CENTER,
                &row.time_label,
                FontId::monospace(fs * 0.78),
                style::DIM,
            );
            ui.vertical(|ui| {
                ui.horizontal_wrapped(|ui| {
                    ui.spacing_mut().item_spacing.x = 0.0;
                    let tab_c = tab_color(item.tab);
                    chip(ui, &channel_label(&item.channel), fs * 0.8, tab_c, with_alpha(tab_c, 0x22), vec2(5.0, 0.0), 4, body_h, Sense::hover());
                    ui.add_space(5.0);
                    ui.label(RichText::new(&item.player).color(class_color(item.class.as_deref())));
                    if let Some(b) = badge(item, show_t) {
                        let (fg, bg) = match b.kind {
                            BadgeKind::Error => (style::ERROR_TEXT, style::ERROR_BG),
                            BadgeKind::Paused => (style::AMBER_LIGHT, with_alpha(Color32::from_rgb(245, 158, 11), 51)),
                            BadgeKind::Lang => (style::SEG_TEXT, white(0.1)),
                        };
                        ui.add_space(5.0);
                        chip(ui, &b.text, fs * 0.72, fg, bg, vec2(4.0, 0.0), 3, body_h, Sense::hover());
                    }
                    let text = if show_t { item.translated.as_deref().unwrap_or("") } else { &item.original };
                    let color = if item.status == Status::Pending { style::TEXT.gamma_multiply(0.55) } else { style::TEXT };
                    EmojiLabel::new(RichText::new(format!(": {text}")).color(color)).show(ui);
                });
            });
        });
    });

    let rect = inner.response.rect;
    let sense = if clickable { Sense::click() } else { Sense::hover() };
    let resp = ui.interact(rect, Id::new(("msg-row", item.id)), sense);
    if resp.hovered() {
        let fill = if clickable { with_alpha(style::ACCENT, 36) } else { white(0.05) };
        ui.painter().set(bg_slot, egui::epaint::RectShape::filled(rect, 6.0, fill));
    }
    let mut tip = Vec::new();
    if show_t {
        tip.push(item.original.clone());
    }
    if let Some(b) = badge(item, show_t).and_then(|b| b.tip) {
        tip.push(b);
    }
    if clickable && item.status == Status::Paused {
        tip.push("Clic para traducir este mensaje".into());
    }
    let resp = if clickable { resp.on_hover_cursor(CursorIcon::PointingHand) } else { resp };
    if tip.is_empty() {
        resp
    } else {
        resp.on_hover_text(tip.join("\n"))
    }
}
