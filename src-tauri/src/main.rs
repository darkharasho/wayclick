// Prevent an extra console window on Windows in release.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

use std::{
    sync::Mutex,
    thread::JoinHandle,
    time::{Duration, SystemTime},
};

use serde::{Deserialize, Serialize};
use tauri::{AppHandle, Emitter, Manager, State, WebviewUrl, WebviewWindowBuilder};

mod global_shortcuts;

use wayclick_input::{
    ClickConfig, ClickEngine, ClickKind, ClosedLoopPositioner, CursorReader, HoldController,
    HoldTarget, Keycode, KwinCursorReader, MouseButton, Repeat, StopFlag, Target, VirtualKeyboard,
    VirtualMouse,
};

/// Configuration sent from the UI for one run.
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
struct RunConfig {
    interval_ms: u64,
    button: String,
    action: String,     // "click" | "hold"
    click_kind: String, // "single" | "double"
    hold_key: Option<String>,
    repeat: Option<u64>,      // None = infinite
    position: Option<[i32; 2]>, // None = follow cursor
    jitter_ms: u64,
    reposition_each_click: bool,
}

fn button_from(s: &str) -> MouseButton {
    match s {
        "right" => MouseButton::Right,
        "middle" => MouseButton::Middle,
        _ => MouseButton::Left,
    }
}

#[derive(Default)]
struct Running {
    stop: Option<StopFlag>,
    handle: Option<JoinHandle<()>>,
}

struct AppState(Mutex<Running>);

/// What the engine is currently doing, mirrored to the UI.
#[derive(Serialize, Clone)]
#[serde(rename_all = "camelCase")]
struct StatusEvent {
    phase: String, // "arming" | "running" | "stopped"
}

fn emit_status(app: &AppHandle, phase: &str) {
    let _ = app.emit("engine:status", StatusEvent { phase: phase.into() });
}

fn seed() -> u64 {
    SystemTime::now()
        .duration_since(SystemTime::UNIX_EPOCH)
        .map(|d| d.as_nanos() as u64)
        .unwrap_or(0x9E3779B97F4A7C15)
        | 1
}

/// The worker that owns the virtual devices and runs until `stop` is set.
fn run_worker(cfg: RunConfig, stop: StopFlag, app: AppHandle) {
    eprintln!(
        "[wayclick] worker start: action={} position={:?} interval={}ms",
        cfg.action, cfg.position, cfg.interval_ms
    );
    let result = (|| -> wayclick_input::Result<()> {
        eprintln!("[wayclick] creating virtual mouse…");
        let mouse = VirtualMouse::create()?;
        eprintln!("[wayclick] mouse created; emitting running");
        emit_status(&app, "running");

        if cfg.action == "hold" {
            let keyboard = VirtualKeyboard::create()?;
            let ctrl = HoldController::new(&mouse, &keyboard);
            let target = match cfg.hold_key.as_deref().and_then(Keycode::from_name) {
                Some(k) => HoldTarget::Key(k),
                None => HoldTarget::Mouse(button_from(&cfg.button)),
            };
            ctrl.hold(target)?;
            while !stop.is_stopped() {
                std::thread::sleep(Duration::from_millis(40));
            }
            ctrl.release(target)?;
            return Ok(());
        }

        // Click action.
        let reader = KwinCursorReader::new()?;
        let positioner = ClosedLoopPositioner::new(&mouse, &reader);
        let engine = ClickEngine::new(&mouse, Some(&positioner));

        let click_cfg = ClickConfig {
            button: button_from(&cfg.button),
            kind: if cfg.click_kind == "double" { ClickKind::Double } else { ClickKind::Single },
            interval: Duration::from_millis(cfg.interval_ms),
            jitter: Duration::from_millis(cfg.jitter_ms),
            repeat: cfg.repeat.map(Repeat::Count).unwrap_or(Repeat::Infinite),
            target: match cfg.position {
                Some([x, y]) => Target::Fixed { x, y },
                None => Target::FollowCursor,
            },
            hold: Duration::from_millis(20),
            double_gap: Duration::from_millis(40),
            reposition_each_click: cfg.reposition_each_click,
        };
        engine.run(&click_cfg, &stop, seed())?;
        Ok(())
    })();

    if let Err(e) = result {
        eprintln!("[wayclick] worker ERROR: {e}");
        let _ = app.emit("engine:error", e.to_string());
    }
    eprintln!("[wayclick] worker stopped");
    emit_status(&app, "stopped");
}

