// Proper virtual graphics tablet (Wacom-Intuos-style) for Wayclick.
//
// A minimal abs/touch/pen descriptor is ignored by libinput. A *complete*
// tablet descriptor is not: ABS_X/ABS_Y with resolution, ABS_PRESSURE,
// BTN_TOOL_PEN (proximity), BTN_TOUCH (tip), BTN_STYLUS/STYLUS2, and
// INPUT_PROP_POINTER (indirect tablet -> maps its area to the whole desktop).
//
// If libinput/KWin accept this, we get TRUE absolute positioning in one shot
// (no closed loop, no compositor-specific cursor read) and tablet is a
// standard protocol across KWin/GNOME/wlroots -- the portable path.
//
// Modes:
//   park  <x> <y> <secs>        proximity-in at pixel (x,y), hold, proximity-out
//   click <x> <y> [l|r|m]       position then click (tip / stylus / stylus2)
// Options: --dw <desktop_w> --dh <desktop_h>  (pixel->abs mapping space; default 6880x1440)

use std::{thread::sleep, time::Duration};

use input_linux::{
    AbsoluteAxis, AbsoluteEvent, AbsoluteInfo, AbsoluteInfoSetup, EventKind, EventTime, InputEvent,
    InputId, InputProperty, Key, KeyEvent, KeyState, SynchronizeEvent, UInputHandle,
    sys::{BUS_USB, input_event},
};

const ABS_MAX: i32 = 32767;
const PRESSURE_MAX: i32 = 4096;

fn now() -> EventTime {
    EventTime::new(0, 0)
}

struct Tablet {
    h: UInputHandle<std::fs::File>,
    dw: i32,
    dh: i32,
}

impl Tablet {
    fn ax(&self, x: i32) -> i32 {
        ((x as i64 * ABS_MAX as i64) / self.dw.max(1) as i64) as i32
    }
    fn ay(&self, y: i32) -> i32 {
        ((y as i64 * ABS_MAX as i64) / self.dh.max(1) as i64) as i32
    }

    fn emit(&self, evs: &[input_event]) {
        self.h.write(evs).expect("write");
    }

    fn abs(&self, axis: AbsoluteAxis, v: i32) -> input_event {
        InputEvent::from(AbsoluteEvent::new(now(), axis, v)).as_raw().to_owned()
    }
    fn key(&self, k: Key, s: KeyState) -> input_event {
        InputEvent::from(KeyEvent::new(now(), k, s)).as_raw().to_owned()
    }
    fn syn(&self) -> input_event {
        InputEvent::from(SynchronizeEvent::report(now())).as_raw().to_owned()
    }

