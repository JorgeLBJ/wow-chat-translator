//! The settings window (port of `app/src/renderer/settings.html` + `settings.js`): a
//! second, deferred egui viewport with the tabs Traducción / Ventana / General.
//! Form rules live in `settings_form`; network and CLI work runs on short threads.

use std::sync::{Arc, Mutex, MutexGuard};

use eframe::egui::{
    self, vec2, Align, Color32, CornerRadius, CursorIcon, FontId, Frame, Id, Key, Layout, Margin, Response,
    Order, RichText, ScrollArea, Sense, Stroke, Ui, ViewportBuilder, ViewportClass, ViewportCommand, ViewportId,
};

use crate::secret::{encrypt_api_key, read_api_key};
use crate::translators::claude_code::find_claude_executable;
use crate::translators::google::GoogleTranslator;
use crate::translators::http::{HttpClient, UreqClient};
use crate::translators::{build_translator, llm_for, Llm};
use crate::core::prefs::{ItemTooltips, TranslatorSettings};

use super::{addon_panel, autostart};
use super::settings_form::{
    filter_models, form_for_provider, is_llm, key_hint, key_placeholder, load_models, models_message, probe, provider_option_label,
    request_key, save_translator, saved_translator, shows_key, shows_url, validate_form, KeyInput, SavedTranslator,
    TranslatorForm, FALLBACK_HINT, MODEL_PLACEHOLDER, PROVIDER_OPTIONS,
};
use super::shared::Shared;
use super::style::{self, translator_label};
use super::tray;
use super::widgets::{self, ButtonKind};
use super::win;

pub const TITLE: &str = "Ajustes · WoW Chat Translator";
const WIDTH: f32 = 500.0;
const HEIGHT: f32 = 600.0;
const SHEET_BG: Color32 = Color32::from_rgb(0x0f, 0x14, 0x1d);
pub(super) const HINT: Color32 = Color32::from_rgb(0x6b, 0x74, 0x80);
const OK: Color32 = Color32::from_rgb(0x86, 0xef, 0xac);
const ERR: Color32 = Color32::from_rgb(0xfc, 0xa5, 0xa5);
const CAPTURE_NOTE: &str = "Útil para streams: la ventana no saldrá en capturas ni grabaciones.";
const LOG_NOTE: &str = "Muestra también botín, oro, sistema, logros y PNJ en la pestaña Registro (sin traducir).";
const ITEM_TIPS_NOTE: &str = "Al pasar el ratón por un objeto enlazado muestra sus datos. «WoW (addon)» usa el texto del propio juego; «Wowhead (internet)» envía el id del objeto a wowhead.com.";
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
    /// The model list under the Modelo field is showing.
    models_open: bool,
    result: Option<(String, Kind)>,
    busy: bool,
    /// Bumped when the form is reset, so a late job result is dropped.
    generation: u64,
    /// Display affinity applied to this window (`hideFromCapture`), once known.
    capture_hidden: Option<bool>,
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
            models_open: false,
            result: None,
            busy: false,
            generation: 0,
            capture_hidden: None,
        }
    }

    fn switch_provider(&mut self, provider: &str) {
        self.form = form_for_provider(provider, &self.saved);
        self.models_open = false;
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

/// Developer screenshots only: opens the window on `pane` (translate|window|general).
pub fn dev_open(ctx: &egui::Context, shared: &Shared, state: &Arc<Mutex<SettingsState>>, pane: &str) {
    open(ctx, shared, state);
    let mut st = lock(state);
    st.pane = match pane {
        "window" => Pane::Window,
        "general" => Pane::General,
        _ => Pane::Translate,
    };
    if pane == "models" {
        // OpenRouter with a loaded, filtered model list.
        st.switch_provider("openrouter");
        st.models = ["deepseek/deepseek-chat", "deepseek/deepseek-r1", "deepseek/deepseek-v3.2", "openai/gpt-5-mini", "google/gemini-2.5-flash"]
            .map(String::from)
            .to_vec();
        st.form.model = "deepseek".into();
        st.models_open = true;
        st.result = Some((models_message(st.models.len()), Kind::Ok));
    }
}

/// ⚙: opens the window, or brings the open one to the front.
pub fn open(ctx: &egui::Context, shared: &Shared, state: &Arc<Mutex<SettingsState>>) {
    if shared.settings_open() {
        ctx.send_viewport_cmd_to(viewport_id(), ViewportCommand::Visible(true));
        ctx.send_viewport_cmd_to(viewport_id(), ViewportCommand::Focus);
        return;
    }
    *lock(state) = SettingsState::new(shared);
    shared.addon().reset();
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
        // Esc closes the model list first, then the window.
        let list_open = lock(&state).models_open;
        if ctx.input(|i| i.viewport().close_requested() || (i.key_pressed(Key::Escape) && !list_open)) {
            close(ctx, &shared);
            return;
        }
        draw(ctx, &shared, &state);
    });
}

