//! Diagnostic: closed-loop positioning round trip.
//!
//! Waits for the pointer to be idle (so it never fights a human), then moves
//! it to a nearby target, verifies convergence, and puts it back. Prints PASS
//! or FAIL with timings.
//!
//!   cargo run -p wayclick-input --example roundtrip -- [max-idle-wait-secs]

use std::time::{Duration, Instant};

use wayclick_input::{
    ClosedLoopPositioner, CursorReader, KwinCursorReader, PointerPositioner, VirtualMouse,
};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let max_wait: u64 = std::env::args().nth(1).and_then(|s| s.parse().ok()).unwrap_or(60);
    let reader = KwinCursorReader::new()?;

    // Politeness gate: require 4s of no pointer motion before touching it.
    let deadline = Instant::now() + Duration::from_secs(max_wait);
    let mut still_since = Instant::now();
    let mut last = reader.position()?;
    loop {
        std::thread::sleep(Duration::from_millis(500));
        let now = reader.position()?;
        if now != last {
            still_since = Instant::now();
            last = now;
        } else if still_since.elapsed() >= Duration::from_secs(4) {
            break;
        }
        if Instant::now() > deadline {
            println!("SKIPPED: pointer never went idle within {max_wait}s");
            return Ok(());
        }
    }

    let mouse = VirtualMouse::create()?;
    let positioner = ClosedLoopPositioner::new(&mouse, &reader);
    let (ox, oy) = reader.position()?;
    // Short hop: stays inside any pointer-confining window the cursor rests
    // in (a focused game confining the pointer is an environmental failure,
    // not a positioner bug — see the NotConverged docs).
    let (tx, ty) = (if ox > 400 { ox - 60 } else { ox + 60 }, if oy > 400 { oy - 60 } else { oy + 60 });

    let t = Instant::now();
    let out = positioner.move_to(tx, ty);
    let move_ms = t.elapsed().as_millis();
    let (rx, ry) = reader.position()?;
    let hit = (rx - tx).abs() <= 1 && (ry - ty).abs() <= 1;

    // Always try to put the pointer back where the user left it.
    let t = Instant::now();
    let back = positioner.move_to(ox, oy);
    let back_ms = t.elapsed().as_millis();
    let (bx, by) = reader.position()?;
    let restored = (bx - ox).abs() <= 1 && (by - oy).abs() <= 1;

    println!(
        "{}: ({ox},{oy}) -> ({tx},{ty}) landed ({rx},{ry}) in {move_ms} ms ({out:?}); \
         restored ({bx},{by}) in {back_ms} ms ({back:?})",
        if hit && restored { "PASS" } else { "FAIL" }
    );
    Ok(())
}