    // Enter proximity and position the tool (pen hovering, no tip contact).
    fn proximity_in(&self, x: i32, y: i32) {
        self.emit(&[
            self.key(Key::ButtonToolPen, KeyState::PRESSED),
            self.abs(AbsoluteAxis::X, self.ax(x)),
            self.abs(AbsoluteAxis::Y, self.ay(y)),
            self.abs(AbsoluteAxis::Pressure, 0),
            self.syn(),
        ]);
    }
    fn move_to(&self, x: i32, y: i32) {
        self.emit(&[
            self.abs(AbsoluteAxis::X, self.ax(x)),
            self.abs(AbsoluteAxis::Y, self.ay(y)),
            self.syn(),
        ]);
    }
    fn proximity_out(&self) {
        self.emit(&[
            self.abs(AbsoluteAxis::Pressure, 0),
            self.key(Key::ButtonTouch, KeyState::RELEASED),
            self.key(Key::ButtonToolPen, KeyState::RELEASED),
            self.syn(),
        ]);
    }
    fn tip_down(&self) {
        self.emit(&[
            self.abs(AbsoluteAxis::Pressure, PRESSURE_MAX),
            self.key(Key::ButtonTouch, KeyState::PRESSED),
            self.syn(),
        ]);
    }
    fn tip_up(&self) {
        self.emit(&[
            self.key(Key::ButtonTouch, KeyState::RELEASED),
            self.abs(AbsoluteAxis::Pressure, 0),
            self.syn(),
        ]);
    }
    fn stylus(&self, k: Key, down: bool) {
        let s = if down { KeyState::PRESSED } else { KeyState::RELEASED };
        self.emit(&[self.key(k, s), self.syn()]);
    }
}

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let mut dw = 6880;
    let mut dh = 1440;
    let mut pos = Vec::new();
    let mut i = 0;
    while i < args.len() {
        match args[i].as_str() {
            "--dw" => { i += 1; dw = args[i].parse().unwrap(); }
            "--dh" => { i += 1; dh = args[i].parse().unwrap(); }
            s => pos.push(s.to_string()),
        }
        i += 1;
    }
    if pos.is_empty() {
        eprintln!("usage: tablet park <x> <y> <secs> | tablet click <x> <y> [l|r|m]");
        std::process::exit(2);
    }

    let file = std::fs::OpenOptions::new().write(true).open("/dev/uinput").expect("open uinput");
    let h = UInputHandle::new(file);

    // --- Descriptor: complete indirect tablet ---
    h.set_evbit(EventKind::Key).unwrap();
    h.set_evbit(EventKind::Absolute).unwrap();
    h.set_evbit(EventKind::Synchronize).unwrap();

    h.set_keybit(Key::ButtonToolPen).unwrap();
    h.set_keybit(Key::ButtonTouch).unwrap();
    h.set_keybit(Key::ButtonStylus).unwrap();
    h.set_keybit(Key::ButtonStylus2).unwrap();

    h.set_propbit(InputProperty::Pointer).unwrap(); // indirect -> maps to whole desktop

    h.set_absbit(AbsoluteAxis::X).unwrap();
    h.set_absbit(AbsoluteAxis::Y).unwrap();
    h.set_absbit(AbsoluteAxis::Pressure).unwrap();

    let abs = |axis, max, res| AbsoluteInfoSetup {
        axis,
        info: AbsoluteInfo { value: 0, minimum: 0, maximum: max, fuzz: 0, flat: 0, resolution: res },
    };

    h.create(
        &InputId { bustype: BUS_USB, vendor: 0x056a /* Wacom */, product: 0x00de, version: 0x0100 },
        b"wayclick-virtual-tablet",
        0,
        &[
            abs(AbsoluteAxis::X, ABS_MAX, 100),
            abs(AbsoluteAxis::Y, ABS_MAX, 100),
            abs(AbsoluteAxis::Pressure, PRESSURE_MAX, 0),
        ],
    )
    .expect("create tablet");

    let t = Tablet { h, dw, dh };
    println!("virtual tablet created (maps {}x{} -> abs {}). registering...", dw, dh, ABS_MAX);
    sleep(Duration::from_millis(1500));

    match pos[0].as_str() {
        "park" => {
            let x: i32 = pos[1].parse().unwrap();
            let y: i32 = pos[2].parse().unwrap();
            let secs: u64 = pos.get(3).map(|s| s.parse().unwrap()).unwrap_or(3);
            t.proximity_in(x, y);
            t.move_to(x, y);
            println!("pen in proximity at ({x},{y}), holding {secs}s");
            sleep(Duration::from_secs(secs));
            t.proximity_out();
        }
        "click" => {
            let x: i32 = pos[1].parse().unwrap();
            let y: i32 = pos[2].parse().unwrap();
            let btn = pos.get(3).map(|s| s.as_str()).unwrap_or("l");
            t.proximity_in(x, y);
            t.move_to(x, y);
            sleep(Duration::from_millis(300));
            match btn {
                "r" => { t.stylus(Key::ButtonStylus, true); sleep(Duration::from_millis(40)); t.stylus(Key::ButtonStylus, false); }
                "m" => { t.stylus(Key::ButtonStylus2, true); sleep(Duration::from_millis(40)); t.stylus(Key::ButtonStylus2, false); }
                _ => { t.tip_down(); sleep(Duration::from_millis(40)); t.tip_up(); }
            }
            sleep(Duration::from_millis(200));
            t.proximity_out();
            println!("clicked '{btn}' at ({x},{y})");
        }
        other => { eprintln!("unknown mode {other}"); }
    }

    sleep(Duration::from_millis(200));
    let _ = t.h.dev_destroy();
    println!("done.");
}
