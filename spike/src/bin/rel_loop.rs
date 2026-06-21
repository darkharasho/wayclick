// Relative-mouse closed-loop prober for Wayclick.
//
// libinput/KWin ignore absolute axes from a virtual mouse, but a RELATIVE mouse
// is fully supported (all buttons). Combined with reading KWin's real cursor
// position (workspace.cursorPos, which Wayland otherwise hides from clients),
// we can position the pointer at an absolute pixel by emitting relative deltas
// in a feedback loop, then click with any button.
//
// This binary creates one persistent relative mouse and reads commands from
// stdin, one per line, so an outside controller (the closed loop) can drive it
// without paying device re-registration cost each step:
//   m <dx> <dy>   emit a relative motion (REL_X/REL_Y)
//   l             left click   (BTN_LEFT press+release)
//   r             right click  (BTN_RIGHT)
//   c             middle click (BTN_MIDDLE)
//   q             quit
//
// Usage: rel_loop   (then feed commands on stdin)

use std::io::BufRead;

use input_linux::{
    EventKind, EventTime, InputEvent, InputId, Key, KeyEvent, KeyState, RelativeAxis,
    RelativeEvent, SynchronizeEvent, UInputHandle,
    sys::{BUS_USB, input_event},
};

fn now() -> EventTime {
    EventTime::new(0, 0)
}

fn syn(uinput: &UInputHandle<std::fs::File>) {
    let e: [input_event; 1] = [InputEvent::from(SynchronizeEvent::report(now()))
        .as_raw()
        .to_owned()];
    uinput.write(&e).unwrap();
}

fn rel(uinput: &UInputHandle<std::fs::File>, dx: i32, dy: i32) {
    let mut evs: Vec<input_event> = Vec::new();
    if dx != 0 {
        evs.push(
            InputEvent::from(RelativeEvent::new(now(), RelativeAxis::X, dx))
                .as_raw()
                .to_owned(),
        );
    }
    if dy != 0 {
        evs.push(
            InputEvent::from(RelativeEvent::new(now(), RelativeAxis::Y, dy))
                .as_raw()
                .to_owned(),
        );
    }
    evs.push(
        InputEvent::from(SynchronizeEvent::report(now()))
            .as_raw()
            .to_owned(),
    );
    uinput.write(&evs).unwrap();
}

fn click(uinput: &UInputHandle<std::fs::File>, key: Key) {
    for state in [KeyState::PRESSED, KeyState::RELEASED] {
        let e: [input_event; 1] = [InputEvent::from(KeyEvent::new(now(), key, state))
            .as_raw()
            .to_owned()];
        uinput.write(&e).unwrap();
        syn(uinput);
        std::thread::sleep(std::time::Duration::from_millis(20));
    }
}

fn main() {
    let file = std::fs::OpenOptions::new()
        .write(true)
        .open("/dev/uinput")
        .expect("open /dev/uinput");
    let uinput = UInputHandle::new(file);

    uinput.set_evbit(EventKind::Relative).unwrap();
    uinput.set_evbit(EventKind::Key).unwrap();
    uinput.set_evbit(EventKind::Synchronize).unwrap();
    uinput.set_relbit(RelativeAxis::X).unwrap();
    uinput.set_relbit(RelativeAxis::Y).unwrap();
    uinput.set_keybit(Key::ButtonLeft).unwrap();
    uinput.set_keybit(Key::ButtonRight).unwrap();
    uinput.set_keybit(Key::ButtonMiddle).unwrap();

    uinput
        .create(
            &InputId {
                bustype: BUS_USB,
                vendor: 0x3232,
                product: 0x567a,
                version: 1,
            },
            b"wayclick-rel-loop",
            0,
            &[],
        )
        .expect("create");

    // Let KWin enumerate the device before we start driving it.
    std::thread::sleep(std::time::Duration::from_millis(1200));
    println!("READY");

    let stdin = std::io::stdin();
    for line in stdin.lock().lines() {
        let line = match line {
            Ok(l) => l,
            Err(_) => break,
        };
        let mut it = line.split_whitespace();
        match it.next() {
            Some("m") => {
                let dx: i32 = it.next().unwrap_or("0").parse().unwrap_or(0);
                let dy: i32 = it.next().unwrap_or("0").parse().unwrap_or(0);
                rel(&uinput, dx, dy);
                println!("OK m {dx} {dy}");
            }
            Some("l") => {
                click(&uinput, Key::ButtonLeft);
                println!("OK l");
            }
            Some("r") => {
                click(&uinput, Key::ButtonRight);
                println!("OK r");
            }
            Some("c") => {
                click(&uinput, Key::ButtonMiddle);
                println!("OK c");
            }
            Some("q") | None => break,
            _ => println!("ERR unknown"),
        }
        use std::io::Write;
        std::io::stdout().flush().ok();
    }

    let _ = uinput.dev_destroy();
}
