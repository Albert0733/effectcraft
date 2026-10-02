//! EffectCraft desktop app.
//!
//! Usage: `effectcraft [--control <port>] [--demo|--empty|--home] [project.ecproj | media files…]`
//!
//! `--control <port>` (or `EFFECTCRAFT_CONTROL_PORT`) starts a localhost JSON-lines control server;
//! see `effectcraft_ui_egui::control` for the methods.

mod audio_out;
mod control_server;

use effectcraft_ui_egui::EffectcraftApp;
use serde_json::json;

fn main() -> eframe::Result {
    let mut control_port: Option<u16> = std::env::var("EFFECTCRAFT_CONTROL_PORT").ok().and_then(|p| p.parse().ok());
    let mut files = Vec::new();
    let mut demo = true;
    let mut home = false;
    let mut args = std::env::args().skip(1);
    while let Some(a) = args.next() {
        match a.as_str() {
            "--control" => control_port = args.next().and_then(|p| p.parse().ok()),
            "--demo" => demo = true,
            "--empty" => demo = false,
            "--home" => home = true,
            "--version" => {
                println!("effectcraft {}", env!("CARGO_PKG_VERSION"));
                return Ok(());
            }
            _ => files.push(a),
        }
    }
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_title("EffectCraft")
            .with_inner_size([1680.0, 1020.0])
            .with_min_inner_size([960.0, 600.0])
            .with_drag_and_drop(true)
            // Started for an agent (`--control`): open without taking the user's keyboard focus.
            .with_active(control_port.is_none()),
        event_loop_builder: agent_event_loop(control_port.is_some()),
        ..Default::default()
    };
    eframe::run_native(
        "EffectCraft",
        options,
        Box::new(move |cc| {
            let mut session = effectcraft_host::session();
            let project = files.iter().find(|f| f.ends_with(".ecproj")).cloned();
            if let Some(p) = project {
                if let Err(e) = session.execute("file.open", json!({"path": p})) {
                    eprintln!("effectcraft: {e}");
                }
            } else if demo {
                let _ = session.execute("file.openDemoProject", json!({}));
            }
            let media: Vec<String> = files.iter().filter(|f| !f.ends_with(".ecproj")).cloned().collect();
            if !media.is_empty()
                && let Err(e) = session.execute("file.import", json!({"paths": media}))
            {
                eprintln!("effectcraft: {e}");
            }
            let mut app = EffectcraftApp::new(session);
            app.ui.start_screen = home;
            app.hooks.pick_files = Some(Box::new(|exts: &[&str]| {
                rfd::FileDialog::new().add_filter("Media", exts).pick_files().unwrap_or_default().into_iter().map(|p| p.to_string_lossy().to_string()).collect()
            }));
            app.hooks.pick_save = Some(Box::new(|name: &str| {
                rfd::FileDialog::new().add_filter("EffectCraft Project", &["ecproj"]).set_file_name(name).save_file().map(|p| p.to_string_lossy().to_string())
            }));
            app.hooks.pick_open_project =
                Some(Box::new(|| rfd::FileDialog::new().add_filter("EffectCraft Project", &["ecproj"]).pick_file().map(|p| p.to_string_lossy().to_string())));
            app.hooks.audio_device = Some(Box::new(audio_out::open));
            if let Some(port) = control_port {
                disable_app_nap();
                let rx = control_server::start(port, cc.egui_ctx.clone());
                app = app.with_control(rx);
            }
            Ok(Box::new(app))
        }),
    )
}

/// Driven by an agent (`--control`): don't activate the app on launch, so the user's keyboard
/// focus stays where it is (macOS; winit activates ignoring other apps by default).
fn agent_event_loop(agent: bool) -> Option<eframe::EventLoopBuilderHook> {
    #[cfg(target_os = "macos")]
    if agent {
        return Some(Box::new(|b| {
            use winit::platform::macos::EventLoopBuilderExtMacOS;
            b.with_activate_ignoring_other_apps(false);
        }));
    }
    let _ = agent;
    None
}

/// Agents drive the app while its window is covered or on another Space. macOS App Nap would
/// throttle the event loop then, stalling the control channel; opt out for `--control` runs.
fn disable_app_nap() {
    #[cfg(target_os = "macos")]
    {
        use objc2_foundation::{NSActivityOptions, NSProcessInfo, NSString};
        let info = NSProcessInfo::processInfo();
        let reason = NSString::from_str("EffectCraft control channel");
        let opts = NSActivityOptions::UserInitiatedAllowingIdleSystemSleep | NSActivityOptions::LatencyCritical;
        // Leaked on purpose: the activity lasts for the life of the process.
        std::mem::forget(info.beginActivityWithOptions_reason(opts, &reason));
    }
}
