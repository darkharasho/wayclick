// Absolute-pointer spike for Wayclick.
//
// Proves the load-bearing unknown: can a uinput device that declares
// EV_ABS + ABS_X/ABS_Y reposition the Wayland pointer to an ABSOLUTE pixel
// and click there? TheClicker is relative-only, so this path is greenfield.
//
// Usage:
//   abs-pointer-spike <x> <y> [--no-click] [--max-x N] [--max-y N]
//                              [--hold-ms N] [--settle-ms N] [--register-ms N]
//
// Defaults map the axis range to this machine's combined desktop (6880x1440),
// so <x> <y> are interpreted as desktop pixels.

use std::{thread::sleep, time::Duration};

use input_linux::{
    AbsoluteAxis, AbsoluteEvent, AbsoluteInfo, AbsoluteInfoSetup, EventKind, EventTime, InputEvent,
    InputId, InputProperty, Key, KeyEvent, KeyState, SynchronizeEvent, UInputHandle,
    sys::{BUS_USB, input_event},
};

const VENDOR: u16 = 0x3232;
const PRODUCT: u16 = 0x5679; // distinct from TheClicker's 0x5678
const VERSION: u16 = 0x0001;

struct Config {
    x: i32,
    y: i32,
    max_x: i32,
    max_y: i32,
    click: bool,
    hold_ms: u64,
    settle_ms: u64,
    register_ms: u64,
    hold_secs: u64,
    sweep_secs: u64,
    // device-shape experiments
    prop_pointer: bool, // INPUT_PROP_POINTER (absolute coords map to screen as a pointer)
    prop_direct: bool,  // INPUT_PROP_DIRECT (touchscreen-style)
    touch: bool,        // declare BTN_TOUCH
    pen: bool,          // declare BTN_TOOL_PEN
    touch_click: bool,  // click using BTN_TOUCH (+ BTN_TOOL_PEN if --pen) instead of BTN_LEFT
    no_mouse_btns: bool, // omit BTN_LEFT/RIGHT/MIDDLE so udev won't tag ID_INPUT_MOUSE
}

fn parse_args() -> Config {
    let mut cfg = Config {
        x: -1,
        y: -1,
        max_x: 6880,
        max_y: 1440,
        click: true,
        hold_ms: 40,
        settle_ms: 400,
        register_ms: 1200,
        hold_secs: 0,
        sweep_secs: 0,
        prop_pointer: false,
        prop_direct: false,
        touch: false,
        pen: false,
        touch_click: false,
        no_mouse_btns: false,
    };
    let mut positional = Vec::new();
    let mut args = std::env::args().skip(1);
    while let Some(a) = args.next() {
        match a.as_str() {
            "--no-click" => cfg.click = false,
            "--max-x" => cfg.max_x = args.next().unwrap().parse().unwrap(),
            "--max-y" => cfg.max_y = args.next().unwrap().parse().unwrap(),
            "--hold-ms" => cfg.hold_ms = args.next().unwrap().parse().unwrap(),
            "--settle-ms" => cfg.settle_ms = args.next().unwrap().parse().unwrap(),
            "--register-ms" => cfg.register_ms = args.next().unwrap().parse().unwrap(),
            // --hold N: create device, sleep N secs (inspect /proc/bus/input/devices), exit.
            "--hold" => cfg.hold_secs = args.next().unwrap().parse().unwrap(),
            // --sweep N: slowly drag the pointer left->right across y for N secs.
            "--sweep" => cfg.sweep_secs = args.next().unwrap().parse().unwrap(),
            "--pointer" => cfg.prop_pointer = true,
            "--direct" => cfg.prop_direct = true,
            "--touch" => cfg.touch = true,
            "--pen" => cfg.pen = true,
            "--touch-click" => cfg.touch_click = true,
            "--no-mouse-btns" => cfg.no_mouse_btns = true,
            _ => positional.push(a),
        }
    }
    if positional.len() == 2 {
        cfg.x = positional[0].parse().expect("x must be an integer");
        cfg.y = positional[1].parse().expect("y must be an integer");
    } else if cfg.hold_secs == 0 && cfg.sweep_secs == 0 {
        eprintln!("usage: abs-pointer-spike <x> <y> [flags] | --hold N [x y] | --sweep N");
        std::process::exit(2);
    }
    cfg
}

fn now() -> EventTime {
    // Wall-clock isn't required for synthetic events; zero timestamps are fine
    // and the kernel fills them in. Keep it simple and deterministic.
    EventTime::new(0, 0)
}

