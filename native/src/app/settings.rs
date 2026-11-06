//! The settings window (port of `app/src/renderer/settings.html` + `settings.js`): a
//! second, deferred egui viewport with the tabs Traducción / Ventana / General.
//! Form rules live in `settings_form`; network and CLI work runs on short threads.

use std::sync::{Arc, Mutex, MutexGuard};

use eframe::egui::{
    self, vec2, Align, Align2, Color32, CornerRadius, CursorIcon, FontId, Frame, Id, Key, Layout, Margin, Response,
    RichText, ScrollArea, Sense, Stroke, Ui, ViewportBuilder, ViewportClass, ViewportCommand, ViewportId,
};

use crate::secret::{encrypt_api_key, read_api_key};
use crate::translators::claude_code::find_claude_executable;
use crate::translators::google::GoogleTranslator;
use crate::translators::http::{HttpClient, UreqClient};
use crate::translators::{build_translator, llm_for, Llm};
use crate::core::prefs::TranslatorSettings;

use super::autostart;
use super::settings_form::{
    form_for_provider, is_llm, key_hint, key_placeholder, load_models, models_message, probe, provider_option_label,
    request_key, save_translator, saved_translator, shows_key, shows_url, validate_form, KeyInput, SavedTranslator,
    TranslatorForm, FALLBACK_HINT, MODEL_PLACEHOLDER, PROVIDER_OPTIONS,
};
use super::shared::Shared;
use super::style::{self, translator_label};
use super::tray;
use super::win;

pub const TITLE: &str = "Ajustes · WoW Chat Translator";
const WIDTH: f32 = 500.0;
const HEIGHT: f32 = 600.0;
const SHEET_BG: Color32 = Color32::from_rgb(0x0f, 0x14, 0x1d);
const HINT: Color32 = Color32::from_rgb(0x6b, 0x74, 0x80);
const OK: Color32 = Color32::from_rgb(0x86, 0xef, 0xac);
const ERR: Color32 = Color32::from_rgb(0xfc, 0xa5, 0xa5);
const STRIP_NOTE: &str = "El addon envía el chat como una franja diminuta de colores en la esquina superior izquierda de WoW. No pongas ninguna ventana encima de esa esquina.";

