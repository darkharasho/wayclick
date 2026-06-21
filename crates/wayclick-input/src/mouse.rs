//! A virtual mouse backed by `/dev/uinput`.
//!
//! This is the proven core of Wayclick: a relative pointer with left/right/middle
//! buttons. libinput accepts a relative virtual mouse unconditionally (all
//! buttons work and reach every app), which is why the engine positions via
//! relative motion rather than absolute axes — see the `positioner` module.

use std::{fs::File, thread::sleep, time::Duration};

use input_linux::{
    EventKind, EventTime, InputEvent, InputId, Key, KeyEvent, KeyState, RelativeAxis,
    RelativeEvent, SynchronizeEvent, UInputHandle,
    sys::{BUS_USB, input_event},
};

use crate::error::{InputError, Result};

/// Which mouse button a click uses.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MouseButton {
    Left,
    Right,
    Middle,
}

impl MouseButton {
    fn key(self) -> Key {
        match self {
            MouseButton::Left => Key::ButtonLeft,
            MouseButton::Right => Key::ButtonRight,
            MouseButton::Middle => Key::ButtonMiddle,
        }
    }
}

/// How long to give libinput/the compositor to enumerate the device after
/// creation before the first event is sent. Events emitted before the
/// compositor has the device are silently dropped.
const REGISTER_SETTLE: Duration = Duration::from_millis(1200);

/// A relative virtual mouse. Drops its uinput device on `Drop`.
pub struct VirtualMouse {
    handle: UInputHandle<File>,
}

impl VirtualMouse {
    /// Create the virtual mouse and wait for the compositor to enumerate it.
    pub fn create() -> Result<Self> {
        let mouse = Self::create_no_wait()?;
        sleep(REGISTER_SETTLE);
        Ok(mouse)
    }

    /// Create the device without the registration wait. Callers that create the
    /// device well before first use (or run their own settle) can use this; most
    /// callers want [`VirtualMouse::create`].
    pub fn create_no_wait() -> Result<Self> {
        let file = std::fs::OpenOptions::new()
            .write(true)
            .open("/dev/uinput")
            .map_err(InputError::OpenUinput)?;
        let handle = UInputHandle::new(file);

        handle.set_evbit(EventKind::Relative).map_err(InputError::Create)?;
        handle.set_evbit(EventKind::Key).map_err(InputError::Create)?;
        handle.set_evbit(EventKind::Synchronize).map_err(InputError::Create)?;
        handle.set_relbit(RelativeAxis::X).map_err(InputError::Create)?;
        handle.set_relbit(RelativeAxis::Y).map_err(InputError::Create)?;
        for b in [Key::ButtonLeft, Key::ButtonRight, Key::ButtonMiddle] {
            handle.set_keybit(b).map_err(InputError::Create)?;
        }

        handle
            .create(
                &InputId {
                    bustype: BUS_USB,
                    vendor: 0x3232,
                    product: 0x567a,
                    version: 1,
                },
                b"wayclick-virtual-mouse",
                0,
                &[],
            )
            .map_err(InputError::Create)?;

        Ok(Self { handle })
    }

    /// Emit a relative motion. The compositor applies pointer acceleration, so
    /// the on-screen delta is not 1:1 — absolute positioning is done by the
    /// `positioner` module via a read/move feedback loop.
    pub fn move_relative(&self, dx: i32, dy: i32) -> Result<()> {
        let mut evs: Vec<input_event> = Vec::with_capacity(3);
        if dx != 0 {
            evs.push(rel(RelativeAxis::X, dx));
        }
        if dy != 0 {
            evs.push(rel(RelativeAxis::Y, dy));
        }
        if evs.is_empty() {
            return Ok(());
        }
        evs.push(syn());
        self.write(&evs)
    }

    /// Press and hold a button (no release). Pair with [`VirtualMouse::release`].
    pub fn press(&self, button: MouseButton) -> Result<()> {
        self.write(&[key(button.key(), KeyState::PRESSED), syn()])
    }

    /// Release a previously pressed button.
    pub fn release(&self, button: MouseButton) -> Result<()> {
        self.write(&[key(button.key(), KeyState::RELEASED), syn()])
    }

    /// A full press+release click with a short, configurable hold.
    pub fn click(&self, button: MouseButton, hold: Duration) -> Result<()> {
        self.press(button)?;
        sleep(hold);
        self.release(button)
    }

    fn write(&self, evs: &[input_event]) -> Result<()> {
        self.handle.write(evs).map_err(InputError::Write)?;
        Ok(())
    }
}

impl Drop for VirtualMouse {
    fn drop(&mut self) {
        let _ = self.handle.dev_destroy();
    }
}

fn now() -> EventTime {
    // The kernel timestamps synthetic events; zero is fine and deterministic.
    EventTime::new(0, 0)
}

fn rel(axis: RelativeAxis, v: i32) -> input_event {
    InputEvent::from(RelativeEvent::new(now(), axis, v)).as_raw().to_owned()
}

fn key(k: Key, state: KeyState) -> input_event {
    InputEvent::from(KeyEvent::new(now(), k, state)).as_raw().to_owned()
}

fn syn() -> input_event {
    InputEvent::from(SynchronizeEvent::report(now())).as_raw().to_owned()
}
