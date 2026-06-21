//! System-wide hotkey via the XDG `GlobalShortcuts` portal.
//!
//! Tauri's global-shortcut plugin grabs keys through X11, which doesn't work
//! for Wayland-native windows. The portal is the Wayland-correct path: the
//! compositor (KDE) owns the binding, we register a "toggle" shortcut with a
//! preferred trigger of F6, and emit `hotkey:toggle` whenever it fires —
//! focused or not. Users rebind it in their system shortcut settings.

use ashpd::WindowIdentifier;
use ashpd::desktop::global_shortcuts::{GlobalShortcuts, NewShortcut};
use futures_util::StreamExt;
use tauri::{AppHandle, Emitter};

pub async fn run(app: AppHandle) -> ashpd::Result<()> {
    let shortcuts = GlobalShortcuts::new().await?;
    let session = shortcuts.create_session().await?;

    let wanted = [NewShortcut::new("toggle", "Toggle wayclick on/off")
        .preferred_trigger(Some("F6"))];
    let request = shortcuts
        .bind_shortcuts(&session, &wanted, &WindowIdentifier::default())
        .await?;
    // The bind response carries the actual triggers KDE assigned; we don't need
    // them, but resolving it confirms the bind went through.
    let _ = request.response();

    let mut activated = shortcuts.receive_activated().await?;
    while let Some(action) = activated.next().await {
        if action.shortcut_id() == "toggle" {
            let _ = app.emit("hotkey:toggle", ());
        }
    }
    // Keep the session alive for the whole loop.
    drop(session);
    Ok(())
}
