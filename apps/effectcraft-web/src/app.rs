//! The app: startup (storage, session restore, hooks), the eframe app and the page glue.

use std::cell::RefCell;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use effectcraft_engine::Session;

use crate::{api, audio, files, persist, worker};
use effectcraft_ui_egui::EffectcraftApp;
use serde_json::json;
use wasm_bindgen::prelude::*;

/// Work for the UI thread with access to the app (posted by async tasks: picked files…).
type Action = Box<dyn FnOnce(&mut EffectcraftApp)>;

static WORKER: AtomicBool = AtomicBool::new(false);

/// Running inside a Web Worker (no page, no downloads).
pub fn is_worker() -> bool {
    WORKER.load(Ordering::Relaxed)
}

pub(crate) fn set_worker() {
    WORKER.store(true, Ordering::Relaxed);
}

/// `rel` resolved against the page's URL (`worker.js`, `audio-worklet.js`); `""` = the page's
/// folder.
pub fn page_url(rel: &str) -> String {
    let href = web_sys::window().and_then(|w| w.location().href().ok()).unwrap_or_default();
    web_sys::Url::new_with_base(if rel.is_empty() { "./" } else { rel }, &href).map(|u| u.href()).unwrap_or_else(|_| rel.to_string())
}

thread_local! {
    static ACTIONS: RefCell<Vec<Action>> = const { RefCell::new(Vec::new()) };
    static CTX: RefCell<Option<egui::Context>> = const { RefCell::new(None) };
}

/// Run `f` on the UI thread at the next frame.
pub fn post(f: impl FnOnce(&mut EffectcraftApp) + 'static) {
    ACTIONS.with(|a| a.borrow_mut().push(Box::new(f)));
    repaint();
}

/// Ask for a UI frame.
pub fn repaint() {
    CTX.with(|c| {
        if let Some(c) = c.borrow().as_ref() {
            c.request_repaint();
        }
    });
}

/// Open a project or import media from table paths (picked or dropped files).
pub(crate) fn open_or_import(app: &mut EffectcraftApp, paths: Vec<String>) {
    let (projects, media): (Vec<String>, Vec<String>) = paths.into_iter().partition(|p| p.to_ascii_lowercase().ends_with(".ecproj"));
    if let Some(p) = projects.last()
        && let Err(e) = app.session.execute("file.open", json!({"path": p}))
    {
        app.ui.status = e.to_string();
    }
    if !media.is_empty() {
        match app.session.execute("file.import", json!({"paths": media})) {
            Ok(r) => {
                if let Some(errs) = r["errors"].as_array().filter(|e| !e.is_empty()) {
                    app.ui.status = errs.iter().filter_map(|e| e.as_str()).collect::<Vec<_>>().join("; ");
                }
            }
            Err(e) => app.ui.status = e.to_string(),
        }
    }
}

/// The eframe app: the shared EffectCraft UI plus the browser host's per-frame duties.
pub struct WebApp {
    pub app: EffectcraftApp,
    /// Keep the session snapshot (off with `?empty`).
    pub persist: bool,
}

impl eframe::App for WebApp {
    fn raw_input_hook(&mut self, ctx: &egui::Context, raw_input: &mut egui::RawInput) {
        self.app.raw_input_hook(ctx, raw_input);
    }

    fn logic(&mut self, ctx: &egui::Context, frame: &mut eframe::Frame) {
        let actions = ACTIONS.with(|a| std::mem::take(&mut *a.borrow_mut()));
        for f in actions {
            f(&mut self.app);
        }
        // Files dropped on the page: read them, then open/import.
        let dropped: Vec<web_sys::File> = ctx.input(|i| i.raw.dropped_files.iter().filter_map(|f| f.web_file().cloned()).collect());
        if !dropped.is_empty() {
            files::read_files(dropped, |paths| post(move |app| open_or_import(app, paths)));
        }
        self.app.logic(ctx, frame);
        api::poll_replies();
        // Render Queue outputs written this frame (here or by a worker) → downloads.
        files::flush_downloads();
        // What a reload comes back to.
        if self.persist {
            persist::snapshot(&self.app.session, ctx.input(|i| i.time), 1.0);
        }
        api::set_info("gpu", json!({"compositor": self.app.gpu_adapter(), "viewerOnGpu": self.app.viewer_on_gpu()}));
    }

    fn ui(&mut self, ui: &mut egui::Ui, frame: &mut eframe::Frame) {
        self.app.ui(ui, frame);
    }
}

fn query() -> Option<web_sys::UrlSearchParams> {
    web_sys::window().and_then(|w| w.location().search().ok()).and_then(|s| web_sys::UrlSearchParams::new_with_str(&s).ok())
}

fn query_flag(name: &str) -> bool {
    query().is_some_and(|p| p.has(name))
}