fn main() {
    let cfg = parse_args();

    let file = std::fs::OpenOptions::new()
        .write(true)
        .open("/dev/uinput")
        .expect("open /dev/uinput (need rw access via uaccess ACL or input group)");

    let uinput = UInputHandle::new(file);

    // Event types: absolute axes, keys (buttons), and sync.
    uinput.set_evbit(EventKind::Absolute).unwrap();
    uinput.set_evbit(EventKind::Key).unwrap();
    uinput.set_evbit(EventKind::Synchronize).unwrap();

    // Buttons.
    if !cfg.no_mouse_btns {
        uinput.set_keybit(Key::ButtonLeft).unwrap();
        uinput.set_keybit(Key::ButtonRight).unwrap();
        uinput.set_keybit(Key::ButtonMiddle).unwrap();
    }
    if cfg.touch {
        uinput.set_keybit(Key::ButtonTouch).unwrap();
    }
    if cfg.pen {
        uinput.set_keybit(Key::ButtonToolPen).unwrap();
    }

    // Optional input properties that change how libinput classifies the device.
    if cfg.prop_pointer {
        uinput.set_propbit(InputProperty::Pointer).unwrap();
    }
    if cfg.prop_direct {
        uinput.set_propbit(InputProperty::Direct).unwrap();
    }

    // Absolute axes.
    uinput.set_absbit(AbsoluteAxis::X).unwrap();
    uinput.set_absbit(AbsoluteAxis::Y).unwrap();

    let abs_info = |axis, max| AbsoluteInfoSetup {
        axis,
        info: AbsoluteInfo {
            value: 0,
            minimum: 0,
            maximum: max,
            fuzz: 0,
            flat: 0,
            resolution: 0,
        },
    };

    uinput
        .create(
            &InputId {
                bustype: BUS_USB,
                vendor: VENDOR,
                product: PRODUCT,
                version: VERSION,
            },
            b"wayclick-abs-spike",
            0,
            &[
                abs_info(AbsoluteAxis::X, cfg.max_x),
                abs_info(AbsoluteAxis::Y, cfg.max_y),
            ],
        )
        .expect("create uinput device");

    println!(
        "device created: range x[0..{}] y[0..{}], target ({}, {}), click={}",
        cfg.max_x, cfg.max_y, cfg.x, cfg.y, cfg.click
    );

    // Give libinput/KWin time to enumerate the new device before we send events.
    println!("waiting {}ms for compositor to register device...", cfg.register_ms);
    sleep(Duration::from_millis(cfg.register_ms));

    if cfg.hold_secs > 0 {
        // For tablet/touch devices the pointer only follows while the tool is in
        // proximity / the touch is down, so optionally hold contact during the park.
        let contact = cfg.pen || cfg.touch_click;
        if cfg.x >= 0 && cfg.y >= 0 {
            if cfg.pen {
                button(&uinput, Key::ButtonToolPen, KeyState::PRESSED);
            }
            move_to(&uinput, cfg.x, cfg.y);
            if cfg.touch_click {
                button(&uinput, Key::ButtonTouch, KeyState::PRESSED);
            }
            move_to(&uinput, cfg.x, cfg.y);
            println!(
                "parked pointer at ({}, {}){}",
                cfg.x,
                cfg.y,
                if contact { " (contact held)" } else { "" }
            );
        }
        println!(
            "holding device open for {}s -- inspect now:\n  grep -A8 wayclick /proc/bus/input/devices",
            cfg.hold_secs
        );
        sleep(Duration::from_secs(cfg.hold_secs));
        if contact {
            if cfg.touch_click {
                button(&uinput, Key::ButtonTouch, KeyState::RELEASED);
            }
            if cfg.pen {
                button(&uinput, Key::ButtonToolPen, KeyState::RELEASED);
            }
        }
        let _ = uinput.dev_destroy();
        println!("destroyed.");
        return;
    }

    if cfg.sweep_secs > 0 {
        let steps = cfg.sweep_secs * 30; // ~30 Hz
        let y = cfg.max_y / 2;
        println!("sweeping x[0..{}] at y={} for {}s...", cfg.max_x, y, cfg.sweep_secs);
        for i in 0..=steps {
            let x = (cfg.max_x as i64 * i as i64 / steps as i64) as i32;
            move_to(&uinput, x, y);
            sleep(Duration::from_millis(33));
        }
        let _ = uinput.dev_destroy();
        println!("destroyed.");
        return;
    }

    move_to(&uinput, cfg.x, cfg.y);
    println!("moved to ({}, {})", cfg.x, cfg.y);
    sleep(Duration::from_millis(cfg.settle_ms));

    if cfg.click {
        // Re-assert position right before the click so it lands where intended.
        move_to(&uinput, cfg.x, cfg.y);
        if cfg.touch_click {
            // Touchscreen/tablet-style contact: tool down, touch down, ... up.
            if cfg.pen {
                button(&uinput, Key::ButtonToolPen, KeyState::PRESSED);
            }
            button(&uinput, Key::ButtonTouch, KeyState::PRESSED);
            sleep(Duration::from_millis(cfg.hold_ms));
            button(&uinput, Key::ButtonTouch, KeyState::RELEASED);
            if cfg.pen {
                button(&uinput, Key::ButtonToolPen, KeyState::RELEASED);
            }
        } else {
            button(&uinput, Key::ButtonLeft, KeyState::PRESSED);
            sleep(Duration::from_millis(cfg.hold_ms));
            button(&uinput, Key::ButtonLeft, KeyState::RELEASED);
        }
        println!("clicked at ({}, {})", cfg.x, cfg.y);
    }

    sleep(Duration::from_millis(200));
    let _ = uinput.dev_destroy();
    println!("done.");
}

fn move_to(uinput: &UInputHandle<std::fs::File>, x: i32, y: i32) {
    let events: [input_event; 3] = [
        InputEvent::from(AbsoluteEvent::new(now(), AbsoluteAxis::X, x))
            .as_raw()
            .to_owned(),
        InputEvent::from(AbsoluteEvent::new(now(), AbsoluteAxis::Y, y))
            .as_raw()
            .to_owned(),
        InputEvent::from(SynchronizeEvent::report(now()))
            .as_raw()
            .to_owned(),
    ];
    uinput.write(&events).expect("write move events");
}

fn button(uinput: &UInputHandle<std::fs::File>, key: Key, state: KeyState) {
    let events: [input_event; 2] = [
        InputEvent::from(KeyEvent::new(now(), key, state))
            .as_raw()
            .to_owned(),
        InputEvent::from(SynchronizeEvent::report(now()))
            .as_raw()
            .to_owned(),
    ];
    uinput.write(&events).expect("write button events");
}
