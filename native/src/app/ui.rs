//! The overlay window drawing (port of `app/src/renderer/overlay.html` + `overlay.js`).
//! Only drawing and input here; state rules live in `messages`, `style`, `shared`.

use std::cell::Cell;
use std::collections::HashSet;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use eframe::egui::{
    self, pos2, text::LayoutJob, vec2, Align, Align2, Color32, CursorIcon, FontId, Frame, Id, Key, LayerId, Layout,
    Margin, Order, Rect, Response, RichText, Sense, Shape, Stroke, StrokeKind, TextFormat, TextStyle, Ui, ViewportCommand,
};
use egui_twemoji::EmojiLabel;

use crate::core::channels::{log_highlights, log_label};
use crate::core::item_tips::{link_segments, ItemTooltip, ItemTooltipCache, Segment, TipLine};
use crate::core::links::LinkRef;
use crate::core::pipeline::{OverlayItem, Status, UiEvent};
use crate::core::prefs::{Bounds, ItemTooltips};
use crate::core::wowhead::{panel_view, PanelView, WowheadCache, WowheadState};

use super::compose::{self, copied_message, failed_message, translate_outgoing, OUT_LANGS};
use super::messages::{badge, can_translate, flip_label, panel_goes_above, row_mode, PANEL_GAP, shows_translation, BadgeKind, Mode, Row, Tab};
use super::settings::{self, SettingsState};
use super::shared::Shared;
use super::style::{self, channel_label, class_color, item_quality_color, log_highlight_color, log_text_color, tab_color};
use super::{widgets, win};

const BOUNDS_SAVE_DELAY: Duration = Duration::from_millis(600);
const PAUSED_NOTE: &str = "⏸ En pausa · clic en un mensaje para traducir";
const COMPOSE_ID: &str = "compose-input";
const COPIED: Color32 = Color32::from_rgb(0x86, 0xef, 0xac);
const FAILED: Color32 = Color32::from_rgb(0xfc, 0xa5, 0xa5);
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
    settings: Arc<Mutex<SettingsState>>,
    icon: Option<Arc<egui::IconData>>,
    compose: Compose,
    /// Rows the user flipped from the context menu ("Ver original" / "Ver traducción"):
    /// they show the other text than the global Traducido / Original mode.
    flipped: HashSet<u64>,
    /// `WCT_DEV_STATE=menu`: draws the row context menu on this message (screenshots).
    dev_menu: Option<u64>,
    /// `WCT_DEV_STATE=itemtip`: draws the item panel of the sample item link open.
    dev_itemtip: bool,
    /// `WCT_DEV_STATE=select`: drags the mouse over the first message with synthetic
    /// input events to select its text (screenshots); the step of that drag.
    dev_select: Option<u32>,
    /// Rect of the first message row, as drawn last frame (for `dev_select`).
    first_row_rect: Option<Rect>,
    /// Display affinity applied to the window (`hideFromCapture`), once known.
    capture_hidden: Option<bool>,
}

/// The compose box: text, EN/ES, the last result line and the running translation.
struct Compose {
    text: String,
    lang: &'static str,
    preview: Option<(String, Color32)>,
    pending: Arc<Mutex<Option<Result<String, String>>>>,
    busy: bool,
}

impl Default for Compose {
    fn default() -> Self {
        Self { text: String::new(), lang: "en", preview: None, pending: Arc::new(Mutex::new(None)), busy: false }
    }
}

impl OverlayApp {
    pub fn new(shared: Arc<Shared>, started_hidden: bool, icon: Option<Arc<egui::IconData>>) -> Self {
        let opacity = shared.config().prefs.opacity;
        let settings = Arc::new(Mutex::new(SettingsState::new(&shared)));
        Self {
            settings,
            icon,
            compose: Compose::default(),
            shared,
            mode: Mode::Translated,
            started_hidden,
            first_frame: true,
            scroll_to_bottom: true,
            opacity,
            last_bounds: None,
            bounds_dirty_since: None,
            flipped: HashSet::new(),
            dev_menu: None,
            dev_itemtip: false,
            dev_select: None,
            first_row_rect: None,
            capture_hidden: None,
        }
    }

