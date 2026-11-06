//! Installing the bundled addon from the app: the first-run dialog (shown once) and the
//! same panel in Ajustes → General → Addon. Detection and install rules live in
//! `crate::addon`; this is the drawing and the user's choices.

use std::collections::HashSet;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use eframe::egui::{
    self, vec2, Align, Color32, Frame, Id, Key, Layout, Margin, RichText, ScrollArea, Stroke, Ui, ViewportBuilder,
    ViewportClass, ViewportId,
};

use crate::addon::detect::{detect, normalize_key, resolve_user_folder, AddonState, Detection, Flavor, WowSystem};
use crate::addon::install::{install_bundled, InstallResult};
use crate::addon::prefs::{self as addon_prefs, AddonPrefs};
use crate::addon::system::{pick_folder, WindowsSystem};
use crate::addon::{is_older, needs_update, VERSION};

use super::settings;
use super::shared::Shared;
use super::style;
use super::widgets::{self, ButtonKind};
use super::win;

pub const UPDATE_HINT: &str = "Actualiza el addon (Ajustes → General → Addon)";
pub const DIALOG_TITLE: &str = "Addon · WoW Chat Translator";
const RELOAD_NOTE: &str = "Escribe /reload en WoW para cargar el addon.";
const OK: Color32 = Color32::from_rgb(0x86, 0xef, 0xac);
const ERR: Color32 = Color32::from_rgb(0xfc, 0xa5, 0xa5);
const SHEET_BG: Color32 = Color32::from_rgb(0x0f, 0x14, 0x1d);
const WIDTH: f32 = 480.0;
const HEIGHT: f32 = 440.0;

