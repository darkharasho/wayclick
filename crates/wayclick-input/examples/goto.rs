//! Manual smoke test for the input engine: position the pointer at an absolute
//! pixel and optionally click.
//!
//!   cargo run -p wayclick-input --example goto -- <x> <y> [left|right|middle]
//!
//! Note: a focused fullscreen app holding a pointer grab will block positioning.

use std::time::Duration;

use wayclick_input::{
    ClosedLoopPositioner, CursorReader, KwinCursorReader, MouseButton, PointerPositioner,
    VirtualMouse,
};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    if args.len() < 2 {
        eprintln!("usage: goto <x> <y> [left|right|middle]");
        std::process::exit(2);
    }
    let x: i32 = args[0].parse()?;
    let y: i32 = args[1].parse()?;
    let button = args.get(2).map(|s| match s.as_str() {
        "right" => MouseButton::Right,
        "middle" => MouseButton::Middle,
        _ => MouseButton::Left,
    });

    println!("creating virtual mouse...");
    let mouse = VirtualMouse::create()?;
    let reader = KwinCursorReader::new()?;
    let positioner = ClosedLoopPositioner::new(&mouse, &reader);

    println!("positioning to ({x}, {y})...");
    positioner.move_to(x, y)?;
    let (rx, ry) = reader.position()?;
    println!("arrived at ({rx}, {ry})");

    if let Some(b) = button {
        mouse.click(b, Duration::from_millis(40))?;
        println!("clicked {b:?}");
    }
    Ok(())
}