    /// Developer screenshots only (`WCT_DEV_CAPTURE=1`): `WCT_DEV_STATE` is a comma list of
    /// `hover`, `compose` (typed text + the "Copiado" line) and `settings=translate|window|general`.
    fn apply_dev_state(&mut self, ctx: &egui::Context) {
        if !win::dev_capture() {
            return;
        }
        let spec = std::env::var("WCT_DEV_STATE").unwrap_or_default();
        for part in spec.split(',').map(str::trim) {
            match part.split_once('=') {
                Some(("settings", pane)) => settings::dev_open(ctx, &self.shared, &self.settings, pane),
                _ if part == "menu" => self.dev_menu = Some(900_001),
                _ if part == "itemtip" => self.dev_itemtip = true,
                _ if part == "select" => self.dev_select = Some(0),
                _ if part == "addon" => super::addon_panel::dev_open(&self.shared),
                _ if part == "sample" => {
                    let mut model = self.shared.model();
                    let samples = [
                        ("General", Some("general"), "PALADIN", "Rj", "metro lineal, wer kommt mit?", "metro lineal, ¿quién viene?", "de"),
                        ("Trade", Some("trade"), "MAGE", "Slickback", "TANK LFM and HEALER SFK!", "¡TANQUE LFM y SANADOR SFK!", "en"),
                        ("Guild", Some("guild"), "PRIEST", "Lunaria", "gg everyone, see you at raid tonight 😄", "gg a todos, nos vemos en la banda esta noche 😄", "en"),
                        ("Party", Some("group"), "WARRIOR", "Thorgar", "wait for mana before the next pull pls", "esperad a tener maná antes del siguiente pull, por favor", "en"),
                        ("Whisper", Some("whisper"), "ROGUE", "Nyxie", "salut ! tu peux m'aider pour la quête ?", "¡hola! ¿me puedes ayudar con la misión?", "fr"),
                        ("General", Some("general"), "DRUID", "Folhaverde", "alguém sabe onde fica o treinador de herbalismo?", "¿alguien sabe dónde está el instructor de herboristería?", "pt"),
                        ("Raid", Some("group"), "SHAMAN", "Kazrak", "bloodlust on pull, everyone stack behind the boss", "ansia de sangre al empezar, todos juntos detrás del jefe", "en"),
                        ("Trade", Some("trade"), "HUNTER", "Arrowind", "WTS [Thunderfury, Blessed Blade of the Windseeker] 9999g, pst", "Vendo [Thunderfury, Blessed Blade of the Windseeker] 9999g, susúrrame", "en"),
                    ];
                    for (i, (channel, tab, class, player, original, translated, lang)) in samples.into_iter().enumerate() {
                        let item = OverlayItem {
                            id: 900_000 + i as u64,
                            time: 0,
                            channel: channel.into(),
                            tab,
                            class: Some(class.into()),
                            player: player.into(),
                            original: original.into(),
                            translated: Some(translated.into()),
                            lang: Some(lang.into()),
                            status: Status::Done,
                            provider: Some("google".into()),
                            fallback_reason: None,
                            error: None,
                            links: if i == 7 { vec![dev_item_link()] } else { Vec::new() },
                        };
                        model.list.apply(UiEvent::Add(item), &|_| "15:38".into());
                    }
                    // Its tooltip, as the addon would have sent it.
                    model.tips.insert(DEV_ITEM_ID, dev_item_tooltip());
                    // And as Wowhead would answer, so screenshots never go online.
                    model.wowhead.insert_ready(DEV_ITEM_ID, dev_item_tooltip());
                    // With the log option on, a few Registro lines too.
                    if self.shared.config().prefs.show_log {
                        let log = [
                            ("*Loot", "", "Recibes botín: [Mineral de cobre]x2."),
                            ("*Money", "", "Recibes botín: 1 de plata, 25 de cobre."),
                            ("*Reputation", "", "Reputación con Ventormenta aumentada en 25."),
                            ("*NPC", "Guardia de Ventormenta", "¡Mantente alerta, ciudadano!"),
                            ("*System", "", "Ya no estás ausente."),
                            ("*System", "", "Brannok has gone offline."),
                            ("*System", "", "[Selwyn] has come online."),
                            ("*Emote", "", "You cheer at Selwyn."),
                            ("*Addon", "", "Zygor Guides Viewer: 377 guides are loaded."),
                            ("*Addon", "", "Zygor Guides Viewer: 377 guides are loaded."),
                            ("*Addon", "", "Zygor Guides Viewer: 377 guides are loaded."),
                        ];
                        for (i, (channel, player, text)) in log.into_iter().enumerate() {
                            let item = OverlayItem {
                                id: 900_100 + i as u64,
                                time: 0,
                                channel: channel.into(),
                                tab: Some("log"),
                                class: None,
                                player: player.into(),
                                original: text.into(),
                                translated: None,
                                lang: None,
                                status: Status::Done,
                                provider: None,
                                fallback_reason: None,
                                error: None,
                                links: Vec::new(),
                            };
                            model.list.apply(UiEvent::Add(item), &|_| "15:39".into());
                        }
                    }
                }
                _ if part == "compose" => {
                    self.compose.text = "hola, ¿qué tal?".into();
                    self.compose.preview = Some((copied_message("hello, how are you?"), COPIED));
                }
                _ => {}
            }
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

    /// Draws the bar's contents; true while the pointer is over one of its controls
    /// (everywhere else the bar moves the window, see `widgets::caption_drag`).
    fn title_bar(&mut self, ui: &mut Ui, maximized: bool) -> bool {
        let ctx = ui.ctx().clone();
        let reader = self.shared.status().reader;
        let paused = self.shared.config().prefs.translation_paused;
        let on_top = self.shared.config().prefs.always_on_top;
        let narrow = ui.available_width() < 400.0;
        let mut hot = false;

        ui.allocate_ui_with_layout(vec2(ui.available_width(), 24.0), Layout::right_to_left(Align::Center), |ui| {
            ui.spacing_mut().item_spacing.x = 2.0;
            let r = icon_button(ui, "✕", style::ICON, None, style::CLOSE_HOVER, "Ocultar (sigue en la bandeja)");
            hot |= r.contains_pointer();
            if r.clicked() {
                self.shared.hide_by_user();
            }
            let (max_icon, max_tip) = if maximized { ("❐", "Restaurar") } else { ("☐", "Maximizar") };
            let r = icon_button(ui, max_icon, style::ICON, None, white(0.1), max_tip);
            hot |= r.contains_pointer();
            if r.clicked() {
                ctx.send_viewport_cmd(ViewportCommand::Maximized(!maximized));
            }
            let r = icon_button(ui, "—", style::ICON, None, white(0.1), "Minimizar");
            hot |= r.contains_pointer();
            if r.clicked() {
                ctx.send_viewport_cmd(ViewportCommand::Minimized(true));
            }
            separator(ui);
            let r = icon_button(ui, "⚙", style::ICON, None, white(0.1), "Ajustes");
            hot |= r.contains_pointer();
            if r.clicked() {
                settings::open(&ctx, &self.shared, &self.settings);
            }
            let r = pin_button(ui, on_top);
            hot |= r.contains_pointer();
            if r.clicked() {
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
            let r = icon_button(ui, pause_icon, fg, bg, white(0.1), pause_tip);
            hot |= r.contains_pointer();
            if r.clicked() {
                self.shared.set_paused(!paused);
            }
            separator(ui);
            hot |= self.mode_toggle(ui, narrow);
            ui.add_space(4.0);
            ui.with_layout(Layout::left_to_right(Align::Center), |ui| {
                // `.t` + `<small>`: one line that ellipsizes as a whole when space runs out.
                let mut job = LayoutJob::default();
                job.append("WoW Chat Translator", 0.0, TextFormat::simple(widgets::semibold(12.5), style::TEXT));
                job.append(reader.title(), 6.0, TextFormat::simple(FontId::proportional(11.0), style::MUTED));
                ui.add(egui::Label::new(job).truncate().sense(Sense::hover()));
            });
        });
        hot
    }

    /// `.seg` Traducido / Original; true while the pointer is over it.
    fn mode_toggle(&mut self, ui: &mut Ui, narrow: bool) -> bool {
        let items = [("Traducido", Mode::Translated), ("Original", Mode::Original)];
        let pad = if narrow { 6.0 } else { 9.0 };
        let (picked, over) = widgets::segmented(ui, Id::new("mode-seg"), &items, self.mode, pad, 20.0);
        if let Some(mode) = picked {
            self.mode = mode;
            self.flipped.clear(); // the global switch sets every row again
            self.scroll_to_bottom = true;
        }
        over
    }

    // Tabs -------------------------------------------------------------------------------

    fn tabs(&mut self, ui: &mut Ui) {
        let mut clicked = None;
        let show_log = self.shared.config().prefs.show_log;
        {
            let model = self.shared.model();
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = 2.0;
                for tab in Tab::ALL {
                    if tab == Tab::Log && !show_log {
                        continue;
                    }
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
        let mut flip = None;
        let wanted = Cell::new(None);
        {
            let item_tips = self.shared.config().prefs.item_tooltips;
            let model = self.shared.model();
            let tips = (item_tips != ItemTooltips::Off)
                .then_some(TipSources { mode: item_tips, addon: &model.tips, wowhead: &model.wowhead, wanted: &wanted });
            // Dragging over the list selects text (like Electron), it does not scroll it.
            let mut area = egui::ScrollArea::vertical()
                .stick_to_bottom(true)
                .auto_shrink([false, false])
                .scroll_source(egui::containers::scroll_area::ScrollSource { drag: false, ..Default::default() });
            if std::mem::take(&mut self.scroll_to_bottom) {
                // A large finite offset (egui clamps it): f32::MAX made the content height NaN.
                area = area.vertical_scroll_offset(1.0e9);
            }
            area.show(ui, |ui| {
                ui.style_mut().text_styles.insert(TextStyle::Body, FontId::proportional(font_size));
                // Message text can be selected and copied with Ctrl+C (EmojiLabel's own
                // `selectable` does not reach its text segments, the style flag does).
                ui.style_mut().interaction.selectable_labels = true;
                ui.spacing_mut().item_spacing.y = 0.0;
                let mut any = false;
                let mut row_index = 0;
                for row in model.list.visible() {
                    any = true;
                    let mode = row_mode(self.mode, self.flipped.contains(&row.item.id));
                    let force_tip = self.dev_itemtip && row.item.links.iter().any(|l| l.id == DEV_ITEM_ID);
                    let resp = message_row(ui, row, mode, font_size, self.dev_menu == Some(row.item.id), tips, force_tip);
                    if self.first_row_rect.is_none() || row_index == 0 {
                        self.first_row_rect = Some(resp.rect);
                    }
                    row_index += 1;
                    if resp.clicked() && can_translate(&row.item) {
                        clicked = Some(row.item.id);
                    }
                    let shown = if shows_translation(&row.item, mode) {
                        row.item.translated.clone().unwrap_or_default()
                    } else {
                        row.item.original.clone()
                    };
                    let shared = &self.shared;
                    let copy = |text: &str| {
                        if let Err(e) = win::copy_to_clipboard(shared.hwnd(), text) {
                            shared.log.write(&format!("copy failed: {e}"));
                        }
                    };
                    let flip_clicked = Cell::new(false);
                    let menu = |ui: &mut Ui| {
                        widgets::field_style(ui);
                        if let Some(label) = flip_label(&row.item, mode) {
                            if ui.button(label).clicked() {
                                flip_clicked.set(true);
                                ui.close();
                            }
                            ui.separator();
                        }
                        if ui.button("Copiar mensaje").clicked() {
                            copy(&shown);
                            ui.close();
                        }
                        if ui.button("Copiar original").clicked() {
                            copy(&row.item.original);
                            ui.close();
                        }
                        if ui.button("Copiar nombre").clicked() {
                            copy(&row.item.player);
                            ui.close();
                        }
                    };
                    if self.dev_menu == Some(row.item.id) {
                        // Screenshot of the context menu without a real right click.
                        egui::Area::new(Id::new("dev-row-menu"))
                            .order(Order::Foreground)
                            .fixed_pos(resp.rect.center())
                            .show(ui.ctx(), |ui| Frame::menu(ui.style()).show(ui, menu));
                    } else {
                        resp.context_menu(menu);
                    }
                    if flip_clicked.get() {
                        flip = Some(row.item.id);
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
        // A hovered item without Wowhead data yet (the model lock is released by now).
        if let Some(id) = wanted.get() {
            self.shared.request_wowhead(id);
        }
        if let Some(id) = flip {
            if !self.flipped.remove(&id) {
                self.flipped.insert(id);
            }
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
        // The settings window changes the opacity too: always show the saved value.
        let (paused, opacity) = {
            let c = self.shared.config();
            (c.prefs.translation_paused, c.prefs.opacity)
        };
        self.opacity = opacity;
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 10.0;
            let (dot, _) = ui.allocate_exact_size(vec2(7.0, 7.0), Sense::hover());
            ui.painter().circle_filled(dot.center(), 3.5, reader.dot());
            ui.add_space(-5.0);
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                let resp = ui.add(egui::Slider::new(&mut self.opacity, 0.4..=1.0).show_value(false));
                if resp.changed() {
                    self.shared.config().prefs.opacity = self.opacity;
                    self.shared.hwnd().set_opacity(self.opacity);
                }
                if resp.drag_stopped() || (resp.changed() && !resp.dragged()) {
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
                    let mut shown = transient.map(|(t, _)| t).unwrap_or(text);
                    if shown.is_empty() && self.shared.addon_hint() {
                        shown = super::addon_panel::UPDATE_HINT.to_string();
                    }
                    if !shown.is_empty() {
                        ui.add(egui::Label::new(RichText::new(shown).size(11.0).color(style::AMBER)).truncate());
                    }
                });
            });
        });
    }

    // Compose ----------------------------------------------------------------------------

    fn compose(&mut self, ui: &mut Ui) {
        const ROW_H: f32 = 30.0;
        ui.allocate_ui_with_layout(vec2(ui.available_width(), ROW_H), Layout::right_to_left(Align::Center), |ui| {
            ui.spacing_mut().item_spacing.x = 6.0;
            widgets::field_style(ui);
            // The toggle is laid out first, so the input only takes the width left over.
            let items = OUT_LANGS.map(|(code, label)| (label, code));
            let (picked, _) = widgets::segmented(ui, Id::new("lang-seg"), &items, self.compose.lang, 9.0, ROW_H - 4.0);
            if let Some(code) = picked {
                self.compose.lang = code;
                ui.memory_mut(|m| m.request_focus(Id::new(COMPOSE_ID)));
            }
            let edit = egui::TextEdit::singleline(&mut self.compose.text)
                .id(Id::new(COMPOSE_ID))
                .hint_text(RichText::new(compose::PLACEHOLDER).color(style::DIM))
                .text_color(widgets::INPUT_TEXT)
                .font(FontId::proportional(13.0))
                .vertical_align(Align::Center)
                .margin(Margin::symmetric(9, 4))
                .background_color(white(0.06));
            let resp = ui.add_sized(vec2(ui.available_width().max(40.0), ROW_H), edit);
            if resp.lost_focus() {
                let (enter, esc) = ui.input(|i| (i.key_pressed(Key::Enter), i.key_pressed(Key::Escape)));
                if esc {
                    win::focus_game();
                } else if enter {
                    self.send_compose(ui.ctx());
                }
            }
        });
        if let Some((text, color)) = &self.compose.preview {
            ui.add_space(5.0);
            ui.add(egui::Label::new(RichText::new(text).size(11.5).color(*color)).wrap());
        }
    }

    /// Enter: translate (or pass Spanish through) off the UI thread.
    fn send_compose(&mut self, ctx: &egui::Context) {
        if self.compose.text.trim().is_empty() || self.compose.busy {
            return;
        }
        self.compose.busy = true;
        self.compose.preview = Some((compose::TRANSLATING.into(), style::MUTED));
        let (text, lang) = (self.compose.text.clone(), self.compose.lang);
        let (shared, pending) = (self.shared.clone(), self.compose.pending.clone());
        let ctx = ctx.clone();
        let _ = std::thread::Builder::new().name("compose".into()).spawn(move || {
            let translator = shared.translator();
            let result = translate_outgoing(&text, lang, &*translator);
            *pending.lock().unwrap_or_else(|e| e.into_inner()) = Some(result);
            ctx.request_repaint_of(egui::ViewportId::ROOT);
        });
    }

    /// The translation is back: copy it, say so and bring WoW to the front.
    fn finish_compose(&mut self) {
        let Some(result) = self.compose.pending.lock().unwrap_or_else(|e| e.into_inner()).take() else { return };
        self.compose.busy = false;
        match result.and_then(|out| win::copy_to_clipboard(self.shared.hwnd(), &out).map(|()| out)) {
            Ok(out) => {
                self.compose.preview = Some((copied_message(&out), COPIED));
                self.compose.text.clear();
                // Like Parley: WoW comes back, ready for Enter, Ctrl+V, Enter.
                if !win::focus_game() {
                    self.shared.log.write("focus-game failed");
                }
            }
            Err(e) => {
                self.shared.log.write(&format!("compose failed: {e}"));
                self.compose.preview = Some((failed_message(&e), FAILED));
            }
        }
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
    fn raw_input_hook(&mut self, ctx: &egui::Context, raw: &mut egui::RawInput) {
        let (Some(step), Some(rect)) = (self.dev_select, self.first_row_rect) else { return };
        let y = rect.top() + 12.0;
        // Over the message text of the first row (after time, tag, name and badge).
        let from = pos2(rect.left() + 200.0, y);
        let to = pos2(rect.left() + 330.0, y);
        let button = |pos, pressed| egui::Event::PointerButton {
            pos,
            button: egui::PointerButton::Primary,
            pressed,
            modifiers: egui::Modifiers::NONE,
        };
        match step {
            0 => raw.events.push(egui::Event::PointerMoved(from)),
            1 => raw.events.push(button(from, true)),
            2..=6 => raw.events.push(egui::Event::PointerMoved(from + (to - from) * ((step - 1) as f32 / 5.0))),
            7 => raw.events.push(button(to, false)),
            _ => {
                self.dev_select = None;
                return;
            }
        }
        self.dev_select = Some(step + 1);
        ctx.request_repaint();
    }

    fn clear_color(&self, _visuals: &egui::Visuals) -> [f32; 4] {
        style::BG.to_normalized_gamma_f32()
    }

    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        if std::mem::take(&mut self.first_frame) {
            if self.started_hidden {
                // eframe shows the window after its first frame; --hidden keeps it in the tray.
                self.shared.hwnd().hide();
            }
            self.apply_dev_state(ctx);
        }
        // "Ocultar de las capturas de pantalla" (streams); screenshot runs always allow capture.
        let hide = self.shared.config().prefs.hide_from_capture && !win::dev_capture();
        if self.capture_hidden != Some(hide) && self.shared.hwnd().is_set() {
            self.shared.hwnd().set_capture_hidden(hide);
            self.capture_hidden = Some(hide);
        }
        super::addon_panel::show_first_run(ctx, &self.shared, self.icon.clone());
        if ctx.input(|i| i.viewport().close_requested()) {
            // ✕ and Alt+F4 only hide; quitting is done from the tray menu.
            ctx.send_viewport_cmd(ViewportCommand::CancelClose);
            self.shared.hide_by_user();
        }
        self.persist_bounds(ctx);
        self.finish_compose();
        if self.shared.take_compose_focus() {
            ctx.memory_mut(|m| m.request_focus(Id::new(COMPOSE_ID)));
        }
        if self.shared.settings_open() {
            settings::show(ctx, self.shared.clone(), self.settings.clone(), self.icon.clone());
        }
        let maximized = ctx.input(|i| i.viewport().maximized).unwrap_or(false);
        let font_size = self.shared.config().prefs.font_size.clamp(9.0, 28.0);

        let bar = egui::TopBottomPanel::top("titlebar")
            .show_separator_line(false)
            .frame(Frame::new().fill(white(0.03)).inner_margin(Margin { left: 12, right: 6, top: 6, bottom: 6 }))
            .show(ctx, |ui| self.title_bar(ui, maximized));
        widgets::caption_drag(ctx, Id::new("overlay-caption"), bar.response.rect, bar.inner, Some(maximized));
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
        egui::TopBottomPanel::bottom("compose")
            .show_separator_line(false)
            .frame(Frame::new().fill(style::BG).inner_margin(Margin { left: 10, right: 10, top: 8, bottom: 6 }))
            .show(ctx, |ui| {
                hline(ui, ui.max_rect().top() - 8.0);
                self.compose(ui);
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
    widgets::text_in(ui.painter(), rect, icon, FontId::proportional(13.0), color);
    resp.on_hover_text(tip)
}

/// 📌 (`#pin`): one glyph, same size and place in both states. On: light blue on a
/// blue tint. Off: the same glyph in grey, dimmed (brighter on hover).
fn pin_button(ui: &mut Ui, on: bool) -> Response {
    let (rect, resp) = ui.allocate_exact_size(vec2(28.0, 24.0), Sense::click());
    let hovered = resp.hovered();
    let p = ui.painter();
    let color = if on {
        p.rect_filled(rect, 5.0, with_alpha(style::ACCENT, 56));
        style::ACCENT_LIGHT
    } else {
        if hovered {
            p.rect_filled(rect, 5.0, white(0.1));
        }
        style::ICON.gamma_multiply(if hovered { 0.8 } else { 0.45 })
    };
    widgets::text_in(p, rect, "📌", FontId::proportional(13.0), color);
    let tip = if on { "Siempre encima: activado" } else { "Siempre encima: desactivado" };
    resp.on_hover_text(tip)
}

/// Text in a rounded box; `height` is the allocated row height (the box is centered).
#[allow(clippy::too_many_arguments)]
fn chip(ui: &mut Ui, text: &str, font: FontId, fg: Color32, bg: Color32, pad: egui::Vec2, radius: u8, height: f32, sense: Sense) -> Response {
    let box_size = vec2(widgets::text_width(ui, text, &font), font.size * 1.33) + pad * 2.0;
    let (rect, resp) = ui.allocate_exact_size(vec2(box_size.x, height.max(box_size.y)), sense);
    let chip_rect = Rect::from_center_size(rect.center(), box_size);
    if bg != Color32::TRANSPARENT {
        ui.painter().rect_filled(chip_rect, radius as f32, bg);
    }
    widgets::text_in(ui.painter(), chip_rect, text, font, fg);
    resp
}

fn tab_button(ui: &mut Ui, label: &str, on: bool, count: Option<u32>) -> Response {
    let font = FontId::proportional(11.5);
    let badge_font = FontId::proportional(9.5);
    let count_text = count.map(|n| n.to_string());
    let count_w = count_text.as_ref().map_or(0.0, |t| widgets::text_width(ui, t, &badge_font) + 10.0 + 4.0);
    let (rect, resp) = widgets::tab_rect(ui, label, &font, count_w);
    if on {
        widgets::tab_fill(ui, rect, 0.06);
    }
    let color = widgets::tab_text_color(on, resp.hovered());
    let label_w = widgets::text_width(ui, label, &font);
    let label_rect = Rect::from_min_size(pos2(rect.left() + 10.0, rect.top()), vec2(label_w, rect.height()));
    widgets::text_in(ui.painter(), label_rect, label, font, color);
    if let Some(t) = count_text {
        let w = widgets::text_width(ui, &t, &badge_font) + 10.0;
        let pill = Rect::from_center_size(pos2(label_rect.right() + 4.0 + w / 2.0, rect.center().y), vec2(w, 14.0));
        ui.painter().rect_filled(pill, 7.0, style::ACCENT);
        widgets::text_in(ui.painter(), pill, &t, badge_font, Color32::WHITE);
    }
    resp.on_hover_cursor(CursorIcon::PointingHand)
}

/// One chat line: time, channel tag, player, badge, text (with colored emoji).
/// One message. `highlight` draws it as hovered (screenshots of its context menu).
/// `tips` is set when item data is on ("Datos de objetos"): item links are then drawn in
/// their quality color and show their panel on hover (`force_tip`: open, for screenshots).
fn message_row(
    ui: &mut Ui,
    row: &Row,
    mode: Mode,
    fs: f32,
    highlight: bool,
    tips: Option<TipSources<'_>>,
    force_tip: bool,
) -> Response {
    let item = &row.item;
    let show_t = shows_translation(item, mode);
    let clickable = can_translate(item);
    let bg_slot = ui.painter().add(Shape::Noop);
    let body_h = fs * 1.35;
    let row_id = Id::new(("msg-row", item.id));
    let link_hovered = Cell::new(false);

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
                    // Log lines: neutral chip with a Spanish label, player only when present.
                    let log = log_label(&item.channel);
                    let tab_c = tab_color(item.tab);
                    let label = log.map_or_else(|| channel_label(&item.channel), str::to_string);
                    chip(ui, &label, widgets::semibold(fs * 0.8), tab_c, with_alpha(tab_c, 0x22), vec2(5.0, 0.0), 4, body_h, Sense::hover());
                    ui.add_space(5.0);
                    if log.is_none() || !item.player.is_empty() {
                        ui.add(
                            egui::Label::new(RichText::new(&item.player).family(widgets::semibold_family()).color(class_color(item.class.as_deref()))),
                        );
                    }
                    if let Some(b) = badge(item, show_t) {
                        let (fg, bg) = match b.kind {
                            BadgeKind::Error => (style::ERROR_TEXT, style::ERROR_BG),
                            BadgeKind::Paused => (style::AMBER_LIGHT, with_alpha(Color32::from_rgb(245, 158, 11), 51)),
                            BadgeKind::Lang => (style::SEG_TEXT, white(0.1)),
                        };
                        ui.add_space(5.0);
                        chip(ui, &b.text, FontId::proportional(fs * 0.72), fg, bg, vec2(4.0, 0.0), 3, body_h, Sense::hover());
                    }
                    let text = if show_t { item.translated.as_deref().unwrap_or("") } else { &item.original };
                    // Log lines use the game's chat color for their kind (system yellow, emotes
                    // orange, loot green...); everything else the normal text color.
                    let base = log_text_color(&item.channel).unwrap_or(style::TEXT);
                    let color = if item.status == Status::Pending { base.gamma_multiply(0.55) } else { base };
                    let lead = if log.is_some() && item.player.is_empty() { "" } else { ": " };
                    match tips.and_then(|tips| item_segments(text, &item.links).map(|segs| (tips, segs))) {
                        Some((tips, segs)) => {
                            let mut force = force_tip;
                            if !lead.is_empty() {
                                EmojiLabel::new(RichText::new(lead).color(color)).show(ui);
                            }
                            for seg in segs {
                                match seg {
                                    Segment::Text(t) => {
                                        EmojiLabel::new(RichText::new(t).color(color)).show(ui);
                                    }
                                    Segment::Link(name, link) => {
                                        let view = tips.view(link.id);
                                        let c = item_quality_color(view.and_then(|v| v.tip()).and_then(|t| t.quality));
                                        let resp = ui.add(egui::Label::new(RichText::new(&name).color(c)));
                                        // The row's own click area covers the text, so
                                        // hover is read from the pointer position.
                                        if std::mem::take(&mut force) || ui.ctx().pointer_hover_pos().is_some_and(|p| resp.rect.contains(p)) {
                                            link_hovered.set(true);
                                            tips.hovered(ui, link.id);
                                            if let Some(view) = view {
                                                item_panel(ui, &resp, &name, view, fs);
                                            }
                                        }
                                    }
                                }
                            }
                        }
                        None if log.is_some() => {
                            // "online"/"joins" in green, "offline"/"leaves" in red, like the game.
                            if !lead.is_empty() {
                                EmojiLabel::new(RichText::new(lead).color(color)).show(ui);
                            }
                            for (span, h) in log_highlights(text) {
                                let c = h.map_or(color, log_highlight_color);
                                EmojiLabel::new(RichText::new(span).color(c)).show(ui);
                            }
                        }
                        None => {
                            EmojiLabel::new(RichText::new(format!("{lead}{text}")).color(color)).show(ui);
                        }
                    }
                    // Stacked identical log lines: ×N after the text.
                    if row.count > 1 {
                        ui.add_space(6.0);
                        let n = format!("×{}", row.count);
                        chip(ui, &n, FontId::proportional(fs * 0.72), style::SEG_TEXT, white(0.1), vec2(4.0, 0.0), 3, body_h, Sense::hover());
                    }
                });
            });
        });
    });

    let rect = inner.response.rect;
    // Click only (never drag): a press-and-drag goes to the selectable labels below and
    // selects text, a plain click translates paused rows, a right click opens the menu.
    let resp = ui.interact(rect, row_id, Sense::click());
    if highlight || resp.hovered() {
        let fill = if clickable { with_alpha(style::ACCENT, 36) } else { white(0.05) };
        ui.painter().set(bg_slot, egui::epaint::RectShape::filled(rect, 6.0, fill));
    }
    let mut tip = Vec::new();
    if let Some(b) = badge(item, show_t).and_then(|b| b.tip) {
        tip.push(b);
    }
    if clickable && item.status == Status::Paused {
        tip.push("Clic para traducir este mensaje".into());
    }
    let resp = if clickable { resp.on_hover_cursor(CursorIcon::PointingHand) } else { resp };
    if tip.is_empty() || link_hovered.get() {
        resp
    } else {
        resp.on_hover_text(tip.join("\n"))
    }
}

/// The item data sources of the hover panel for one frame.
#[derive(Clone, Copy)]
struct TipSources<'a> {
    mode: ItemTooltips,
    addon: &'a ItemTooltipCache,
    wowhead: &'a WowheadCache,
    /// The hovered item to fetch from Wowhead after the frame's model lock is released.
    wanted: &'a Cell<Option<u64>>,
}

