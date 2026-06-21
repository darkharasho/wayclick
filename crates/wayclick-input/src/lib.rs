//! Wayclick input engine.
//!
//! Splits cleanly into two layers:
//! - [`VirtualMouse`] — a relative pointer over `/dev/uinput`. Portable and
//!   proven; libinput accepts it everywhere and all buttons reach all apps.
//! - Positioning ([`CursorReader`] + [`PointerPositioner`]) — compositor-specific.
//!   On KWin there is no absolute virtual pointer, so absolute targets are
//!   reached by a closed loop: read the real cursor, nudge, repeat.
//!
//! See the crate's design notes for why absolute uinput devices (abs mouse,
//! touchscreen, virtual tablet) do not work on KWin.

mod cursor;
mod engine;
mod error;
mod hold;
mod keyboard;
mod mouse;
mod positioner;

pub use cursor::{CursorReader, KwinCursorReader};
pub use engine::{ClickConfig, ClickEngine, ClickKind, Repeat, Rng, StopFlag, Target};
pub use error::{InputError, Result};
pub use hold::{HoldController, HoldTarget};
pub use keyboard::{Keycode, VirtualKeyboard, common_keys};
pub use mouse::{MouseButton, VirtualMouse};
pub use positioner::{ClosedLoopPositioner, LoopConfig, PointerPositioner};