pub fn viewport_id() -> ViewportId {
    ViewportId::from_hash_of("wct-settings")
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Pane {
    Translate,
    Window,
    General,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Kind {
    Neutral,
    Ok,
    Err,
}

/// Everything the settings window shows; shared with its background jobs.
pub struct SettingsState {
    pane: Pane,
    saved: SavedTranslator,
    form: TranslatorForm,
    key_text: String,
    models: Vec<String>,
    result: Option<(String, Kind)>,
    busy: bool,
    /// Bumped when the form is reset, so a late job result is dropped.
    generation: u64,
}

impl SettingsState {
    pub fn new(shared: &Shared) -> Self {
        let saved = saved_translator(&shared.config());
        let form = form_for_provider(&saved.provider, &saved);
        Self {
            pane: Pane::Translate,
            saved,
            form,
            key_text: String::new(),
            models: Vec::new(),
            result: None,
            busy: false,
            generation: 0,
        }
    }

    fn switch_provider(&mut self, provider: &str) {
        self.form = form_for_provider(provider, &self.saved);
        self.key_text.clear();
        self.models.clear();
        self.result = None;
        self.busy = false;
        self.generation += 1;
    }
}

fn lock(state: &Mutex<SettingsState>) -> MutexGuard<'_, SettingsState> {
    state.lock().unwrap_or_else(|e| e.into_inner())
}

/// ⚙: opens the window, or brings the open one to the front.
pub fn open(ctx: &egui::Context, shared: &Shared, state: &Arc<Mutex<SettingsState>>) {
    if shared.settings_open() {
        ctx.send_viewport_cmd_to(viewport_id(), ViewportCommand::Visible(true));
        ctx.send_viewport_cmd_to(viewport_id(), ViewportCommand::Focus);
        return;
    }
    *lock(state) = SettingsState::new(shared);
    shared.set_settings_open(true);
}

fn close(ctx: &egui::Context, shared: &Shared) {
    ctx.send_viewport_cmd(ViewportCommand::Visible(false));
    shared.set_settings_open(false);
}

/// Called every overlay frame while the window is open.
pub fn show(ctx: &egui::Context, shared: Arc<Shared>, state: Arc<Mutex<SettingsState>>, icon: Option<Arc<egui::IconData>>) {
    let (wx, wy, ww, wh) = win::work_area_points();
    let mut builder = ViewportBuilder::default()
        .with_title(TITLE)
        .with_decorations(false)
        .with_resizable(true)
        .with_inner_size([WIDTH, HEIGHT])
        .with_min_inner_size([420.0, 420.0])
        .with_position([wx + (ww - WIDTH) / 2.0, wy + (wh - HEIGHT) / 2.0])
        .with_always_on_top();
    if let Some(icon) = icon {
        builder = builder.with_icon(icon);
    }
    ctx.show_viewport_deferred(viewport_id(), builder, move |ctx, class| {
        if class == ViewportClass::Embedded {
            return;
        }
        if ctx.input(|i| i.viewport().close_requested() || i.key_pressed(Key::Escape)) {
            close(ctx, &shared);
            return;
        }
        draw(ctx, &shared, &state);
    });
}

fn draw(ctx: &egui::Context, shared: &Arc<Shared>, state: &Arc<Mutex<SettingsState>>) {
    egui::TopBottomPanel::top("settings-head")
        .show_separator_line(false)
        .frame(Frame::new().fill(SHEET_BG).inner_margin(Margin { left: 14, right: 8, top: 8, bottom: 0 }))
        .show(ctx, |ui| {
            let bar = ui.interact(ui.max_rect(), Id::new("settings-drag"), Sense::click_and_drag());
            if bar.drag_started_by(egui::PointerButton::Primary) {
                ctx.send_viewport_cmd(ViewportCommand::StartDrag);
            }
            ui.horizontal(|ui| {
                ui.label(RichText::new("Ajustes").size(15.0).strong().color(style::TEXT));
                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                    if close_button(ui).clicked() {
                        close(ctx, shared);
                    }
                });
            });
            ui.add_space(6.0);
            let mut st = lock(state);
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = 2.0;
                for (label, pane) in [("Traducción", Pane::Translate), ("Ventana", Pane::Window), ("General", Pane::General)] {
                    if stab(ui, label, st.pane == pane).clicked() {
                        st.pane = pane;
                    }
                }
            });
            let y = ui.min_rect().bottom();
            ui.painter().hline(ui.max_rect().x_range(), y, Stroke::new(1.0_f32, Color32::from_white_alpha(20)));
        });
    egui::CentralPanel::default()
        .frame(Frame::new().fill(SHEET_BG).inner_margin(Margin { left: 14, right: 14, top: 8, bottom: 12 }))
        .show(ctx, |ui| {
            ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| {
                ui.spacing_mut().item_spacing.y = 4.0;
                let pane = lock(state).pane;
                match pane {
                    Pane::Translate => translate_pane(ui, shared, state),
                    Pane::Window => window_pane(ui, shared),
                    Pane::General => general_pane(ui, shared),
                }
            });
        });
}

// Traducción -----------------------------------------------------------------------------

