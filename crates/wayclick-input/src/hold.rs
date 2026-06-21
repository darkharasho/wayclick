//! The auto-hold action: press a key or mouse button and keep it held until
//! released. Distinct from clicking — the target stays down.

use crate::{
    error::Result,
    keyboard::{Keycode, VirtualKeyboard},
    mouse::{MouseButton, VirtualMouse},
};

/// What the hold action holds down.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HoldTarget {
    Mouse(MouseButton),
    Key(Keycode),
}

/// Holds a [`HoldTarget`] down via the appropriate device. The caller owns the
/// devices; this just dispatches press/release to the right one.
pub struct HoldController<'a> {
    pub mouse: &'a VirtualMouse,
    pub keyboard: &'a VirtualKeyboard,
}

impl<'a> HoldController<'a> {
    pub fn new(mouse: &'a VirtualMouse, keyboard: &'a VirtualKeyboard) -> Self {
        Self { mouse, keyboard }
    }

    /// Press the target down (and keep it down).
    pub fn hold(&self, target: HoldTarget) -> Result<()> {
        match target {
            HoldTarget::Mouse(b) => self.mouse.press(b),
            HoldTarget::Key(k) => self.keyboard.press(k),
        }
    }

    /// Release the target.
    pub fn release(&self, target: HoldTarget) -> Result<()> {
        match target {
            HoldTarget::Mouse(b) => self.mouse.release(b),
            HoldTarget::Key(k) => self.keyboard.release(k),
        }
    }
}
