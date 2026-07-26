//! Reading the true pointer position from the compositor.
//!
//! Wayland deliberately hides the global pointer position from clients, so the
//! closed-loop positioner needs an out-of-band way to read it. This is
//! inherently compositor-specific, hence the [`CursorReader`] trait.
//!
//! [`KwinCursorReader`] is the KWin/Plasma backend. KWin exposes the pointer via
//! its scripting API (`workspace.cursorPos`); we load a one-line script that
//! calls back over D-Bus with the value. The reader owns a native D-Bus
//! connection (zbus) and receives that callback directly — no subprocesses, no
//! fixed sleeps — so a read normally costs a few milliseconds instead of ~500ms.

use std::{
    sync::{
        Mutex,
        atomic::{AtomicU64, Ordering},
        mpsc,
    },
    time::{Duration, Instant},
};

use zbus::{Connection, Proxy};

use crate::error::{InputError, Result};

/// Reads the global pointer position in desktop pixels.
pub trait CursorReader {
    fn position(&self) -> Result<(i32, i32)>;
}

/// How long to wait for the KWin script to call back. Script load + run is
/// normally a few milliseconds, but a busy compositor (heavy input, fullscreen
/// rendering) can delay script execution by hundreds of ms.
const CALLBACK_TIMEOUT: Duration = Duration::from_millis(900);
/// Full load→start→wait cycles to attempt before giving up.
const READ_ATTEMPTS: u32 = 2;

/// Distinguishes readers within a process so concurrent readers (engine +
/// point picker) never collide on bus names, script files, or plugin names.
static READER_SEQ: AtomicU64 = AtomicU64::new(0);

/// The D-Bus object the KWin script calls back into.
struct Spy {
    tx: mpsc::Sender<(i32, i32, i32)>,
}

#[zbus::interface(name = "org.wayclick.spy")]
impl Spy {
    /// KWin marshals `Math.round(...)` JS numbers as int32, matching this
    /// signature. `seq` identifies which read the callback belongs to — a
    /// busy compositor can run a script late, and its callback must not be
    /// mistaken for the answer to a newer read.
    fn cursor(&self, seq: i32, x: i32, y: i32) {
        let _ = self.tx.send((seq, x, y));
    }
}

/// KWin/Plasma 6 cursor reader (Wayland).
///
/// Uses zbus's *async* API driven by an owned one-worker runtime. Not the
/// blocking API: in tokio flavor it parks zbus's socket/dispatch tasks on a
/// private runtime that only advances while a blocking call is in flight, so
/// incoming callbacks freeze the moment you wait for them outside a call.
/// The owned runtime's worker thread dispatches continuously instead.
pub struct KwinCursorReader {
    rt: tokio::runtime::Runtime,
    conn: Connection,
    /// The unique bus name the KWin script calls back to.
    service: String,
    script_path: std::path::PathBuf,
    /// Receives `(seq, x, y)` callbacks; Mutex so `position(&self)` is shareable.
    rx: Mutex<mpsc::Receiver<(i32, i32, i32)>>,
    /// Unique per-reader tag used in the bus name, script file, and plugin name.
    tag: String,
    /// Per-read counter: tags callbacks and freshens KWin plugin names.
    read_seq: AtomicU64,
}

impl KwinCursorReader {
    pub fn new() -> Result<Self> {
        let tag = format!("p{}r{}", std::process::id(), READER_SEQ.fetch_add(1, Ordering::Relaxed));

        let rt = tokio::runtime::Builder::new_multi_thread()
            .worker_threads(1)
            .enable_all()
            .thread_name("wayclick-dbus")
            .build()
            .map_err(|e| InputError::CursorRead(format!("tokio runtime: {e}")))?;

        let (tx, rx) = mpsc::channel();
        let service = format!("org.wayclick.spy.{tag}");
        let conn = rt.block_on(async {
            let conn = Connection::session()
                .await
                .map_err(|e| InputError::CursorRead(format!("session bus: {e}")))?;
            conn.object_server()
                .at("/c", Spy { tx })
                .await
                .map_err(|e| InputError::CursorRead(format!("exporting callback object: {e}")))?;
            conn.request_name(service.as_str())
                .await
                .map_err(|e| InputError::CursorRead(format!("requesting bus name {service}: {e}")))?;
            Ok::<_, InputError>(conn)
        })?;

        let script_path = std::env::temp_dir().join(format!("wayclick-curpos-{tag}.js"));

        Ok(Self {
            rt,
            conn,
            service,
            script_path,
            rx: Mutex::new(rx),
            tag,
            read_seq: AtomicU64::new(0),
        })
    }

    fn scripting(&self) -> Result<Proxy<'_>> {
        self.rt
            .block_on(Proxy::new(&self.conn, "org.kde.KWin", "/Scripting", "org.kde.kwin.Scripting"))
            .map_err(|e| InputError::CursorRead(format!("kwin scripting proxy: {e}")))
    }

    fn call(
        &self,
        proxy: &Proxy<'_>,
        method: &str,
        args: &(impl zbus::export::serde::ser::Serialize + zbus::zvariant::DynamicType),
    ) -> zbus::Result<()> {
        self.rt.block_on(proxy.call_method(method, args)).map(|_| ())
    }

    /// One load→start→wait cycle. Returns None on callback timeout.
    fn read_once(
        &self,
        scripting: &Proxy<'_>,
        rx: &mpsc::Receiver<(i32, i32, i32)>,
        seq: i32,
    ) -> Result<Option<(i32, i32)>> {
        let script = format!(
            "var p = workspace.cursorPos;\n\
             callDBus(\"{}\", \"/c\", \"org.wayclick.spy\", \"Cursor\", {seq}, \
             Math.round(p.x), Math.round(p.y));\n",
            self.service
        );
        std::fs::write(&self.script_path, script)
            .map_err(|e| InputError::CursorRead(format!("writing kwin script: {e}")))?;

        let plugin = format!("wayclick_curpos_{}_{seq}", self.tag);
        let path = self.script_path.to_string_lossy().to_string();

        self.call(scripting, "loadScript", &(path, plugin.as_str()))
            .map_err(|e| InputError::CursorRead(format!("loadScript: {e}")))?;
        let started = self.call(scripting, "start", &());

        let deadline = Instant::now() + CALLBACK_TIMEOUT;
        let answer = loop {
            let left = deadline.saturating_duration_since(Instant::now());
            match rx.recv_timeout(left) {
                // A stale callback from an earlier, slow read: keep waiting.
                Ok((s, ..)) if s != seq => continue,
                Ok((_, x, y)) => break Some((x, y)),
                Err(_) => break None,
            }
        };

        let _ = self.call(scripting, "unloadScript", &(plugin.as_str(),));
        started.map_err(|e| InputError::CursorRead(format!("scripting start: {e}")))?;
        Ok(answer)
    }
}

impl CursorReader for KwinCursorReader {
    fn position(&self) -> Result<(i32, i32)> {
        let rx = self.rx.lock().unwrap();
        let scripting = self.scripting()?;

        for _ in 0..READ_ATTEMPTS {
            let seq = (self.read_seq.fetch_add(1, Ordering::Relaxed) % i32::MAX as u64) as i32;
            if let Some(pos) = self.read_once(&scripting, &rx, seq)? {
                return Ok(pos);
            }
        }
        Err(InputError::CursorRead(format!(
            "KWin script did not report a cursor position in {READ_ATTEMPTS} attempts \
             (is this KWin/Plasma?)"
        )))
    }
}

impl Drop for KwinCursorReader {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.script_path);
    }
}