#[tauri::command]
fn start(state: State<AppState>, app: AppHandle, config: RunConfig) -> Result<(), String> {
    eprintln!("[wayclick] start command invoked");
    let mut running = state.0.lock().unwrap();
    if running.handle.is_some() {
        eprintln!("[wayclick] start ignored — already running");
        return Ok(()); // already running
    }
    let stop = StopFlag::new();
    emit_status(&app, "arming");
    let worker_stop = stop.clone();
    let worker_app = app.clone();
    let handle = std::thread::spawn(move || run_worker(config, worker_stop, worker_app));
    running.stop = Some(stop);
    running.handle = Some(handle);
    Ok(())
}

#[tauri::command]
fn stop(state: State<AppState>) -> Result<(), String> {
    let (stop, handle) = {
        let mut running = state.0.lock().unwrap();
        (running.stop.take(), running.handle.take())
    };
    if let Some(s) = stop {
        s.stop();
    }
    if let Some(h) = handle {
        let _ = h.join();
    }
    Ok(())
}

#[tauri::command]
fn is_running(state: State<AppState>) -> bool {
    state.0.lock().unwrap().handle.is_some()
}

/// First-run permission state for `/dev/uinput`.
#[derive(Serialize, Clone)]
#[serde(rename_all = "camelCase")]
struct AccessStatus {
    /// The app can write events right now — nothing more to do.
    writable: bool,
    /// The uinput kernel device node exists (module loaded).
    module_loaded: bool,
    /// Our udev rule is installed.
    rule_installed: bool,
    /// The current user is in the `input` group (takes effect after re-login).
    in_group: bool,
}

fn user_in_input_group() -> bool {
    std::process::Command::new("id")
        .arg("-nG")
        .output()
        .ok()
        .map(|o| {
            String::from_utf8_lossy(&o.stdout)
                .split_whitespace()
                .any(|g| g == "input")
        })
        .unwrap_or(false)
}

#[tauri::command]
fn access_status() -> AccessStatus {
    let writable = std::fs::OpenOptions::new()
        .write(true)
        .open("/dev/uinput")
        .is_ok();
    AccessStatus {
        writable,
        module_loaded: std::path::Path::new("/dev/uinput").exists(),
        rule_installed: std::path::Path::new("/etc/udev/rules.d/99-wayclick.rules").exists(),
        in_group: user_in_input_group(),
    }
}

/// Run the one-time privileged setup: load uinput, install a udev rule granting
/// the `input` group access, and add the user to that group. Prompts for the
/// password via the system polkit agent. Group membership takes effect after the
/// user logs out and back in.
#[tauri::command]
fn grant_access() -> Result<(), String> {
    let user = std::env::var("USER")
        .or_else(|_| std::env::var("LOGNAME"))
        .map_err(|_| "could not determine the current user".to_string())?;

    let script = format!(
        r#"set -e
modprobe uinput || true
echo uinput > /etc/modules-load.d/uinput.conf
cat > /etc/udev/rules.d/99-wayclick.rules <<'RULE'
KERNEL=="uinput", GROUP="input", MODE="0660", OPTIONS+="static_node=uinput"
RULE
udevadm control --reload-rules
udevadm trigger /dev/uinput || udevadm trigger
usermod -aG input {user}
"#
    );

    let status = std::process::Command::new("pkexec")
        .arg("sh")
        .arg("-c")
        .arg(&script)
        .status()
        .map_err(|e| format!("could not launch pkexec: {e}"))?;

    if status.success() {
        Ok(())
    } else {
        Err("Setup was cancelled or failed.".into())
    }
}