fn dialog_id() -> ViewportId {
    ViewportId::from_hash_of("wct-addon-dialog")
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Mode {
    FirstRun,
    Settings,
}

/// The panel's state, shared by the dialog and the settings window.
pub struct AddonPanel {
    prefs_path: PathBuf,
    pub prefs: AddonPrefs,
    detection: Option<Detection>,
    checked: HashSet<String>,
    replace_link: HashSet<String>,
    results: Vec<(String, InstallResult)>,
    note: Option<(String, bool)>,
    picked: Arc<Mutex<Option<Option<PathBuf>>>>,
    picking: bool,
    dialog_decided: bool,
    dialog_open: bool,
    dialog_capture_hidden: Option<bool>,
    /// Screenshot run: answers are not saved.
    dev: bool,
}

impl AddonPanel {
    pub fn new(prefs_path: PathBuf) -> Self {
        Self {
            prefs: addon_prefs::load(&prefs_path),
            prefs_path,
            detection: None,
            checked: HashSet::new(),
            replace_link: HashSet::new(),
            results: Vec::new(),
            note: None,
            picked: Arc::new(Mutex::new(None)),
            picking: false,
            dialog_decided: false,
            dialog_open: false,
            dialog_capture_hidden: None,
            dev: false,
        }
    }

    /// Runs detection again; with `preselect`, ticks the last played flavor (and in
    /// the settings panel every flavor that already has the addon).
    fn refresh(&mut self, preselect: bool, include_installed: bool) {
        let d = detect(&WindowsSystem, &self.prefs.extra_folders);
        if preselect {
            self.checked = d
                .flavors()
                .filter(|f| d.is_last_played(f) || (include_installed && matches!(f.addon, AddonState::Installed { .. })))
                .map(|f| normalize_key(&f.path))
                .collect();
            if self.checked.is_empty() && d.flavors().count() == 1 {
                self.checked = d.flavors().map(|f| normalize_key(&f.path)).collect();
            }
        }
        self.detection = Some(d);
    }

    /// Settings opened: start from a fresh detection.
    pub fn reset(&mut self) {
        self.detection = None;
        self.results.clear();
        self.note = None;
    }

    fn save_prefs(&self) {
        if !self.dev {
            let _ = addon_prefs::save(&self.prefs_path, &self.prefs);
        }
    }

    fn mark_prompt_done(&mut self) {
        self.prefs.addon_prompt_done = true;
        self.save_prefs();
    }

    fn install_checked(&mut self, game_running: bool) {
        let Some(d) = &self.detection else { return };
        let targets: Vec<Flavor> = d.flavors().filter(|f| self.checked.contains(&normalize_key(&f.path))).cloned().collect();
        self.results = targets
            .iter()
            .map(|f| (f.name.clone(), install_bundled(&f.path, self.replace_link.contains(&normalize_key(&f.path)))))
            .collect();
        let any_ok = self.results.iter().any(|(_, r)| r.is_ok());
        self.note = (any_ok && game_running).then(|| (RELOAD_NOTE.to_string(), true));
        self.refresh(false, false);
    }

    /// A folder came back from the picker.
    fn take_picked(&mut self) {
        let picked = self.picked.lock().unwrap_or_else(|e| e.into_inner()).take();
        let Some(result) = picked else { return };
        self.picking = false;
        let Some(folder) = result else { return };
        let Some(root) = resolve_user_folder(&folder) else {
            self.note = Some((format!("No se encontró WoW en {}", folder.display()), false));
            return;
        };
        if !self.prefs.extra_folders.iter().any(|p| normalize_key(p) == normalize_key(&root)) {
            self.prefs.extra_folders.push(root.clone());
            self.save_prefs();
        }
        self.refresh(false, false);
        if let Some(d) = &self.detection {
            let key = normalize_key(&root);
            for f in d.flavors().filter(|f| f.path.parent().is_some_and(|p| normalize_key(p) == key)) {
                self.checked.insert(normalize_key(&f.path));
            }
        }
        self.note = None;
    }

    fn start_pick(&mut self, ctx: &egui::Context) {
        self.picking = true;
        let owner = unsafe { windows::Win32::UI::WindowsAndMessaging::GetForegroundWindow() }.0 as isize;
        let slot = self.picked.clone();
        let ctx = ctx.clone();
        let _ = std::thread::Builder::new().name("folder-picker".into()).spawn(move || {
            let folder = pick_folder(Some(owner).filter(|h| *h != 0), "Elige la carpeta de World of Warcraft");
            *slot.lock().unwrap_or_else(|e| e.into_inner()) = Some(folder);
            for id in [dialog_id(), settings::viewport_id(), ViewportId::ROOT] {
                ctx.request_repaint_of(id);
            }
        });
    }
}

/// Developer screenshots only: opens the first-run dialog whatever the prefs say.
pub fn dev_open(shared: &Shared) {
    let mut p = shared.addon();
    p.dev = true;
    p.dialog_decided = true;
    p.dialog_open = true;
    p.refresh(true, false);
}

/// The addon in the flavor being played (else the last played) is missing or older
/// than the bundled one. A development link never counts.
pub fn stale_addon(shared: &Shared) -> bool {
    let extra = shared.addon().prefs.extra_folders.clone();
    let d = detect(&WindowsSystem, &extra);
    let running: Vec<String> =
        WindowsSystem.running_wow_exes().iter().filter_map(|e| e.parent().map(normalize_key)).collect();
    let flavor = d.flavors().find(|f| running.contains(&normalize_key(&f.path))).or_else(|| d.last_played_flavor());
    flavor.is_some_and(|f| needs_update(&f.addon, VERSION))
}

fn status_of(addon: &AddonState) -> (String, Color32) {
    match addon {
        AddonState::Missing => ("No instalado".into(), style::MUTED),
        AddonState::Linked { .. } => ("Enlazado a una copia de desarrollo".into(), style::AMBER),
        AddonState::Installed { version: Some(v) } if is_older(Some(v), VERSION) => (format!("Instalado {v} · hay {VERSION}"), style::AMBER),
        AddonState::Installed { version: Some(v) } => (format!("Al día ({v})"), OK),
        AddonState::Installed { version: None } => ("Instalado (versión desconocida)".into(), style::AMBER),
    }
}

fn hint(ui: &mut Ui, text: &str) {
    ui.add(egui::Label::new(RichText::new(text).size(10.5).color(settings::HINT)).wrap());
}

/// The flavor list, the folder picker and the install button.
pub fn panel(ui: &mut Ui, shared: &Shared, p: &mut AddonPanel, mode: Mode) {
    p.take_picked();
    if p.detection.is_none() {
        p.refresh(true, mode == Mode::Settings);
    }
    if p.picking {
        ui.ctx().request_repaint_after(Duration::from_millis(250));
    }
    let flavors: Vec<Flavor> = p.detection.as_ref().map(|d| d.flavors().cloned().collect()).unwrap_or_default();
    let last_played: Option<String> = p.detection.as_ref().and_then(|d| d.last_played_flavor()).map(|f| normalize_key(&f.path));

    hint(ui, &format!("Versión incluida en la app: {VERSION}. Marca las versiones de WoW donde quieres el addon."));
    for root in p.detection.iter().flat_map(|d| d.roots.iter()).filter(|r| !r.flavors.is_empty()) {
        hint(ui, &format!("Carpeta de WoW: {}", root.path.display()));
    }
    ui.add_space(4.0);
    if flavors.is_empty() {
        ui.label(RichText::new("No se encontró WoW en este PC. Usa «Elegir otra carpeta…».").size(12.0).color(style::AMBER));
    } else {
        if flavors.len() > 1 {
            let mut all = flavors.iter().all(|f| p.checked.contains(&normalize_key(&f.path)));
            if widgets::checkbox(ui, &mut all, "Marcar todas", egui::FontId::proportional(12.0), style::MUTED).changed() {
                for f in &flavors {
                    let key = normalize_key(&f.path);
                    if all {
                        p.checked.insert(key);
                    } else {
                        p.checked.remove(&key);
                    }
                }
            }
        }
        for f in &flavors {
            flavor_row(ui, p, f, last_played.as_deref() == Some(normalize_key(&f.path).as_str()));
        }
    }

    if !p.results.is_empty() {
        ui.add_space(6.0);
        for (name, r) in &p.results {
            let color = match r {
                r if r.is_ok() => OK,
                InstallResult::SkippedLinked { .. } => style::AMBER,
                _ => ERR,
            };
            ui.add(egui::Label::new(RichText::new(format!("{name}: {}", r.message())).size(11.5).color(color)).wrap());
        }
    }
    if let Some((text, ok)) = &p.note {
        ui.add_space(4.0);
        ui.add(egui::Label::new(RichText::new(text).size(12.0).color(if *ok { style::AMBER } else { ERR })).wrap());
    }

    ui.add_space(8.0);
    let any_checked = flavors.iter().any(|f| p.checked.contains(&normalize_key(&f.path)));
    let mut close = false;
    ui.with_layout(Layout::right_to_left(Align::Min), |ui| {
        ui.spacing_mut().item_spacing.x = 6.0;
        let label = match mode {
            Mode::FirstRun => "Instalar addon",
            Mode::Settings => "Reinstalar / actualizar",
        };
        if widgets::button(ui, label, ButtonKind::Primary, any_checked).clicked() {
            p.install_checked(shared.game_running());
            if mode == Mode::FirstRun {
                p.mark_prompt_done();
            }
        }
        if widgets::button(ui, "Elegir otra carpeta…", ButtonKind::Normal, !p.picking).clicked() {
            p.start_pick(ui.ctx());
        }
        if mode == Mode::FirstRun {
            ui.with_layout(Layout::left_to_right(Align::Min), |ui| {
                let label = if p.results.is_empty() { "Ahora no" } else { "Listo" };
                close = widgets::button(ui, label, ButtonKind::Normal, true).clicked();
            });
        }
    });
    if close {
        p.mark_prompt_done();
        p.dialog_open = false;
        shared.repaint();
    }
}

fn flavor_row(ui: &mut Ui, p: &mut AddonPanel, f: &Flavor, last_played: bool) {
    let key = normalize_key(&f.path);
    ui.add_space(2.0);
    let row = ui.horizontal(|ui| {
        let mut on = p.checked.contains(&key);
        let title = if last_played { format!("{} · último jugado", f.name) } else { f.name.clone() };
        if widgets::checkbox(ui, &mut on, &title, widgets::semibold(12.5), style::TEXT).changed() {
            if on {
                p.checked.insert(key.clone());
            } else {
                p.checked.remove(&key);
            }
        }
        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
            let (text, color) = status_of(&f.addon);
            ui.label(RichText::new(text).size(11.5).color(color));
        });
    });
    row.response.on_hover_text(f.path.display().to_string());
    if let AddonState::Linked { target, .. } = &f.addon {
        ui.horizontal(|ui| {
            ui.add_space(21.0);
            let mut replace = p.replace_link.contains(&key);
            let label = "Reemplazar enlace (la copia de desarrollo no se borra)";
            if widgets::checkbox(ui, &mut replace, label, egui::FontId::proportional(11.5), style::MUTED).changed() {
                if replace {
                    p.replace_link.insert(key.clone());
                } else {
                    p.replace_link.remove(&key);
                }
            }
        });
        if let Some(t) = target {
            ui.horizontal(|ui| {
                ui.add_space(21.0);
                hint(ui, &format!("→ {}", t.display()));
            });
        }
    }
}