fn translate_pane(ui: &mut Ui, shared: &Arc<Shared>, state: &Arc<Mutex<SettingsState>>) {
    let mut st = lock(state);
    let full = ui.available_width();

    field_label(ui, "Proveedor");
    let mut provider = st.form.provider.clone();
    egui::ComboBox::from_id_salt("t-provider")
        .width(full)
        .selected_text(provider_option_label(&provider))
        .show_ui(ui, |ui| {
            for (id, label) in PROVIDER_OPTIONS {
                ui.selectable_value(&mut provider, id.to_string(), label);
            }
        });
    if provider != st.form.provider {
        st.switch_provider(&provider);
    }

    let p = st.form.provider.clone();
    if is_llm(&p) {
        if shows_url(&p) {
            field_label(ui, "URL del servidor");
            ui.add(egui::TextEdit::singleline(&mut st.form.base_url).desired_width(full));
        }
        if shows_key(&p) {
            field_label(ui, "Clave API");
            let placeholder = key_placeholder(&p, &st.saved);
            let resp = ui.add(egui::TextEdit::singleline(&mut st.key_text).password(true).hint_text(placeholder).desired_width(full));
            if resp.changed() {
                st.form.key = KeyInput::Typed(st.key_text.clone());
            }
        }
        hint(ui, key_hint(&p));
        field_label(ui, "Modelo");
        let mut load = false;
        ui.horizontal(|ui| {
            let button_w = 120.0;
            let list_w = if st.models.is_empty() { 0.0 } else { 30.0 };
            let edit_w = (ui.available_width() - button_w - list_w).max(80.0);
            ui.add(egui::TextEdit::singleline(&mut st.form.model).hint_text(MODEL_PLACEHOLDER).desired_width(edit_w));
            if !st.models.is_empty() {
                let mut chosen = None;
                egui::ComboBox::from_id_salt("t-models").width(18.0).selected_text("").show_ui(ui, |ui| {
                    for m in &st.models {
                        if ui.selectable_label(st.form.model == *m, m).clicked() {
                            chosen = Some(m.clone());
                        }
                    }
                });
                if let Some(m) = chosen {
                    st.form.model = m;
                }
            }
            load = ui.add_enabled(!st.busy, button("Cargar modelos", false)).clicked();
        });
        hint(ui, FALLBACK_HINT);
        if load {
            st.result = Some(("Cargando modelos…".into(), Kind::Neutral));
            spawn_job(ui.ctx(), shared, state, &mut st, Job::Models);
        }
    }

    if let Some((text, kind)) = &st.result {
        ui.add_space(6.0);
        let color = match kind {
            Kind::Ok => OK,
            Kind::Err => ERR,
            Kind::Neutral => style::MUTED,
        };
        ui.add(egui::Label::new(RichText::new(text).size(11.5).color(color)).wrap());
    }

    ui.add_space(10.0);
    ui.with_layout(Layout::right_to_left(Align::Min), |ui| {
        if ui.add_enabled(!st.busy, button("Guardar traductor", true)).clicked() {
            save(shared, &mut st);
        }
        if ui.add_enabled(!st.busy, button("Probar", false)).clicked() {
            st.result = Some(("Probando…".into(), Kind::Neutral));
            spawn_job(ui.ctx(), shared, state, &mut st, Job::Probe);
        }
    });
}

#[derive(Clone, Copy)]
enum Job {
    Probe,
    Models,
}

/// Runs "Probar" / "Cargar modelos" off the UI thread. The saved key is only ever
/// obtained through `read_api_key` (binding-checked), never decrypted here.
fn spawn_job(ctx: &egui::Context, shared: &Arc<Shared>, state: &Arc<Mutex<SettingsState>>, st: &mut SettingsState, job: Job) {
    st.busy = true;
    let generation = st.generation;
    let form = st.form.clone();
    let config = shared.config().clone();
    let state = state.clone();
    let ctx = ctx.clone();
    let _ = std::thread::Builder::new().name("settings-job".into()).spawn(move || {
        let key = request_key(&config, &form, &read_api_key);
        let http: Arc<dyn HttpClient> = Arc::new(UreqClient::new());
        let make_llm = |s: &TranslatorSettings, k: Option<String>| -> Result<Box<dyn Llm>, String> {
            llm_for(s, k, http.clone(), &find_claude_executable)
        };
        let outcome = match job {
            Job::Probe => probe(&form, key, &GoogleTranslator::new(http.clone()), &make_llm).map(|text| (text, Vec::new())),
            Job::Models => load_models(&form, key, &make_llm).map(|models| (models_message(models.len()), models)),
        };
        let mut st = lock(&state);
        if st.generation != generation {
            return;
        }
        st.busy = false;
        st.result = Some(match (job, outcome) {
            (Job::Probe, Ok((text, _))) => (text, Kind::Ok),
            (Job::Probe, Err(e)) => (format!("Error: {e}"), Kind::Err),
            (Job::Models, Ok((text, models))) => {
                if st.form.model.trim().is_empty() && models.len() == 1 {
                    st.form.model = models[0].clone();
                }
                st.models = models;
                (text, Kind::Ok)
            }
            (Job::Models, Err(e)) => (format!("No se pudo cargar la lista: {e}"), Kind::Err),
        });
        drop(st);
        ctx.request_repaint_of(viewport_id());
    });
}

