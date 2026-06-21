//! A virtual keyboard over `/dev/uinput`, for the auto-hold-key action.
//!
//! Separate device from [`crate::VirtualMouse`] so udev classifies each cleanly
//! (mouse vs keyboard). Used to press and hold an arbitrary key down until
//! released — e.g. holding `W` in a game.

use std::{fs::File, thread::sleep, time::Duration};

use input_linux::{
    EventKind, EventTime, InputEvent, InputId, Key, KeyEvent, KeyState, SynchronizeEvent,
    UInputHandle,
    sys::{BUS_USB, input_event},
};

use crate::error::{InputError, Result};

/// The set of keys the virtual keyboard declares and the UI can offer. A device
/// can only emit keys it declared at creation, so this is the full pickable set.
pub fn common_keys() -> &'static [(&'static str, Key)] {
    use Key::*;
    &[
        ("A", A), ("B", B), ("C", C), ("D", D), ("E", E), ("F", F), ("G", G),
        ("H", H), ("I", I), ("J", J), ("K", K), ("L", L), ("M", M), ("N", N),
        ("O", O), ("P", P), ("Q", Q), ("R", R), ("S", S), ("T", T), ("U", U),
        ("V", V), ("W", W), ("X", X), ("Y", Y), ("Z", Z),
        ("0", Num0), ("1", Num1), ("2", Num2), ("3", Num3), ("4", Num4),
        ("5", Num5), ("6", Num6), ("7", Num7), ("8", Num8), ("9", Num9),
        ("Space", Space), ("Enter", Enter), ("Tab", Tab), ("Esc", Esc),
        ("Backspace", Backspace), ("CapsLock", CapsLock),
        ("LeftShift", LeftShift), ("RightShift", RightShift),
        ("LeftCtrl", LeftCtrl), ("RightCtrl", RightCtrl),
        ("LeftAlt", LeftAlt), ("RightAlt", RightAlt),
        ("LeftMeta", LeftMeta), ("RightMeta", RightMeta),
        ("Up", Up), ("Down", Down), ("Left", Left), ("Right", Right),
        ("F1", F1), ("F2", F2), ("F3", F3), ("F4", F4), ("F5", F5), ("F6", F6),
        ("F7", F7), ("F8", F8), ("F9", F9), ("F10", F10), ("F11", F11), ("F12", F12),
    ]
}

/// A keyboard key the engine can press. Wraps `input-linux`'s key so callers
/// don't depend on it directly.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Keycode(Key);

impl Keycode {
    /// Look up a key by its display name (see [`common_keys`]).
    pub fn from_name(name: &str) -> Option<Self> {
        common_keys()
            .iter()
            .find(|(n, _)| n.eq_ignore_ascii_case(name))
            .map(|(_, k)| Keycode(*k))
    }

    /// The display name for this key, if it's in the common set.
    pub fn name(self) -> Option<&'static str> {
        common_keys().iter().find(|(_, k)| *k == self.0).map(|(n, _)| *n)
    }
}

const REGISTER_SETTLE: Duration = Duration::from_millis(1200);

/// A virtual keyboard. Drops its uinput device on `Drop`.
pub struct VirtualKeyboard {
    handle: UInputHandle<File>,
}

impl VirtualKeyboard {
    pub fn create() -> Result<Self> {
        let kb = Self::create_no_wait()?;
        sleep(REGISTER_SETTLE);
        Ok(kb)
    }

    pub fn create_no_wait() -> Result<Self> {
        let file = std::fs::OpenOptions::new()
            .write(true)
            .open("/dev/uinput")
            .map_err(InputError::OpenUinput)?;
        let handle = UInputHandle::new(file);

        handle.set_evbit(EventKind::Key).map_err(InputError::Create)?;
        handle.set_evbit(EventKind::Synchronize).map_err(InputError::Create)?;
        for (_, key) in common_keys() {
            handle.set_keybit(*key).map_err(InputError::Create)?;
        }

        handle
            .create(
                &InputId { bustype: BUS_USB, vendor: 0x3232, product: 0x567b, version: 1 },
                b"wayclick-virtual-keyboard",
                0,
                &[],
            )
            .map_err(InputError::Create)?;

        Ok(Self { handle })
    }

    pub fn press(&self, key: Keycode) -> Result<()> {
        self.emit(key.0, KeyState::PRESSED)
    }

    pub fn release(&self, key: Keycode) -> Result<()> {
        self.emit(key.0, KeyState::RELEASED)
    }

    /// Press then release with a short hold.
    pub fn tap(&self, key: Keycode, hold: Duration) -> Result<()> {
        self.press(key)?;
        sleep(hold);
        self.release(key)
    }

    fn emit(&self, key: Key, state: KeyState) -> Result<()> {
        let evs: [input_event; 2] = [
            InputEvent::from(KeyEvent::new(EventTime::new(0, 0), key, state))
                .as_raw()
                .to_owned(),
            InputEvent::from(SynchronizeEvent::report(EventTime::new(0, 0)))
                .as_raw()
                .to_owned(),
        ];
        self.handle.write(&evs).map_err(InputError::Write)?;
        Ok(())
    }
}

impl Drop for VirtualKeyboard {
    fn drop(&mut self) {
        let _ = self.handle.dev_destroy();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn key_names_round_trip() {
        let k = Keycode::from_name("w").unwrap();
        assert_eq!(k.name(), Some("W"));
        assert_eq!(Keycode::from_name("Space").unwrap().name(), Some("Space"));
        assert!(Keycode::from_name("nope").is_none());
    }

    #[test]
    fn common_keys_are_unique_names() {
        let keys = common_keys();
        for (i, (n, _)) in keys.iter().enumerate() {
            assert!(
                !keys[i + 1..].iter().any(|(m, _)| m.eq_ignore_ascii_case(n)),
                "duplicate key name {n}"
            );
        }
    }
}