impl<'a> TipSources<'a> {
    fn view(&self, id: u64) -> Option<PanelView<'a>> {
        panel_view(self.mode, self.addon.get(id), self.wowhead.get(id))
    }

    /// Marks a hovered item for fetching when Wowhead is the source and has no answer
    /// yet; the answer repaints, and so does a short timer in case the request could not
    /// start this frame (too many at once).
    fn hovered(&self, ui: &Ui, id: u64) {
        if self.mode != ItemTooltips::Wowhead {
            return;
        }
        match self.wowhead.get(id) {
            None => {
                self.wanted.set(Some(id));
                ui.ctx().request_repaint_after(Duration::from_millis(300));
            }
            Some(WowheadState::Failed { .. }) => self.wanted.set(Some(id)),
            Some(WowheadState::Loading | WowheadState::Ready(_)) => {}
        }
    }
}

/// The text split at its item links, or `None` when it has no item link to draw apart
/// (the row is then drawn exactly as without item data).
fn item_segments(text: &str, links: &[LinkRef]) -> Option<Vec<Segment>> {
    let items: Vec<LinkRef> = links.iter().filter(|l| l.kind == "item").cloned().collect();
    if items.is_empty() {
        return None;
    }
    let segs = link_segments(text, &items);
    segs.iter().any(|s| matches!(s, Segment::Link(..))).then_some(segs)
}