/// "Guardar traductor": validates, stores (key encrypted with DPAPI and bound) and
/// swaps the running translator.
fn save(shared: &Shared, st: &mut SettingsState) {
    if let Err(e) = validate_form(&st.form) {
        st.result = Some((e, Kind::Err));
        return;
    }
    let config = {
        let mut config = shared.config();
        match save_translator(&config.translator, &st.form, &encrypt_api_key) {
            Err(e) => {
                st.result = Some((format!("Error: {e}"), Kind::Err));
                return;
            }
            Ok(next) => config.translator = next,
        }
        config.clone()
    };
    shared.save_config();
    let built = build_translator(&config);
    let label = translator_label(&config.translator);
    shared.set_translator(Arc::from(built.translator), label.clone());
    if let Some(issue) = &built.issue {
        shared.log.write(&format!("translator issue: {issue}"));
        shared.set_status_text(issue);
    } else {
        shared.set_status_text("");
    }
    st.saved = saved_translator(&config);
    st.form.key = KeyInput::Untouched;
    st.key_text.clear();
    st.result = Some((format!("Guardado. Traduciendo con: {label}"), Kind::Ok));
}

// Ventana --------------------------------------------------------------------------------

fn window_pane(ui: &mut Ui, shared: &Arc<Shared>) {
    let prefs = shared.config().prefs.clone();

    let mut on_top = prefs.always_on_top;
    row(ui, "Siempre encima", "La ventana del chat se queda delante de WoW aunque hagas clic en el juego.", |ui| {
        if switch(ui, &mut on_top).changed() {
            shared.config().prefs.always_on_top = on_top;
            shared.save_config();
            let level = if on_top { egui::WindowLevel::AlwaysOnTop } else { egui::WindowLevel::Normal };
            ui.ctx().send_viewport_cmd_to(ViewportId::ROOT, ViewportCommand::WindowLevel(level));
            shared.repaint();
        }
    });

    let mut pct = (prefs.opacity * 100.0).round();
    row(ui, "Opacidad", &format!("{pct} %"), |ui| {
        ui.spacing_mut().slider_width = 140.0;
        let resp = ui.add(egui::Slider::new(&mut pct, 40.0..=100.0).step_by(1.0).show_value(false));
        if resp.changed() {
            let opacity = pct / 100.0;
            shared.config().prefs.opacity = opacity;
            shared.hwnd().set_opacity(opacity);
            shared.repaint();
        }
        if resp.drag_stopped() || (resp.changed() && !resp.dragged()) {
            shared.save_config();
        }
    });

    let mut font = prefs.font_size.round();
    row(ui, "Tamaño del texto", &format!("{font} px"), |ui| {
        ui.spacing_mut().slider_width = 140.0;
        let resp = ui.add(egui::Slider::new(&mut font, 10.0..=22.0).step_by(1.0).show_value(false));
        if resp.changed() {
            shared.config().prefs.font_size = font;
            shared.repaint();
        }
        if resp.drag_stopped() || (resp.changed() && !resp.dragged()) {
            shared.save_config();
        }
    });

    row(ui, "Franja del addon", STRIP_NOTE, |_| {});
}

// General --------------------------------------------------------------------------------

fn general_pane(ui: &mut Ui, shared: &Arc<Shared>) {
    let mut auto = autostart::is_enabled();
    row(ui, "Iniciar con Windows", "Arranca oculta y aparece sola al abrir WoW.", |ui| {
        if switch(ui, &mut auto).changed() {
            match autostart::set_enabled(auto) {
                Ok(()) => {
                    shared.config().prefs.auto_start = auto;
                    shared.save_config();
                    tray::set_autostart_checked(auto);
                    shared.log.write(&format!("autostart {auto}"));
                }
                Err(e) => shared.log.write(&format!("autostart failed: {e}")),
            }
        }
    });
    row(ui, "Mensajes", "Borra la lista de mensajes de la ventana del chat.", |ui| {
        if ui.add(button("Limpiar", false)).clicked() {
            shared.clear_messages();
        }
    });

    ui.add_space(6.0);
    ui.label(RichText::new("Atajo (solo con WoW abierto)").strong().size(12.0).color(style::TEXT));
    ui.horizontal(|ui| {
        kbd(ui, "Ctrl+Shift+Enter");
        ui.label(RichText::new("Escribir un mensaje").size(12.0).color(style::TEXT));
    });
    separator_line(ui);

    row(ui, "Salir", "Cierra la app por completo. ✕ solo la oculta.", |ui| {
        if ui.add(button("Salir de la app", false)).on_hover_cursor(CursorIcon::PointingHand).clicked() {
            tray::quit_app(shared);
        }
    });

    ui.add_space(6.0);
    ui.label(RichText::new("Acerca de").strong().size(12.0).color(style::TEXT));
    hint(ui, "Emoji: gráficos de Twemoji (© Twitter, Inc. y otros colaboradores), con licencia CC-BY 4.0: https://github.com/twitter/twemoji");
    hint(ui, "El diseño de la franja de píxeles se inspiró en Parley y LibSerpix (licencia MIT).");
}

