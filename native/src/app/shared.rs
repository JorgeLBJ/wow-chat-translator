//! State shared by the UI thread, the strip reader and the translation workers.
//! Locks are held only for quick in-memory work: never across capture or HTTP.

use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicIsize, Ordering};
use std::sync::mpsc::Sender;
use std::sync::{Arc, Mutex, MutexGuard, OnceLock, RwLock};
use std::time::Instant;

use eframe::egui;

use crate::core::pipeline::{Job, Pipeline, Translation, Translator, UiEvent};
use crate::core::prefs::{self, Config};
use crate::strip::Message;

use super::debug_log::DebugLog;
use super::instance::InstanceGuard;
use super::messages::MessageList;
use super::style::{hhmm, ReaderState};
use super::win::{utc_offset_minutes_at, Hwnd};

pub const TARGET_LANG: &str = "es";

/// Pipeline plus the list it feeds; events are applied as soon as they happen, so the
/// list stays current (and capped) even while the window is hidden.
pub struct Model {
    pub pipeline: Pipeline,
    pub list: MessageList,
}

impl Model {
    fn sync(&mut self, log: &DebugLog) {
        for event in self.pipeline.take_events() {
            if let UiEvent::Add(item) = &event {
                log.write(&format!(
                    "ui {{\"id\":{},\"tab\":{:?},\"shown\":{}}}",
                    item.id,
                    item.tab.unwrap_or(""),
                    self.list.is_visible(item)
                ));
            }
            self.list.apply(event, &|ms| hhmm(ms, utc_offset_minutes_at(ms)));
        }
    }
}

/// What the footer and title bar show.
pub struct StatusInfo {
    pub reader: ReaderState,
    pub text: String,
    pub translator: String,
    /// A short-lived message (e.g. "Ajustes: próximamente") and when it expires.
    pub transient: Option<(String, Instant)>,
}

pub struct Shared {
    pub model: Mutex<Model>,
    pub config: Mutex<Config>,
    pub config_path: PathBuf,
    pub log: DebugLog,
    pub status: Mutex<StatusInfo>,
    translator: RwLock<Arc<dyn Translator + Send + Sync>>,
    jobs: Sender<Job>,
    hwnd: AtomicIsize,
    /// Hidden with ✕: strip presence must not show it again until the user does.
    user_hidden: AtomicBool,
    /// Wow.exe is running (the global hotkey exists only then).
    game_running: AtomicBool,
    /// Ctrl+Shift+Enter was pressed: the compose box must take the keyboard focus.
    compose_focus: AtomicBool,
    /// The settings window is open (single instance).
    settings_open: AtomicBool,
    ctx: OnceLock<egui::Context>,
    pub instance: Option<InstanceGuard>,
}

fn lock<T>(m: &Mutex<T>) -> MutexGuard<'_, T> {
    m.lock().unwrap_or_else(|e| e.into_inner())
}

impl Shared {
    pub fn new(
        config: Config,
        config_path: PathBuf,
        log: DebugLog,
        translator: Arc<dyn Translator + Send + Sync>,
        status: StatusInfo,
        jobs: Sender<Job>,
        instance: Option<InstanceGuard>,
    ) -> Self {
        let mut pipeline = Pipeline::new(TARGET_LANG);
        pipeline.set_paused(config.prefs.translation_paused);
        Self {
            model: Mutex::new(Model { pipeline, list: MessageList::default() }),
            config: Mutex::new(config),
            config_path,
            log,
            status: Mutex::new(status),
            translator: RwLock::new(translator),
            jobs,
            hwnd: AtomicIsize::new(0),
            user_hidden: AtomicBool::new(false),
            game_running: AtomicBool::new(false),
            compose_focus: AtomicBool::new(false),
            settings_open: AtomicBool::new(false),
            ctx: OnceLock::new(),
            instance,
        }
    }