const TIP_LINE: Color32 = Color32::from_rgb(0xdd, 0xdd, 0xdd);

/// The small item panel under a hovered item link: the name in its quality color, then
/// the tooltip lines (right column right-aligned) and, for Wowhead or its addon
/// fallback, a small source note. Without data, the name and a message ("Cargando
/// datos…" while the addon or Wowhead has not answered, "Sin datos de Wowhead").
fn item_panel(ui: &Ui, link: &Response, name: &str, view: PanelView<'_>, fs: f32) {
    let size = fs.max(12.0);
    // A fixed width like the game's tooltip: without a minimum egui shrinks the panel to
    // its narrowest line and long lines wrap at almost every word.
    let width = (fs * 22.0).clamp(260.0, 360.0);
    // Placed by hand (not egui's tooltip): above the link like the game, below only when
    // it does not fit, and never on top of the link itself, so the pointer stays on the
    // link and the panel cannot flicker. Not interactable: the pointer passes through.
    let ctx = ui.ctx();
    let id = link.id.with("item-panel");
    let prev_height = ctx.memory(|m| m.area_rect(id)).map(|r| r.height());
    let (pivot, pos) = if panel_goes_above(link.rect.top(), prev_height, ctx.content_rect().top()) {
        (Align2::LEFT_BOTTOM, link.rect.left_top() - vec2(0.0, PANEL_GAP))
    } else {
        (Align2::LEFT_TOP, link.rect.left_bottom() + vec2(0.0, PANEL_GAP))
    };
    // The border takes the item's quality color, like the game.
    let border = match &view {
        PanelView::Tip { tip, .. } => item_quality_color(tip.quality),
        PanelView::Message(_) => style::LINK_GOLD,
    };
    egui::Area::new(id).order(Order::Tooltip).pivot(pivot).fixed_pos(pos).constrain(true).interactable(false).show(ctx, |ui| {
      Frame::popup(ui.style()).stroke(Stroke::new(1.5_f32, border)).show(ui, |ui| {
        ui.set_width(width);
        ui.spacing_mut().item_spacing.y = 3.0;
        let (tip, note) = match view {
            PanelView::Tip { tip, note } => (tip, note),
            PanelView::Message(text) => {
                ui.label(RichText::new(name.trim_matches(['[', ']'])).size(size).family(widgets::semibold_family()).color(style::LINK_GOLD));
                ui.label(RichText::new(text).size(size * 0.9).color(style::MUTED));
                return;
            }
        };
        for (i, TipLine { left, right }) in tip.lines.iter().enumerate() {
            let (text, color) = if i == 0 {
                (RichText::new(left).family(widgets::semibold_family()).size(size * 1.05), item_quality_color(tip.quality))
            } else {
                (RichText::new(left).size(size), TIP_LINE)
            };
            match right {
                None => {
                    ui.add(egui::Label::new(text.color(color)).wrap());
                }
                Some(r) => {
                    ui.horizontal(|ui| {
                        ui.label(text.color(color));
                        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                            ui.label(RichText::new(r).size(size).color(TIP_LINE));
                        });
                    });
                }
            }
        }
        if let Some(note) = note {
            ui.add_space(2.0);
            ui.label(RichText::new(note).size(size * 0.78).color(style::DIM));
        }
      });
    });
}

/// Sample item of the `sample` dev state (screenshots): Thunderfury, legendary.
const DEV_ITEM_ID: u64 = 19019;

fn dev_item_link() -> LinkRef {
    LinkRef { name: "[Thunderfury, Blessed Blade of the Windseeker]".into(), kind: "item".into(), id: DEV_ITEM_ID }
}

fn dev_item_tooltip() -> ItemTooltip {
    let text = "q5\nThunderfury, Blessed Blade of the Windseeker\nBinds when picked up\nUnique\nOne-Hand\tSword\n\
        44 - 84 Damage\tSpeed 1.90\n(33.7 damage per second)\n+5 Agility\n+8 Stamina\n+8 Fire Resistance\n\
        +9 Nature Resistance\nChance on hit: Blasts your enemy with lightning, dealing 300 Nature damage and then \
        jumping to additional nearby enemies.\nRequires Level 60";
    crate::core::item_tips::parse_item_tooltip("19019", text).map(|(_, t)| t).expect("valid sample")
}