fn draw(ctx: &egui::Context, shared: &Arc<Shared>, state: &Arc<Mutex<SettingsState>>) {
    let hide = shared.config().prefs.hide_from_capture && !win::dev_capture();
    {
        let mut st = lock(state);
        if st.capture_hidden != Some(hide) {
            let h = win::find_own_window(TITLE);
            if h.is_set() {
                h.set_capture_hidden(hide);
                st.capture_hidden = Some(hide);
            }
        }
    }
    let head = egui::TopBottomPanel::top("settings-head")
        .show_separator_line(false)
        .frame(Frame::new().fill(SHEET_BG).inner_margin(Margin { left: 14, right: 8, top: 8, bottom: 0 }))
        .show(ctx, |ui| {
            let mut hot = false;
            ui.allocate_ui_with_layout(vec2(ui.available_width(), 24.0), Layout::right_to_left(Align::Center), |ui| {
                let r = close_button(ui);
                hot |= r.contains_pointer();
                if r.clicked() {
                    close(ctx, shared);
                }
                ui.with_layout(Layout::left_to_right(Align::Center), |ui| {
                    ui.label(RichText::new("Ajustes").size(14.0).family(widgets::semibold_family()).color(style::TEXT));
                });
            });
            ui.add_space(6.0);
            let mut st = lock(state);
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = 2.0;
                for (label, pane) in [("Traducción", Pane::Translate), ("Ventana", Pane::Window), ("General", Pane::General)] {
                    let r = stab(ui, label, st.pane == pane);
                    hot |= r.contains_pointer();
                    if r.clicked() {
                        st.pane = pane;
                    }
                }
            });
            let y = ui.min_rect().bottom();
            ui.painter().hline(ui.max_rect().x_range(), y, Stroke::new(1.0_f32, Color32::from_white_alpha(20)));
            hot
        });
    // `.sheet-head` is a drag region: everywhere but the ✕ and the tabs moves the window.
    widgets::caption_drag(ctx, Id::new("settings-caption"), head.response.rect, head.inner, None);
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
    widgets::field_style(ui);

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
            ui.add(input(&mut st.form.base_url).desired_width(full));
        }
        if shows_key(&p) {
            field_label(ui, "Clave API");
            let placeholder = key_placeholder(&p, &st.saved);
            let resp = ui.add(input(&mut st.key_text).password(true).hint_text(dim(placeholder)).desired_width(full));
            if resp.changed() {
                st.form.key = KeyInput::Typed(st.key_text.clone());
            }
        }
        hint(ui, key_hint(&p));
        field_label(ui, "Modelo");
        let mut load = false;
        let has_models = !st.models.is_empty();
        // Right to left so the buttons always fit and the field takes what is left.
        let (arrow, edit) = ui
            .with_layout(Layout::right_to_left(Align::Min), |ui| {
                ui.spacing_mut().item_spacing.x = 6.0;
                load = widgets::button(ui, "Cargar modelos", ButtonKind::Normal, !st.busy).clicked();
                let arrow = has_models.then(|| widgets::button(ui, if st.models_open { "▴" } else { "▾" }, ButtonKind::Normal, true));
                let edit = ui.add(input(&mut st.form.model).hint_text(dim(MODEL_PLACEHOLDER)).desired_width(ui.available_width()));
                (arrow, edit)
            })
            .inner;
        if has_models {
            model_list(ui, &mut st, &edit, arrow.as_ref());
        }
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
        ui.spacing_mut().item_spacing.x = 6.0;
        if widgets::button(ui, "Guardar traductor", ButtonKind::Primary, !st.busy).clicked() {
            save(shared, &mut st);
        }
        if widgets::button(ui, "Probar", ButtonKind::Normal, !st.busy).clicked() {
            st.result = Some(("Probando…".into(), Kind::Neutral));
            spawn_job(ui.ctx(), shared, state, &mut st, Job::Probe);
        }
    });
}