/// Shows the first-run dialog until it is answered (once per install of the app).
pub fn show_first_run(ctx: &egui::Context, shared: &Arc<Shared>, icon: Option<Arc<egui::IconData>>) {
    {
        let mut p = shared.addon();
        if !p.dialog_decided {
            p.dialog_decided = true;
            // Screenshot runs only show it when asked to (WCT_DEV_STATE=addon).
            p.dialog_open = !p.prefs.addon_prompt_done && !win::dev_capture();
        }
        if !p.dialog_open {
            return;
        }
    }
    let (wx, wy, ww, wh) = win::work_area_points();
    let mut builder = ViewportBuilder::default()
        .with_title(DIALOG_TITLE)
        .with_decorations(false)
        .with_resizable(true)
        .with_inner_size([WIDTH, HEIGHT])
        .with_min_inner_size([400.0, 300.0])
        .with_position([wx + (ww - WIDTH) / 2.0, wy + (wh - HEIGHT) / 2.0])
        .with_always_on_top();
    if let Some(icon) = icon {
        builder = builder.with_icon(icon);
    }
    let shared = shared.clone();
    ctx.show_viewport_deferred(dialog_id(), builder, move |ctx, class| {
        if class == ViewportClass::Embedded {
            return;
        }
        let mut p = shared.addon();
        if ctx.input(|i| i.viewport().close_requested() || i.key_pressed(Key::Escape)) {
            p.mark_prompt_done();
            p.dialog_open = false;
            shared.repaint();
            return;
        }
        let hide = shared.config().prefs.hide_from_capture && !win::dev_capture();
        if p.dialog_capture_hidden != Some(hide) {
            let h = win::find_own_window(DIALOG_TITLE);
            if h.is_set() {
                h.set_capture_hidden(hide);
                p.dialog_capture_hidden = Some(hide);
            }
        }
        let head = egui::TopBottomPanel::top("addon-head")
            .show_separator_line(false)
            .frame(Frame::new().fill(SHEET_BG).inner_margin(Margin { left: 14, right: 8, top: 8, bottom: 6 }))
            .show(ctx, |ui| {
                let mut hot = false;
                ui.allocate_ui_with_layout(vec2(ui.available_width(), 24.0), Layout::right_to_left(Align::Center), |ui| {
                    let r = settings::close_button(ui);
                    hot |= r.contains_pointer();
                    if r.clicked() {
                        p.mark_prompt_done();
                        p.dialog_open = false;
                        shared.repaint();
                    }
                    ui.with_layout(Layout::left_to_right(Align::Center), |ui| {
                        ui.label(RichText::new("Instalar el addon en WoW").size(14.0).family(widgets::semibold_family()).color(style::TEXT));
                    });
                });
                let y = ui.min_rect().bottom() + 5.0;
                ui.painter().hline(ui.max_rect().x_range(), y, Stroke::new(1.0_f32, Color32::from_white_alpha(20)));
                hot
            });
        widgets::caption_drag(ctx, Id::new("addon-caption"), head.response.rect, head.inner, None);
        egui::CentralPanel::default()
            .frame(Frame::new().fill(SHEET_BG).inner_margin(Margin { left: 14, right: 14, top: 10, bottom: 12 }))
            .show(ctx, |ui| {
                ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| {
                    ui.spacing_mut().item_spacing.y = 4.0;
                    ui.add(
                        egui::Label::new(
                            RichText::new("La app lee el chat a través de un addon de WoW. ¿En qué versiones de WoW lo instalamos?")
                                .size(12.5)
                                .color(style::TEXT),
                        )
                        .wrap(),
                    );
                    ui.add_space(4.0);
                    panel(ui, &shared, &mut p, Mode::FirstRun);
                    ui.add_space(6.0);
                    hint(ui, "Puedes hacerlo más tarde en Ajustes → General → Addon.");
                });
            });
    });
}