// Widgets --------------------------------------------------------------------------------

fn field_label(ui: &mut Ui, text: &str) {
    ui.add_space(6.0);
    ui.label(RichText::new(text).size(12.0).color(Color32::from_gray(0xbb)));
}

fn hint(ui: &mut Ui, text: &str) {
    if !text.is_empty() {
        ui.add(egui::Label::new(RichText::new(text).size(10.5).color(HINT)).wrap());
    }
}

/// `.row-set`: bold title + hint on the left, control on the right, bottom line.
fn row(ui: &mut Ui, title: &str, hint_text: &str, control: impl FnOnce(&mut Ui)) {
    ui.add_space(5.0);
    ui.horizontal(|ui| {
        let control_w = 150.0;
        let text_w = (ui.available_width() - control_w - 12.0).max(120.0);
        ui.vertical(|ui| {
            ui.set_width(text_w);
            ui.label(RichText::new(title).strong().size(13.0).color(style::TEXT));
            hint(ui, hint_text);
        });
        ui.with_layout(Layout::right_to_left(Align::Center), control);
    });
    separator_line(ui);
}

fn separator_line(ui: &mut Ui) {
    ui.add_space(5.0);
    let y = ui.cursor().top();
    ui.painter().hline(ui.max_rect().x_range(), y, Stroke::new(1.0_f32, Color32::from_white_alpha(13)));
    ui.add_space(1.0);
}

fn button(text: &str, primary: bool) -> egui::Button<'static> {
    let (fg, bg) = if primary { (Color32::WHITE, style::ACCENT) } else { (Color32::from_gray(0xdd), Color32::from_white_alpha(20)) };
    egui::Button::new(RichText::new(text.to_string()).size(12.0).color(fg))
        .fill(bg)
        .stroke(Stroke::NONE)
        .corner_radius(CornerRadius::same(6))
        .min_size(vec2(0.0, 26.0))
}

fn kbd(ui: &mut Ui, text: &str) {
    Frame::new()
        .fill(Color32::from_white_alpha(20))
        .corner_radius(CornerRadius::same(4))
        .inner_margin(Margin::symmetric(6, 1))
        .show(ui, |ui| ui.label(RichText::new(text).size(11.0).color(style::TEXT)));
}

/// The `.switch` toggle.
fn switch(ui: &mut Ui, on: &mut bool) -> Response {
    let (rect, mut resp) = ui.allocate_exact_size(vec2(34.0, 18.0), Sense::click());
    if resp.clicked() {
        *on = !*on;
        resp.mark_changed();
    }
    let p = ui.painter();
    p.rect_filled(rect, 9.0, if *on { style::ACCENT } else { Color32::from_white_alpha(38) });
    let x = if *on { rect.right() - 9.0 } else { rect.left() + 9.0 };
    p.circle_filled(egui::pos2(x, rect.center().y), 7.0, Color32::WHITE);
    resp.on_hover_cursor(CursorIcon::PointingHand)
}

fn stab(ui: &mut Ui, label: &str, on: bool) -> Response {
    let galley = ui.painter().layout_no_wrap(label.to_owned(), FontId::proportional(12.0), Color32::WHITE);
    let (rect, resp) = ui.allocate_exact_size(galley.size() + vec2(20.0, 10.0), Sense::click());
    if on {
        ui.painter().rect_filled(rect, CornerRadius { nw: 6, ne: 6, sw: 0, se: 0 }, Color32::from_white_alpha(18));
    }
    let color = if on {
        Color32::WHITE
    } else if resp.hovered() {
        Color32::from_gray(0xdd)
    } else {
        style::TAB_IDLE
    };
    ui.painter().galley(rect.center() - galley.size() / 2.0, galley, color);
    resp.on_hover_cursor(CursorIcon::PointingHand)
}

fn close_button(ui: &mut Ui) -> Response {
    let (rect, resp) = ui.allocate_exact_size(vec2(28.0, 24.0), Sense::click());
    if resp.hovered() {
        ui.painter().rect_filled(rect, 5.0, style::CLOSE_HOVER);
    }
    let color = if resp.hovered() { Color32::WHITE } else { style::ICON };
    ui.painter().text(rect.center(), Align2::CENTER_CENTER, "🗙", FontId::proportional(13.0), color);
    resp.on_hover_text("Cerrar")
}