/// The searchable model list: typing in the field filters it, ▾ opens or closes it,
/// picking a model or clicking elsewhere (Enter, Esc) closes it.
fn model_list(ui: &mut Ui, st: &mut SettingsState, edit: &Response, arrow: Option<&Response>) {
    if edit.changed() {
        st.models_open = true;
    }
    if let Some(a) = arrow {
        if a.clicked() {
            st.models_open = !st.models_open;
            if st.models_open {
                edit.request_focus();
            }
        }
    }
    if !st.models_open {
        return;
    }
    let right = arrow.map_or(edit.rect.right(), |a| a.rect.right());
    let width = right - edit.rect.left();
    let shown: Vec<String> = filter_models(&st.models, &st.form.model).into_iter().map(String::from).collect();
    let mut chosen = None;
    let area = egui::Area::new(Id::new("t-models-list"))
        .order(Order::Foreground)
        .fixed_pos(edit.rect.left_bottom() + vec2(0.0, 2.0))
        .show(ui.ctx(), |ui| {
            Frame::menu(ui.style()).show(ui, |ui| {
                let margin = ui.style().spacing.menu_margin.sum().x;
                ui.set_width((width - margin).max(120.0));
                if shown.is_empty() {
                    ui.label(RichText::new("Ningún modelo coincide").size(11.5).color(style::MUTED));
                    return;
                }
                ScrollArea::vertical().max_height(240.0).auto_shrink([false, true]).show(ui, |ui| {
                    for m in &shown {
                        if ui.selectable_label(st.form.model == *m, m).clicked() {
                            chosen = Some(m.clone());
                        }
                    }
                });
            });
        });
    if let Some(m) = chosen {
        st.form.model = m;
        st.models_open = false;
        return;
    }
    let (pressed_at, enter, esc) = ui.ctx().input(|i| {
        (i.pointer.any_pressed().then(|| i.pointer.interact_pos()).flatten(), i.key_pressed(Key::Enter), i.key_pressed(Key::Escape))
    });
    let outside = pressed_at.is_some_and(|p| {
        !area.response.rect.contains(p) && !edit.rect.contains(p) && !arrow.is_some_and(|a| a.rect.contains(p))
    });
    if outside || enter || esc {
        st.models_open = false;
    }
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

    let mut hide = prefs.hide_from_capture;
    row(ui, "Ocultar de las capturas de pantalla", CAPTURE_NOTE, |ui| {
        if switch(ui, &mut hide).changed() {
            shared.config().prefs.hide_from_capture = hide;
            shared.save_config();
            shared.log.write(&format!("hide from capture {hide}"));
            shared.repaint();
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
    let mut show_log = shared.config().prefs.show_log;
    row(ui, "Registro del chat", LOG_NOTE, |ui| {
        if switch(ui, &mut show_log).changed() {
            shared.set_show_log(show_log);
        }
    });
    let current = shared.config().prefs.item_tooltips;
    row(ui, "Datos de objetos", ITEM_TIPS_NOTE, |ui| {
        widgets::field_style(ui);
        let mut value = current;
        egui::ComboBox::from_id_salt("item-tooltips").width(140.0).selected_text(value.label()).show_ui(ui, |ui| {
            for v in ItemTooltips::ALL {
                ui.selectable_value(&mut value, v, v.label());
            }
        });
        if value != current {
            shared.set_item_tooltips(value);
        }
    });
    row(ui, "Mensajes", "Borra la lista de mensajes de la ventana del chat.", |ui| {
        if widgets::button(ui, "Limpiar", ButtonKind::Normal, true).clicked() {
            shared.clear_messages();
        }
    });

    ui.add_space(6.0);
    ui.label(RichText::new("Addon").family(widgets::semibold_family()).size(12.5).color(style::TEXT));
    {
        let mut panel = shared.addon();
        addon_panel::panel(ui, shared, &mut panel, addon_panel::Mode::Settings);
    }
    separator_line(ui);

    ui.add_space(6.0);
    ui.label(RichText::new("Atajo (solo con WoW abierto)").family(widgets::semibold_family()).size(12.0).color(style::TEXT));
    ui.horizontal(|ui| {
        kbd(ui, "Ctrl+Shift+Enter");
        ui.label(RichText::new("Escribir un mensaje").size(12.0).color(style::TEXT));
    });
    separator_line(ui);

    row(ui, "Salir", "Cierra la app por completo. ✕ solo la oculta.", |ui| {
        if widgets::button(ui, "Salir de la app", ButtonKind::Danger, true).clicked() {
            tray::quit_app(shared);
        }
    });

    ui.add_space(6.0);
    ui.label(RichText::new("Acerca de").family(widgets::semibold_family()).size(12.0).color(style::TEXT));
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
            ui.label(RichText::new(title).family(widgets::semibold_family()).size(12.5).color(style::TEXT));
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

/// A text input with the bright typed text of the Electron inputs (`#eee`).
fn input(text: &mut String) -> egui::TextEdit<'_> {
    egui::TextEdit::singleline(text).text_color(widgets::INPUT_TEXT).margin(Margin::symmetric(8, 5))
}

fn dim(text: &str) -> RichText {
    RichText::new(text).color(HINT)
}

/// `kbd`: 11 px text on a rounded grey box, centered like every other control.
fn kbd(ui: &mut Ui, text: &str) {
    let font = FontId::proportional(11.0);
    let (rect, _) = ui.allocate_exact_size(vec2(widgets::text_width(ui, text, &font) + 12.0, 18.0), Sense::hover());
    ui.painter().rect_filled(rect, CornerRadius::same(4), Color32::from_white_alpha(20));
    widgets::text_in(ui.painter(), rect, text, font, style::TEXT);
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

/// `.stab`: 12 px, 5×10 px padding, rounded top corners.
fn stab(ui: &mut Ui, label: &str, on: bool) -> Response {
    let font = FontId::proportional(12.0);
    let (rect, resp) = widgets::tab_rect(ui, label, &font, 0.0);
    if on {
        widgets::tab_fill(ui, rect, 0.07);
    }
    widgets::text_in(ui.painter(), rect, label, font, widgets::tab_text_color(on, resp.hovered()));
    resp.on_hover_cursor(CursorIcon::PointingHand)
}

pub(super) fn close_button(ui: &mut Ui) -> Response {
    let (rect, resp) = ui.allocate_exact_size(vec2(28.0, 24.0), Sense::click());
    if resp.hovered() {
        ui.painter().rect_filled(rect, 5.0, style::CLOSE_HOVER);
    }
    let color = if resp.hovered() { Color32::WHITE } else { style::ICON };
    widgets::text_in(ui.painter(), rect, "✕", FontId::proportional(13.0), color);
    resp.on_hover_text("Cerrar")
}
