//! Wayland-native global hotkey via the XDG GlobalShortcuts portal — the same
//! mechanism OBS uses. The portal owns the binding; the user assigns the key in
//! their desktop's shortcut settings. No X11, no key grabbing.
//!
//! The one hard requirement is a stable **app id**, which the portal derives
//! from `GIO_LAUNCHED_DESKTOP_FILE` in `/proc/PID/environ` (the *initial* env).
//! A bare AppImage / dev run has none, so the portal rejects it ("an app id is
//! required"). [`reexec_with_gio_identity_if_needed`] fixes that without a
//! system install: write a user `.desktop` file and re-exec with the env var
//! set, so it lands in the process's initial environment.

use std::sync::Mutex;

use ashpd::WindowIdentifier;
use ashpd::desktop::global_shortcuts::{GlobalShortcuts, NewShortcut};
use futures_util::StreamExt;
use tauri::{AppHandle, Emitter};

const APP_ID: &str = "com.darkharasho.wayclick";

/// The trigger the portal reported (e.g. "F6"), or None if unbound. The UI reads
/// this via `hotkey_status` so it works even if it mounts after the bind.
pub static HOTKEY: Mutex<Option<String>> = Mutex::new(None);

/// Give the portal a stable app id, the way each backend wants it:
/// - **KDE** (`xdg-desktop-portal-kde`) reads it from the process's systemd
///   scope, so we re-launch inside an `app-<id>.scope`.
/// - **GNOME** reads `GIO_LAUNCHED_DESKTOP_FILE` from `/proc/PID/environ`, so we
///   set that too.
/// Both need an installed `.desktop` whose basename is the app id, so we drop a
/// user one (no system install required). Call from `main()` before any D-Bus
/// init. After the re-launch this returns immediately.
pub fn establish_identity() {
    if std::env::var_os("WAYCLICK_SCOPED").is_some() {
        return; // already relaunched into our identity
    }

    let desktop_path = ensure_desktop_file();
    let Ok(exe) = std::env::current_exe() else { return };
    let args: Vec<std::ffi::OsString> = std::env::args_os().skip(1).collect();
    let unit = format!("app-{APP_ID}-{}.scope", std::process::id());

    // Preferred path: a transient systemd app scope (KDE reads the id from here).
    let scoped = std::process::Command::new("systemd-run")
        .args(["--user", "--scope", "--quiet", "--collect", "--unit", &unit])
        .arg(&exe)
        .args(&args)
        .env("WAYCLICK_SCOPED", "1")
        .env("GIO_LAUNCHED_DESKTOP_FILE", &desktop_path)
        .env("WEBKIT_DISABLE_DMABUF_RENDERER", "1")
        .status();
    if let Ok(status) = scoped {
        std::process::exit(status.code().unwrap_or(0));
    }

    // systemd-run unavailable (e.g. GNOME without it): re-exec with just GIO set.
    use std::os::unix::process::CommandExt;
    let err = std::process::Command::new(&exe)
        .args(&args)
        .env("WAYCLICK_SCOPED", "1")
        .env("GIO_LAUNCHED_DESKTOP_FILE", &desktop_path)
        .env("WEBKIT_DISABLE_DMABUF_RENDERER", "1")
        .exec();
    eprintln!("[wayclick] identity re-exec failed: {err}");
    std::process::exit(1);
}

/// Write a user `.desktop` whose basename is the app id (unless a system one
/// already exists) and return the path the portal should be pointed at.
fn ensure_desktop_file() -> std::path::PathBuf {
    let system = std::path::PathBuf::from(format!("/usr/share/applications/{APP_ID}.desktop"));
    if system.exists() {
        return system;
    }

    let exec = std::env::var("APPIMAGE")
        .map(std::path::PathBuf::from)
        .unwrap_or_else(|_| std::env::current_exe().unwrap_or_default());
    let apps_dir = std::path::PathBuf::from(std::env::var_os("HOME").unwrap_or_default())
        .join(".local/share/applications");
    let user = apps_dir.join(format!("{APP_ID}.desktop"));

    let needs_write = !user.exists()
        || (std::env::var("APPIMAGE").is_ok()
            && !std::fs::read_to_string(&user)
                .unwrap_or_default()
                .contains(exec.to_string_lossy().as_ref()));
    if needs_write {
        let content = format!(
            "[Desktop Entry]\nName=wayclick\n\
             Comment=A sleek, minimal autoclicker for Wayland\n\
             Exec={exec}\nIcon={APP_ID}\nType=Application\n\
             Categories=Utility;\nStartupNotify=false\n",
            exec = exec.display()
        );
        if std::fs::create_dir_all(&apps_dir).is_ok() {
            let _ = std::fs::write(&user, &content);
        }
    }
    user
}

/// Run the portal session for the lifetime of the app: bind a "toggle" shortcut
/// (no preferred trigger — the user assigns the key in System Settings, like
/// OBS) and emit `hotkey:toggle` on activation.
pub async fn run(app: AppHandle) {
    let proxy = match GlobalShortcuts::new().await {
        Ok(p) => p,
        Err(e) => {
            eprintln!("[wayclick] global shortcuts portal unavailable: {e}");
            return;
        }
    };

    // Subscribe before binding so we can't miss an activation.
    let mut stream = match proxy.receive_activated().await {
        Ok(s) => s,
        Err(e) => {
            eprintln!("[wayclick] failed to subscribe to Activated: {e}");
            return;
        }
    };

    let session = match proxy.create_session().await {
        Ok(s) => s,
        Err(e) => {
            eprintln!("[wayclick] failed to create portal session: {e}");
            return;
        }
    };

    // No preferred_trigger: avoids the confusing first-launch confirmation dialog
    // and lets the user bind the key in System Settings → Shortcuts.
    let shortcuts = [NewShortcut::new("toggle", "Toggle wayclick on/off")];
    let bound = match proxy
        .bind_shortcuts(&session, &shortcuts, &WindowIdentifier::default())
        .await
    {
        Ok(req) => match req.response() {
            Ok(resp) => resp
                .shortcuts()
                .iter()
                .map(|s| s.trigger_description().to_string())
                .next(),
            Err(e) => {
                eprintln!("[wayclick] bind_shortcuts response error: {e}");
                return;
            }
        },
        Err(e) => {
            eprintln!("[wayclick] bind_shortcuts failed: {e}");
            return;
        }
    };

    *HOTKEY.lock().unwrap() = bound.clone().filter(|t| !t.is_empty());
    match bound.as_deref() {
        Some(trigger) if !trigger.is_empty() => {
            eprintln!("[wayclick] hotkey bound: {trigger}");
            let _ = app.emit("hotkey:bound", trigger.to_string());
        }
        _ => {
            // Registered but no key assigned yet — the UI prompts the user.
            eprintln!("[wayclick] hotkey registered but unbound — assign it in System Settings");
            let _ = app.emit("hotkey:unbound", ());
        }
    }

    // `session` must stay alive for the listener — dropping it unbinds.
    while let Some(ev) = stream.next().await {
        if ev.shortcut_id() == "toggle" {
            let _ = app.emit("hotkey:toggle", ());
        }
    }
    drop(session);
}
