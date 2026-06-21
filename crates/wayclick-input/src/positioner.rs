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
    /// Pause after each move so the compositor applies it before the next read.
    pub settle: Duration,
}

impl Default for LoopConfig {
    fn default() -> Self {
        Self {
            tolerance: 1,
            max_step: 150,
            max_iters: 40,
            settle: Duration::from_millis(8),
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

impl<R: CursorReader> PointerPositioner for ClosedLoopPositioner<'_, R> {
    fn move_to(&self, tx: i32, ty: i32) -> Result<()> {
        let mut last = (i32::MIN, i32::MIN);
        for _ in 0..self.cfg.max_iters {
            let (cx, cy) = self.reader.position()?;
            let (dx, dy) = (tx - cx, ty - cy);
            if dx.abs() <= self.cfg.tolerance && dy.abs() <= self.cfg.tolerance {
                return Ok(());
            }
            self.mouse
                .move_relative(clamp(dx, self.cfg.max_step), clamp(dy, self.cfg.max_step))?;
            last = (cx, cy);
            sleep(self.cfg.settle);
        }
        let (x, y) = self.reader.position().unwrap_or(last);
        Err(InputError::NotConverged { tx, ty, x, y, steps: self.cfg.max_iters })
    }
}
