//! Absolute pointer positioning.
//!
//! There is no uinput-level absolute pointer on KWin (libinput drops abs axes
//! for virtual mice and won't expose touch/tablet as the system pointer). The
//! working method is a closed loop: read the true cursor position, emit a
//! relative delta toward the target, repeat until converged. [`PointerPositioner`]
//! abstracts this so a future compositor with a real absolute virtual-pointer
//! protocol (e.g. wlroots `zwlr_virtual_pointer_v1`) can provide a one-shot
//! backend behind the same interface.

use std::{thread::sleep, time::Duration};

use crate::{
    cursor::CursorReader,
    error::{InputError, Result},
    mouse::VirtualMouse,
};

/// Moves the pointer to an absolute desktop pixel.
pub trait PointerPositioner {
    fn move_to(&self, x: i32, y: i32) -> Result<()>;
}

/// Tuning for the closed-loop positioner.
#[derive(Debug, Clone, Copy)]
pub struct LoopConfig {
    /// Convergence tolerance in pixels (per axis).
    pub tolerance: i32,
    /// Max relative step per iteration. Small steps keep pointer acceleration
    /// near-linear so the loop converges instead of overshooting.
    pub max_step: i32,
    /// Max iterations before giving up (a pointer-grabbing fullscreen app can
    /// make convergence impossible).
    pub max_iters: u32,
    /// Poll interval while waiting for the compositor to apply a move.
    pub settle: Duration,
    /// How many settle-polls to wait for a move to become visible before
    /// concluding it was dropped and letting the outer loop retry.
    pub settle_polls: u32,
}

impl Default for LoopConfig {
    fn default() -> Self {
        Self {
            tolerance: 1,
            max_step: 150,
            max_iters: 40,
            settle: Duration::from_millis(8),
            settle_polls: 6,
        }
    }
}

/// Closed-loop positioner: `reader` provides feedback, `mouse` provides motion.
pub struct ClosedLoopPositioner<'a, R: CursorReader> {
    mouse: &'a VirtualMouse,
    reader: &'a R,
    cfg: LoopConfig,
}

impl<'a, R: CursorReader> ClosedLoopPositioner<'a, R> {
    pub fn new(mouse: &'a VirtualMouse, reader: &'a R) -> Self {
        Self { mouse, reader, cfg: LoopConfig::default() }
    }

    pub fn with_config(mouse: &'a VirtualMouse, reader: &'a R, cfg: LoopConfig) -> Self {
        Self { mouse, reader, cfg }
    }
}

fn clamp(v: i32, max: i32) -> i32 {
    v.clamp(-max, max)
}

/// Divide a wanted on-screen delta by the current gain estimate, keeping at
/// least 1 unit of motion in the right direction.
fn descale(v: i32, gain: f64) -> i32 {
    if v == 0 {
        return 0;
    }
    let scaled = (f64::from(v) / gain).round() as i32;
    if scaled == 0 { v.signum() } else { scaled }
}

impl<R: CursorReader> ClosedLoopPositioner<'_, R> {
    /// Wait until the compositor shows the pointer somewhere other than
    /// `(px, py)`, or give up after `settle_polls` polls (the move may have
    /// been legitimately absorbed at a screen edge or by a grab). Returns the
    /// freshest position either way.
    fn wait_for_motion(&self, px: i32, py: i32) -> Result<(i32, i32)> {
        let mut pos = (px, py);
        for _ in 0..self.cfg.settle_polls.max(1) {
            sleep(self.cfg.settle);
            pos = self.reader.position()?;
            if pos != (px, py) {
                break;
            }
        }
        Ok(pos)
    }
}

impl<R: CursorReader> PointerPositioner for ClosedLoopPositioner<'_, R> {
    fn move_to(&self, tx: i32, ty: i32) -> Result<()> {
        // Closed loop: read, nudge, wait until the nudge is *observed*, repeat.
        // Waiting for observed motion (rather than a fixed pause) matters: the
        // cursor read is fast enough now that a fixed pause can race the
        // compositor — re-sending a delta it hasn't applied yet overshoots.
        //
        // libinput applies velocity-dependent pointer acceleration to virtual
        // mice, and at this loop's pace that lands near a constant ~2× (traced
        // on KWin 6.7): sending the raw remaining delta overshoots by the same
        // amount every time and the loop ping-pongs around the target forever.
        // So track the observed applied/sent gain and divide each step by it.
        let trace = std::env::var_os("WAYCLICK_POS_TRACE").is_some();
        let mut gain = 1.0f64;
        let (mut cx, mut cy) = self.reader.position()?;
        for i in 0..self.cfg.max_iters {
            let (dx, dy) = (tx - cx, ty - cy);
            if dx.abs() <= self.cfg.tolerance && dy.abs() <= self.cfg.tolerance {
                return Ok(());
            }
            let (px, py) = (cx, cy);
            let (sx, sy) = (
                clamp(descale(dx, gain), self.cfg.max_step),
                clamp(descale(dy, gain), self.cfg.max_step),
            );
            self.mouse.move_relative(sx, sy)?;
            (cx, cy) = self.wait_for_motion(px, py)?;

            let (mx, my) = (cx - px, cy - py);
            if (mx, my) != (0, 0) {
                // Update the gain estimate from what actually happened. Ratio
                // of magnitudes; EMA smooths accel-curve noise. Skipped when
                // the event was swallowed (fresh device) — that says nothing
                // about scaling.
                let sent = f64::from(sx * sx + sy * sy).sqrt();
                let moved = f64::from(mx * mx + my * my).sqrt();
                if sent > 0.0 {
                    gain = (0.5 * gain + 0.5 * (moved / sent)).clamp(0.2, 5.0);
                }
            }
            if trace {
                eprintln!(
                    "[pos {i:02}] at ({px},{py}) sent ({sx},{sy}) now ({cx},{cy}) \
                     gain {gain:.2} target ({tx},{ty})"
                );
            }
        }
        Err(InputError::NotConverged { tx, ty, x: cx, y: cy, steps: self.cfg.max_iters })
    }
}
