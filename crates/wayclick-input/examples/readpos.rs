//! Diagnostic: time consecutive cursor-position reads.
//!
//!   cargo run -p wayclick-input --example readpos -- [count]

use std::time::Instant;

use wayclick_input::{CursorReader, KwinCursorReader};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let count: u32 = std::env::args().nth(1).and_then(|s| s.parse().ok()).unwrap_or(5);
    let reader = KwinCursorReader::new()?;
    let mut total_ms = 0u128;
    for i in 0..count {
        let t = Instant::now();
        let res = reader.position();
        let ms = t.elapsed().as_millis();
        total_ms += ms;
        match res {
            Ok((x, y)) => println!("read {i}: ({x}, {y}) in {ms} ms"),
            Err(e) => println!("read {i}: ERROR after {ms} ms: {e}"),
        }
    }
    println!("avg: {} ms over {count} reads", total_ms / count as u128);
    Ok(())
}
