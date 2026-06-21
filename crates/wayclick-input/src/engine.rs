//! The autoclicker loop, built on [`VirtualMouse`] and [`PointerPositioner`].
//!
//! Feature parity target (xclicker / OPAutoClicker): fixed & randomized
//! intervals, button choice, single/double click, finite-count vs infinite, and
//! fixed-position vs follow-cursor.
//!
//! Fixed-position note: positioning uses the closed loop, which is not instant,
//! so the engine positions to the fixed target **once** at the start (clicks do
//! not move the pointer, so it stays put). `reposition_each_click` re-asserts the
//! position before every click for correctness at the cost of click rate.

use std::{
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    thread::sleep,
    time::Duration,
};

use crate::{
    error::Result,
    mouse::{MouseButton, VirtualMouse},
    positioner::PointerPositioner,
};

/// How many clicks to perform.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Repeat {
    Infinite,
    Count(u64),
}

/// Single or double click per tick.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ClickKind {
    Single,
    Double,
}

/// Where each click lands.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Target {
    /// Click wherever the pointer currently is.
    FollowCursor,
    /// Click at a fixed desktop pixel.
    Fixed { x: i32, y: i32 },
}

/// Full autoclicker configuration.
#[derive(Debug, Clone, Copy)]
pub struct ClickConfig {
    pub button: MouseButton,
    pub kind: ClickKind,
    /// Base delay between ticks.
    pub interval: Duration,
    /// Uniform random jitter added to the interval in `[0, jitter]`. Zero = fixed.
    pub jitter: Duration,
    pub repeat: Repeat,
    pub target: Target,
    /// Button hold time within a single click.
    pub hold: Duration,
    /// Gap between the two presses of a double click.
    pub double_gap: Duration,
    /// Re-assert the fixed position before every click (slower, drift-proof).
    pub reposition_each_click: bool,
}

impl Default for ClickConfig {
    fn default() -> Self {
        Self {
            button: MouseButton::Left,
            kind: ClickKind::Single,
            interval: Duration::from_millis(100),
            jitter: Duration::ZERO,
            repeat: Repeat::Infinite,
            target: Target::FollowCursor,
            hold: Duration::from_millis(20),
            double_gap: Duration::from_millis(40),
            reposition_each_click: false,
        }
    }
}

/// A cancellable handle shared with whatever toggles the clicker on/off.
#[derive(Clone, Default)]
pub struct StopFlag(Arc<AtomicBool>);

impl StopFlag {
    pub fn new() -> Self {
        Self::default()
    }
    pub fn stop(&self) {
        self.0.store(true, Ordering::Relaxed);
    }
    pub fn is_stopped(&self) -> bool {
        self.0.load(Ordering::Relaxed)
    }
    pub fn reset(&self) {
        self.0.store(false, Ordering::Relaxed);
    }
}

/// Deterministic, dependency-free PRNG (xorshift64*) for interval jitter.
/// Jitter doesn't need cryptographic quality; this keeps the crate lean.
pub struct Rng(u64);

impl Rng {
    pub fn new(seed: u64) -> Self {
        Self(seed | 1)
    }
    fn next_u64(&mut self) -> u64 {
        let mut x = self.0;
        x ^= x >> 12;
        x ^= x << 25;
        x ^= x >> 27;
        self.0 = x;
        x.wrapping_mul(0x2545F4914F6CDD1D)
    }
    /// Uniform value in `[0, max]` (inclusive).
    fn in_range(&mut self, max: u64) -> u64 {
        if max == 0 { 0 } else { self.next_u64() % (max + 1) }
    }
}

/// Compute the delay before the next click: `interval + U[0, jitter]`.
pub fn next_delay(interval: Duration, jitter: Duration, rng: &mut Rng) -> Duration {
    if jitter.is_zero() {
        return interval;
    }
    interval + Duration::from_micros(rng.in_range(jitter.as_micros() as u64))
}

