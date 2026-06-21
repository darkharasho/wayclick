//! Manual smoke test for the click engine.
//!
//!   cargo run -p wayclick-input --example click -- [count] [interval_ms]
//!
//! Follow-cursor mode: clicks wherever the pointer currently is. Move the
//! pointer over a safe target first.

use std::time::Duration;

use wayclick_input::{
    ClickConfig, ClickEngine, ClosedLoopPositioner, KwinCursorReader, Repeat, StopFlag, VirtualMouse,
};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let count: u64 = args.first().and_then(|s| s.parse().ok()).unwrap_or(3);
    let interval_ms: u64 = args.get(1).and_then(|s| s.parse().ok()).unwrap_or(300);

    let mouse = VirtualMouse::create()?;
    let reader = KwinCursorReader::new()?;
    // Positioner is unused in follow-cursor mode but wired up to show the shape.
    let positioner = ClosedLoopPositioner::new(&mouse, &reader);
    let engine = ClickEngine::new(&mouse, Some(&positioner));

    let cfg = ClickConfig {
        interval: Duration::from_millis(interval_ms),
        repeat: Repeat::Count(count),
        ..Default::default()
    };

    let stop = StopFlag::new();
    println!("clicking {count}x every {interval_ms}ms (follow-cursor)...");
    let done = engine.run(&cfg, &stop, 0x9E3779B97F4A7C15)?;
    println!("done: {done} clicks");
    Ok(())
}
