// Prevent an extra console window on Windows in release.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

use std::{
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
    },
    thread::JoinHandle,
    time::{Duration, SystemTime},
};

use serde::{Deserialize, Serialize};
use tauri::{AppHandle, Emitter, Manager, State, WebviewUrl, WebviewWindowBuilder};


use wayclick_input::{
    ClickConfig, ClickEngine, ClickKind, ClosedLoopPositioner, CursorReader, HoldController,
    HoldTarget, Keycode, KwinCursorReader, MouseButton, Repeat, StopFlag, Target, VirtualKeyboard,
    VirtualMouse,
};

mod portal_hotkey;

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

/// The long-lived virtual devices. KWin silently drops events from a freshly
/// created uinput device until it has been enumerated (~1.2s) and its button
/// grab warmed up, so devices are created once — ideally by the background
/// pre-warm at launch — and kept for the app's lifetime instead of per run.
/// Both are created together because creating a uinput device mid-run disrupts
/// KWin's grab on a button the other device is holding.
struct Devices {
    mouse: VirtualMouse,
    keyboard: VirtualKeyboard,
}

/// Settle after device creation: events sent before the compositor has
/// enumerated the device are silently dropped (still seen ~1.5s post-create on
/// KWin 6.7). Paid once per session, normally by the launch pre-warm.
const DEVICE_SETTLE: Duration = Duration::from_millis(2000);

#[derive(Default)]
struct Inner {
    running: Mutex<Running>,
    devices: Mutex<Option<Arc<Devices>>>,
    /// True once the mouse has delivered real click cycles this session. A
    /// cold device needs priming clicks before KWin honors a press-and-hold.
    warmed: AtomicBool,
}

struct AppState(Arc<Inner>);

fn get_or_create_devices(inner: &Inner) -> wayclick_input::Result<Arc<Devices>> {
    let mut guard = inner.devices.lock().unwrap();
    if let Some(d) = guard.as_ref() {
        return Ok(d.clone());
    }
    let mouse = VirtualMouse::create_no_wait()?;
    let keyboard = VirtualKeyboard::create_no_wait()?;
    std::thread::sleep(DEVICE_SETTLE);
    let devices = Arc::new(Devices { mouse, keyboard });
    *guard = Some(devices.clone());
    Ok(devices)
}

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