fn query_value(name: &str) -> Option<String> {
    query().and_then(|p| p.get(name))
}

/// A session wired for the browser: media decoding from memory, expressions, scripting, Render
/// Queue export to downloads, settings in browser storage, renders and analyses in workers.
pub fn session() -> Session {
    let pool = Arc::new(effectcraft_media::MediaPool::new());
    persist::register_media(&pool);
    Session {
        services: Arc::new(files::WebServices),
        footage: pool.clone(),
        importer: Some(Arc::new(files::WebImporter { pool })),
        expr: Some(Arc::new(effectcraft_expr::Expressions)),
        expr_check: Some(effectcraft_expr::check_syntax),
        script: Some(effectcraft_host::script::runner),
        exporter: Some(Arc::new(effectcraft_host::FileExporter { sink: Some(files::export_sink()) })),
        config: Some(Arc::new(persist::config())),
        offload: (!query_flag("noworkers")).then(|| Arc::new(worker::WorkerOffload) as Arc<dyn effectcraft_engine::offload::Offload>),
        ..Default::default()
    }
}

/// Browser file pickers in place of the desktop's file dialogs.
fn install_hooks(app: &mut EffectcraftApp) {
    app.hooks.pick_files = Some(Box::new(|exts: &[&str]| {
        let accept = exts.iter().map(|e| format!(".{e}")).collect::<Vec<_>>().join(",");
        if let Err(e) = files::pick(&accept, true, |paths| post(move |app| open_or_import(app, paths))) {
            log::warn!("file picker: {e:?}");
        }
        // the import happens when the browser hands over the files
        Vec::new()
    }));
    app.hooks.pick_open_project = Some(Box::new(|| {
        if let Err(e) = files::pick(".ecproj", false, |paths| post(move |app| open_or_import(app, paths))) {
            log::warn!("file picker: {e:?}");
        }
        None
    }));
    // Saving downloads the file: the chosen name is all a "save dialog" needs.
    app.hooks.pick_save = Some(Box::new(|name: &str| Some(files::path_for(if name.is_empty() { "Untitled Project.ecproj" } else { name }))));
    app.hooks.audio_device = Some(Box::new(audio::open));
    app.hooks.audio_devices = Some(Box::new(|| vec!["Browser default output".into()]));
}

/// Entry point (called by the page's bootstrap script once the wasm module is instantiated).
#[wasm_bindgen]
pub async fn start(canvas_id: String) -> Result<(), JsValue> {
    eframe::WebLogger::init(log::LevelFilter::Info).ok();
    let doc = web_sys::window().and_then(|w| w.document()).ok_or("no document")?;
    let canvas: web_sys::HtmlCanvasElement = doc.get_element_by_id(&canvas_id).ok_or("no canvas")?.dyn_into()?;
    let empty = query_flag("empty");
    let home = query_flag("home");
    // Browser storage: settings, recent projects, imported media, saved projects, auto-saves and
    // the last session come back.
    let t0 = js_sys::Date::now();
    let stored = match persist::load(&query_value("storage").unwrap_or_default()).await {
        Ok(n) => n,
        Err(e) => {
            log::warn!("browser storage unavailable: {e}");
            0
        }
    };
    api::set_info("storage", json!({"backend": persist::backend(), "entries": stored, "loadMs": js_sys::Date::now() - t0}));
    audio::install_unlock();
    let runner = eframe::WebRunner::new();
    runner
        .start(
            canvas,
            eframe::WebOptions::default(),
            Box::new(move |cc| {
                CTX.with(|c| *c.borrow_mut() = Some(cc.egui_ctx.clone()));
                let mut session = session();
                session.load_settings();
                let recovery = session.begin_recovery();
                // `?empty`: a blank project; `?demo`: the demo project; otherwise the last visit's
                // project (the demo on a first visit).
                let restored = !empty && !query_flag("demo") && persist::restore(&mut session);
                if !restored && !empty {
                    let _ = session.execute("file.openDemoProject", json!({}));
                }
                api::set_info("restored", json!(restored));
                let mut app = EffectcraftApp::new(session);
                app.ui.start_screen = home;
                if let Some(r) = recovery.filter(|r| !restored && r.autosave.is_some()) {
                    app.offer_recovery(r);
                }
                install_hooks(&mut app);
                let (tx, rx) = std::sync::mpsc::channel();
                api::set_sender(tx);
                app = app.with_control(rx);
                let backend = cc.wgpu_render_state.as_ref().map(|rs| format!("{:?}", rs.adapter.get_info().backend));
                api::set_info("backend", json!(backend));
                Ok(Box::new(WebApp { app, persist: !empty }))
            }),
        )
        .await?;
    // Hide the loading message once the app runs.
    if let Some(el) = doc.get_element_by_id("loading") {
        el.remove();
    }
    Ok(())
}