/// Open the system shortcut settings so the user can view/rebind the hotkey.
/// With the portal model the compositor owns the binding, so "rebind" lives in
/// the desktop's own settings rather than inside the app.
#[tauri::command]
fn open_shortcut_settings() {
    // Best-effort across desktops; KDE first.
    for (cmd, args) in [
        ("systemsettings", vec!["kcm_keys"]),
        ("systemsettings5", vec!["kcm_keys"]),
        ("kcmshell6", vec!["kcm_keys"]),
        ("kcmshell5", vec!["kcm_keys"]),
    ] {
        if std::process::Command::new(cmd).args(&args).spawn().is_ok() {
            return;
        }
    }
}

/// Open the fullscreen transparent overlay used to pick a fixed click point.
/// It spans the bounding box of all monitors so any pixel is reachable.
#[tauri::command]
fn pick_point(app: AppHandle) -> Result<(), String> {
    let main = app
        .get_webview_window("main")
        .ok_or("main window missing")?;
    let monitors = main.available_monitors().map_err(|e| e.to_string())?;

    let (mut minx, mut miny, mut maxx, mut maxy) = (i32::MAX, i32::MAX, i32::MIN, i32::MIN);
    for m in &monitors {
        let p = m.position();
        let s = m.size();
        minx = minx.min(p.x);
        miny = miny.min(p.y);
        maxx = maxx.max(p.x + s.width as i32);
        maxy = maxy.max(p.y + s.height as i32);
    }
    if minx == i32::MAX {
        return Err("no monitors found".into());
    }

    let builder = WebviewWindowBuilder::new(
        &app,
        "overlay",
        WebviewUrl::App("index.html?overlay=1".into()),
    )
    .decorations(false)
    .transparent(true)
    .always_on_top(true)
    .skip_taskbar(true)
    .resizable(false)
    .position(minx as f64, miny as f64)
    .inner_size((maxx - minx) as f64, (maxy - miny) as f64);

    let win = builder.build().map_err(|e| e.to_string())?;
    let _ = win.set_focus();
    Ok(())
}

/// Called by the overlay when the user clicks: read the true cursor position
/// (same coordinate space the positioner uses), report it, and close the overlay.
#[tauri::command]
fn point_picked(app: AppHandle) -> Result<(), String> {
    let reader = KwinCursorReader::new().map_err(|e| e.to_string())?;
    let (x, y) = reader.position().map_err(|e| e.to_string())?;
    let _ = app.emit("point:picked", [x, y]);
    if let Some(w) = app.get_webview_window("overlay") {
        let _ = w.close();
    }
    Ok(())
}

#[tauri::command]
fn cancel_pick(app: AppHandle) {
    if let Some(w) = app.get_webview_window("overlay") {
        let _ = w.close();
    }
}

fn main() {
    // WebKitGTK's DMABUF renderer fails to allocate GBM buffers on some
    // Wayland/GPU setups, leaving a blank window. Force it off before the
    // webview initializes unless the user already set a preference.
    if std::env::var_os("WEBKIT_DISABLE_DMABUF_RENDERER").is_none() {
        std::env::set_var("WEBKIT_DISABLE_DMABUF_RENDERER", "1");
    }

    tauri::Builder::default()
        .plugin(tauri_plugin_process::init())
        .plugin(tauri_plugin_updater::Builder::new().build())
        .manage(AppState(Mutex::new(Running::default())))
        .invoke_handler(tauri::generate_handler![
            start,
            stop,
            is_running,
            open_shortcut_settings,
            access_status,
            grant_access,
            pick_point,
            point_picked,
            cancel_pick
        ])
        .setup(|app| {
            // Register the system-wide hotkey via the portal (Wayland-correct).
            let handle = app.handle().clone();
            tauri::async_runtime::spawn(async move {
                if let Err(e) = global_shortcuts::run(handle).await {
                    eprintln!("global shortcut portal unavailable: {e}");
                }
            });
            Ok(())
        })
        .run(tauri::generate_context!())
        .expect("error while running wayclick");
}