    pub fn model(&self) -> MutexGuard<'_, Model> {
        lock(&self.model)
    }

    pub fn config(&self) -> MutexGuard<'_, Config> {
        lock(&self.config)
    }

    pub fn status(&self) -> MutexGuard<'_, StatusInfo> {
        lock(&self.status)
    }

    pub fn set_context(&self, ctx: egui::Context) {
        let _ = self.ctx.set(ctx);
    }

    pub fn set_hwnd(&self, hwnd: Hwnd) {
        self.hwnd.store(hwnd.0, Ordering::SeqCst);
    }

    pub fn hwnd(&self) -> Hwnd {
        Hwnd(self.hwnd.load(Ordering::SeqCst))
    }

    /// Repaints the overlay (also when called from the settings window's frame).
    pub fn repaint(&self) {
        if let Some(ctx) = self.ctx.get() {
            ctx.request_repaint_of(egui::ViewportId::ROOT);
        }
    }

    pub fn translator(&self) -> Arc<dyn Translator + Send + Sync> {
        self.translator.read().unwrap_or_else(|e| e.into_inner()).clone()
    }

    /// Swaps the translator (settings saved); jobs already running finish with the old one.
    pub fn set_translator(&self, translator: Arc<dyn Translator + Send + Sync>, label: String) {
        *self.translator.write().unwrap_or_else(|e| e.into_inner()) = translator;
        self.status().translator = label;
        self.repaint();
    }

    /// Writes the config file; a failure is logged, never fatal.
    pub fn save_config(&self) {
        let config = self.config().clone();
        if let Err(e) = prefs::save(&self.config_path, &config) {
            self.log.write(&format!("config save failed: {e}"));
        }
    }

    // Pipeline -----------------------------------------------------------------------

    pub fn add_messages(&self, messages: &[Message]) {
        let jobs = {
            let mut model = self.model();
            model.pipeline.add_messages(messages);
            model.sync(&self.log);
            model.pipeline.take_jobs()
        };
        self.dispatch(jobs);
        self.repaint();
    }

    /// Click on a paused or failed message.
    pub fn translate_by_id(&self, id: u64) {
        let jobs = {
            let mut model = self.model();
            if !model.pipeline.translate_by_id(id) {
                return;
            }
            model.list.mark_pending(id);
            model.sync(&self.log);
            model.pipeline.take_jobs()
        };
        self.dispatch(jobs);
        self.repaint();
    }

    pub fn complete(&self, id: u64, result: Result<Translation, String>) {
        {
            let mut model = self.model();
            model.pipeline.complete(id, result);
            model.sync(&self.log);
        }
        self.repaint();
    }

    pub fn set_paused(&self, paused: bool) {
        self.model().pipeline.set_paused(paused);
        self.config().prefs.translation_paused = paused;
        self.save_config();
    }

    fn dispatch(&self, jobs: Vec<Job>) {
        for job in jobs {
            let id = job.id;
            if self.jobs.send(job).is_err() {
                self.complete(id, Err("sin traductor".into()));
            }
        }
    }

    /// "Limpiar mensajes": empties the overlay list.
    pub fn clear_messages(&self) {
        self.model().list.clear();
        self.log.write("messages cleared");
        self.repaint();
    }

    // Game, hotkey, settings -------------------------------------------------------------

    pub fn set_game_running(&self, running: bool) {
        self.game_running.store(running, Ordering::SeqCst);
    }

    pub fn game_running(&self) -> bool {
        self.game_running.load(Ordering::SeqCst)
    }

    /// Ctrl+Shift+Enter: show and activate the overlay, then focus the compose box.
    pub fn request_compose_focus(&self) {
        self.compose_focus.store(true, Ordering::SeqCst);
        self.show_by_user();
    }

    pub fn take_compose_focus(&self) -> bool {
        self.compose_focus.swap(false, Ordering::SeqCst)
    }

    pub fn settings_open(&self) -> bool {
        self.settings_open.load(Ordering::SeqCst)
    }

    pub fn set_settings_open(&self, open: bool) {
        self.settings_open.store(open, Ordering::SeqCst);
        self.repaint();
    }

    pub fn ctx(&self) -> Option<&egui::Context> {
        self.ctx.get()
    }

    // Window visibility ---------------------------------------------------------------

    /// Strip presence (through the gate) changed.
    pub fn on_presence(&self, present: bool) {
        self.log.write(&format!("presence {present}"));
        if present {
            if !self.user_hidden.load(Ordering::SeqCst) {
                self.reveal(false);
            }
        } else if !super::win::dev_capture() {
            // Screenshot runs keep the window up even without WoW's strip.
            self.hwnd().hide();
        }
    }

    /// Tray "Mostrar ventana", tray click, a second launch.
    pub fn show_by_user(&self) {
        self.user_hidden.store(false, Ordering::SeqCst);
        self.reveal(true);
    }

    /// The ✕ button / Alt+F4: hide and stay hidden until the user shows it.
    pub fn hide_by_user(&self) {
        self.user_hidden.store(true, Ordering::SeqCst);
        self.hwnd().hide();
    }

    /// The game started: a new session may show the window again.
    pub fn reset_user_hidden(&self) {
        self.user_hidden.store(false, Ordering::SeqCst);
    }

    fn reveal(&self, activate: bool) {
        let hwnd = self.hwnd();
        hwnd.set_opacity(self.config().prefs.opacity);
        if activate {
            hwnd.show_active();
        } else {
            hwnd.show_inactive();
        }
        self.repaint();
    }

    // Status ----------------------------------------------------------------------------

    pub fn set_reader(&self, state: ReaderState) {
        let changed = {
            let mut s = self.status();
            let changed = s.reader != state;
            s.reader = state;
            changed
        };
        if changed {
            self.repaint();
        }
    }

    pub fn set_status_text(&self, text: &str) {
        let changed = {
            let mut s = self.status();
            let changed = s.text != text;
            s.text = text.to_string();
            changed
        };
        if changed {
            self.repaint();
        }
    }
}