/// Whether the loop should continue given how many clicks have happened.
pub fn should_continue(repeat: Repeat, done: u64) -> bool {
    match repeat {
        Repeat::Infinite => true,
        Repeat::Count(n) => done < n,
    }
}

/// The autoclicker. Holds the device; positioning is optional (only needed for
/// `Target::Fixed`).
pub struct ClickEngine<'a, P: PointerPositioner> {
    mouse: &'a VirtualMouse,
    positioner: Option<&'a P>,
}

impl<'a, P: PointerPositioner> ClickEngine<'a, P> {
    pub fn new(mouse: &'a VirtualMouse, positioner: Option<&'a P>) -> Self {
        Self { mouse, positioner }
    }

    fn one_click(&self, cfg: &ClickConfig) -> Result<()> {
        self.mouse.click(cfg.button, cfg.hold)?;
        if cfg.kind == ClickKind::Double {
            sleep(cfg.double_gap);
            self.mouse.click(cfg.button, cfg.hold)?;
        }
        Ok(())
    }

    /// Run until the count is exhausted or `stop` is set. `seed` seeds the
    /// jitter RNG (pass a time-derived value from the caller).
    pub fn run(&self, cfg: &ClickConfig, stop: &StopFlag, seed: u64) -> Result<u64> {
        let mut rng = Rng::new(seed);

        // Fixed-position: assert the target once up front.
        if let Target::Fixed { x, y } = cfg.target {
            let p = self.positioner.expect("fixed target requires a positioner");
            p.move_to(x, y)?;
        }

        let mut done: u64 = 0;
        while should_continue(cfg.repeat, done) && !stop.is_stopped() {
            if cfg.reposition_each_click {
                if let Target::Fixed { x, y } = cfg.target {
                    self.positioner.unwrap().move_to(x, y)?;
                }
            }
            self.one_click(cfg)?;
            done += 1;

            if !should_continue(cfg.repeat, done) {
                break;
            }
            // Sleep in small slices so stop is responsive even on long intervals.
            let delay = next_delay(cfg.interval, cfg.jitter, &mut rng);
            sleep_interruptible(delay, stop);
        }
        Ok(done)
    }
}

fn sleep_interruptible(total: Duration, stop: &StopFlag) {
    const SLICE: Duration = Duration::from_millis(10);
    let mut left = total;
    while left > Duration::ZERO && !stop.is_stopped() {
        let s = left.min(SLICE);
        sleep(s);
        left = left.saturating_sub(s);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fixed_interval_ignores_jitter() {
        let mut rng = Rng::new(42);
        let d = next_delay(Duration::from_millis(100), Duration::ZERO, &mut rng);
        assert_eq!(d, Duration::from_millis(100));
    }

    #[test]
    fn jitter_stays_within_bounds() {
        let mut rng = Rng::new(12345);
        let base = Duration::from_millis(100);
        let jitter = Duration::from_millis(50);
        for _ in 0..10_000 {
            let d = next_delay(base, jitter, &mut rng);
            assert!(d >= base, "delay {d:?} below base");
            assert!(d <= base + jitter, "delay {d:?} above base+jitter");
        }
    }

    #[test]
    fn count_terminates() {
        assert!(should_continue(Repeat::Count(3), 0));
        assert!(should_continue(Repeat::Count(3), 2));
        assert!(!should_continue(Repeat::Count(3), 3));
        assert!(!should_continue(Repeat::Count(3), 4));
    }

    #[test]
    fn infinite_never_terminates() {
        assert!(should_continue(Repeat::Infinite, 0));
        assert!(should_continue(Repeat::Infinite, 1_000_000));
    }

    #[test]
    fn rng_is_deterministic() {
        let mut a = Rng::new(7);
        let mut b = Rng::new(7);
        for _ in 0..100 {
            assert_eq!(a.next_u64(), b.next_u64());
        }
    }
}