/// The worker that borrows the long-lived devices and runs until `stop` is set.
fn run_worker(cfg: RunConfig, stop: StopFlag, app: AppHandle, inner: Arc<Inner>) {
    eprintln!(
        "[wayclick] worker start: action={} position={:?} interval={}ms",
        cfg.action, cfg.position, cfg.interval_ms
    );
    let result = (|| -> wayclick_input::Result<()> {
        let devices = get_or_create_devices(&inner)?;
        let mouse = &devices.mouse;
        emit_status(&app, "running");

        if cfg.action == "hold" {
            match cfg.hold_key.as_deref().and_then(Keycode::from_name) {
                // Mouse-button hold.
                None => {
                    let b = button_from(&cfg.button);
                    // KWin ignores a press-and-hold from a mouse that has never
                    // clicked — the button grab needs warming by real click
                    // cycles (measured: one is not enough, 3+ works). With
                    // long-lived devices any earlier run counts, so this fires
                    // at most once per session, not before every hold.
                    if !inner.warmed.load(Ordering::Relaxed) {
                        for _ in 0..5 {
                            mouse.click(b, Duration::from_millis(40))?;
                            std::thread::sleep(Duration::from_millis(60));
                        }
                        inner.warmed.store(true, Ordering::Relaxed);
                    }
                    mouse.press(b)?;
                    while !stop.is_stopped() {
                        std::thread::sleep(Duration::from_millis(40));
                    }
                    mouse.release(b)?;
                }
                // Key hold: press and hold the key on the long-lived keyboard.
                Some(k) => {
                    let ctrl = HoldController::new(mouse, &devices.keyboard);
                    ctrl.hold(HoldTarget::Key(k))?;
                    while !stop.is_stopped() {
                        std::thread::sleep(Duration::from_millis(40));
                    }
                    ctrl.release(HoldTarget::Key(k))?;
                }
            }
            return Ok(());
        }

        // Click action.
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
            // ≥30ms or KWin drops clicks while the pointer is moving (follow-cursor).
            hold: Duration::from_millis(40),
            double_gap: Duration::from_millis(40),
            reposition_each_click: cfg.reposition_each_click,
        };
        // The KWin cursor reader is only needed to reach a fixed target;
        // follow-cursor runs on any compositor.
        let done = match cfg.position {
            Some(_) => {
                let reader = KwinCursorReader::new()?;
                let positioner = ClosedLoopPositioner::new(mouse, &reader);
                ClickEngine::new(mouse, Some(&positioner)).run(&click_cfg, &stop, seed())?
            }
            None => ClickEngine::<ClosedLoopPositioner<KwinCursorReader>>::new(mouse, None)
                .run(&click_cfg, &stop, seed())?,
        };
        if done > 0 {
            inner.warmed.store(true, Ordering::Relaxed);
        }
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
    let inner = &state.0;
    let mut running = inner.running.lock().unwrap();
    // A worker that ended on its own (finite count, engine error) leaves its
    // handle behind; reap it so Start works again instead of silently no-oping.
    if let Some(h) = running.handle.take() {
        if h.is_finished() {
            let _ = h.join();
            running.stop = None;
        } else {
            eprintln!("[wayclick] start ignored — already running");
            running.handle = Some(h);
            return Ok(());
        }
    }
    let stop = StopFlag::new();
    emit_status(&app, "arming");
    let worker_stop = stop.clone();
    let worker_app = app.clone();
    let worker_inner = inner.clone();
    let handle =
        std::thread::spawn(move || run_worker(config, worker_stop, worker_app, worker_inner));
    running.stop = Some(stop);
    running.handle = Some(handle);
    Ok(())
}

#[tauri::command]
fn stop(state: State<AppState>) -> Result<(), String> {
    let (stop, handle) = {
        let mut running = state.0.running.lock().unwrap();
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
    state
        .0
        .running
        .lock()
        .unwrap()
        .handle
        .as_ref()
        .is_some_and(|h| !h.is_finished())
}

/// The hotkey trigger the portal bound (e.g. "F6"), or null if unbound. Queried
/// by the UI on load so it shows the binding even if it mounted after the bind.
#[tauri::command]
fn hotkey_status() -> Option<String> {
    portal_hotkey::HOTKEY.lock().unwrap().clone()
}

/// Re-assert the exec bit on the AppImage we run from. The updater plugin
/// preserves permissions in every reproduction we ran, yet one real
/// 0.1.5→0.1.6 update ended with a non-executable file and a relaunch dying
/// on EACCES (cause never reproduced). Asserting the bit costs nothing and
/// makes the failure mode impossible. Called at startup and by the UI right
/// after an update installs, before relaunch.
#[tauri::command]
fn ensure_self_executable() {
    use std::os::unix::fs::PermissionsExt;
    let Some(path) = std::env::var_os("APPIMAGE") else { return };
    let path = std::path::PathBuf::from(path);
    let Ok(meta) = std::fs::metadata(&path) else { return };
    let mut perms = meta.permissions();
    if perms.mode() & 0o111 != 0o111 {
        perms.set_mode(0o755);
        match std::fs::set_permissions(&path, perms) {
            Ok(()) => eprintln!("[wayclick] restored missing exec bit on {}", path.display()),
            Err(e) => eprintln!("[wayclick] could not restore exec bit on {}: {e}", path.display()),
        }
    }
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
    // Native Wayland: WebKit's DMABUF renderer fails to allocate GBM buffers on
    // some GPU setups, so disable it (set before GTK init).
    std::env::set_var("WEBKIT_DISABLE_DMABUF_RENDERER", "1");

    // Give the GlobalShortcuts portal a stable app id (systemd scope for KDE,
    // GIO_LAUNCHED_DESKTOP_FILE for GNOME, + a user .desktop). Re-launches the
    // process, so it must run before any D-Bus init.
    portal_hotkey::establish_identity();

    tauri::Builder::default()
        .plugin(tauri_plugin_process::init())
        .plugin(tauri_plugin_updater::Builder::new().build())
        .manage(AppState(Arc::new(Inner::default())))
        .invoke_handler(tauri::generate_handler![
            start,
            stop,
            is_running,
            hotkey_status,
            ensure_self_executable,
            open_shortcut_settings,
            access_status,
            grant_access,
            pick_point,
            point_picked,
            cancel_pick
        ])
        .setup(|app| {
            // Self-heal: if we somehow run from a non-executable AppImage,
            // future launches from the desktop entry would fail.
            ensure_self_executable();
            // Pre-warm: create the virtual devices in the background so the
            // first toggle doesn't pay the enumeration settle. Silently skipped
            // when /dev/uinput isn't accessible yet (first-run gate).
            let inner = app.state::<AppState>().0.clone();
            std::thread::spawn(move || {
                if let Err(e) = get_or_create_devices(&inner) {
                    eprintln!("[wayclick] device pre-warm skipped: {e}");
                }
            });
            let handle = app.handle().clone();
            tauri::async_runtime::spawn(async move {
                portal_hotkey::run(handle).await;
            });
            Ok(())
        })
        .run(tauri::generate_context!())
        .expect("error while running wayclick");
}
