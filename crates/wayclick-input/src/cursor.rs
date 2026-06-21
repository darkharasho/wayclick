//! Reading the true pointer position from the compositor.
//!
//! Wayland deliberately hides the global pointer position from clients, so the
//! closed-loop positioner needs an out-of-band way to read it. This is
//! inherently compositor-specific, hence the [`CursorReader`] trait.
//!
//! [`KwinCursorReader`] is the KWin/Plasma backend. KWin exposes the pointer via
//! its scripting API (`workspace.cursorPos`); we run a one-line script that
//! emits the value over D-Bus and capture it. This v1 shells out to
//! `qdbus`/`dbus-monitor`; a future revision should use a native D-Bus binding
//! (zbus) that owns a service to receive the callback directly.

use std::{
    io::Read,
    process::{Command, Stdio},
    sync::atomic::{AtomicU64, Ordering},
    thread::sleep,
    time::Duration,
};

use crate::error::{InputError, Result};

/// Reads the global pointer position in desktop pixels.
pub trait CursorReader {
    fn position(&self) -> Result<(i32, i32)>;
}

/// KWin/Plasma 6 cursor reader (Wayland).
pub struct KwinCursorReader {
    script_path: std::path::PathBuf,
}

const KWIN_SCRIPT: &str = r#"var p = workspace.cursorPos;
callDBus("org.wayclick.spy", "/c", "org.wayclick.spy", "Cursor", Math.round(p.x), Math.round(p.y));
"#;

static PLUGIN_SEQ: AtomicU64 = AtomicU64::new(0);

impl KwinCursorReader {
    pub fn new() -> Result<Self> {
        let path = std::env::temp_dir().join("wayclick-curpos.js");
        std::fs::write(&path, KWIN_SCRIPT)
            .map_err(|e| InputError::CursorRead(format!("writing kwin script: {e}")))?;
        Ok(Self { script_path: path })
    }

    fn run_qdbus(args: &[&str]) -> Result<()> {
        Command::new("qdbus")
            .args(args)
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status()
            .map_err(|e| InputError::CursorRead(format!("qdbus: {e}")))?;
        Ok(())
    }
}

impl CursorReader for KwinCursorReader {
    fn position(&self) -> Result<(i32, i32)> {
        // Eavesdrop the callback the KWin script will emit.
        let mut monitor = Command::new("dbus-monitor")
            .arg("interface='org.wayclick.spy'")
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()
            .map_err(|e| InputError::CursorRead(format!("dbus-monitor: {e}")))?;

        sleep(Duration::from_millis(150));

        let seq = PLUGIN_SEQ.fetch_add(1, Ordering::Relaxed);
        let plugin = format!("wayclick_curpos_{}_{}", std::process::id(), seq);
        let path = self.script_path.to_string_lossy().to_string();
        Self::run_qdbus(&[
            "org.kde.KWin",
            "/Scripting",
            "org.kde.kwin.Scripting.loadScript",
            &path,
            &plugin,
        ])?;
        Self::run_qdbus(&["org.kde.KWin", "/Scripting", "org.kde.kwin.Scripting.start"])?;
        sleep(Duration::from_millis(350));
        let _ = Self::run_qdbus(&[
            "org.kde.KWin",
            "/Scripting",
            "org.kde.kwin.Scripting.unloadScript",
            &plugin,
        ]);

        let _ = monitor.kill();
        let mut out = String::new();
        if let Some(mut stdout) = monitor.stdout.take() {
            let _ = stdout.read_to_string(&mut out);
        }
        let _ = monitor.wait();

        parse_cursor(&out)
            .ok_or_else(|| InputError::CursorRead("no Cursor message captured".into()))
    }
}

/// Parse the two `int32` arguments following a `member=Cursor` line in
/// dbus-monitor output.
fn parse_cursor(s: &str) -> Option<(i32, i32)> {
    let idx = s.rfind("member=Cursor")?;
    let ints: Vec<i32> = s[idx..]
        .lines()
        .filter_map(|l| l.trim().strip_prefix("int32 "))
        .filter_map(|n| n.trim().parse().ok())
        .collect();
    match ints.as_slice() {
        [x, y, ..] => Some((*x, *y)),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::parse_cursor;

    #[test]
    fn parses_last_cursor_message() {
        let sample = "\
signal time=1.0 sender=:1.1 -> destination=:1.2 ...
method call time=2.0 sender=:1.603 -> destination=org.wayclick.spy serial=1 path=/c; interface=org.wayclick.spy; member=Cursor
   int32 1410
   int32 710
";
        assert_eq!(parse_cursor(sample), Some((1410, 710)));
    }

    #[test]
    fn takes_the_most_recent_message() {
        let sample = "\
member=Cursor
   int32 100
   int32 200
member=Cursor
   int32 900
   int32 800
";
        assert_eq!(parse_cursor(sample), Some((900, 800)));
    }

    #[test]
    fn none_when_absent() {
        assert_eq!(parse_cursor("nothing here"), None);
    }
}
