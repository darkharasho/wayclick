//! Tap a named key once via a virtual keyboard. Used to test global hotkeys.
//!   cargo run -p wayclick-input --example tapkey -- F6

use std::time::Duration;
use wayclick_input::{Keycode, VirtualKeyboard};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let name = std::env::args().nth(1).unwrap_or_else(|| "F6".into());
    let key = Keycode::from_name(&name).ok_or("unknown key")?;
    let kb = VirtualKeyboard::create()?;
    kb.tap(key, Duration::from_millis(40))?;
    println!("tapped {name}");
    Ok(())
}
