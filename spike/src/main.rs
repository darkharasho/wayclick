//! libei feasibility spike, v2: SELF-CONTAINED drag.
//!   Q1: does a libei button HOLD register? Tested as a same-device drag —
//!       libei moves the cursor (absolute), presses, drags, releases. If text
//!       under the path highlights, the hold works.
//!   Q2: absolute pointer available? (printed)
//!   Q3: restore token for one-time consent? (printed)
//!
//! The cursor WILL visibly move to ~(700,500) and drag right — that's the test,
//! not the product behavior. Put a text editor / this chat where x≈700..1300,
//! y≈500 so there's text under the drag path.

use ashpd::desktop::{
    remote_desktop::{
        ConnectToEISOptions, DeviceType, RemoteDesktop, SelectDevicesOptions, StartOptions,
    },
    CreateSessionOptions, PersistMode,
};
use calloop::generic::Generic;
use enumflags2::BitFlags;
use once_cell::sync::Lazy;
use reis::{ei, PendingRequestResult};
use std::{
    collections::HashMap,
    io,
    os::unix::net::UnixStream,
    thread::sleep,
    time::{Duration, Instant},
};

const BTN_LEFT: u32 = 0x110;

static INTERFACES: Lazy<HashMap<&'static str, u32>> = Lazy::new(|| {
    let mut m = HashMap::new();
    for i in [
        "ei_callback",
        "ei_connection",
        "ei_seat",
        "ei_device",
        "ei_pingpong",
        "ei_pointer",
        "ei_pointer_absolute",
        "ei_button",
        "ei_scroll",
    ] {
        m.insert(i, 1);
    }
    m
});

#[derive(Default)]
struct SeatData {
    capabilities: HashMap<String, u64>,
}

#[derive(Default)]
struct DeviceData {
    device_type: Option<ei::device::DeviceType>,
    interfaces: HashMap<String, reis::Object>,
}

impl DeviceData {
    fn interface<T: reis::Interface>(&self) -> Option<T> {
        self.interfaces.get(T::NAME)?.clone().downcast()
    }
}

struct State {
    seats: HashMap<ei::Seat, SeatData>,
    devices: HashMap<ei::Device, DeviceData>,
    button: Option<ei::Button>,
    abs: Option<ei::PointerAbsolute>,
    device: Option<ei::Device>,
    serial: u32,
    sequence: u32,
    t0: Instant,
    tested: bool,
    done: bool,
}

impl State {
    fn micros(&self) -> u64 {
        self.t0.elapsed().as_micros() as u64
    }

    /// Self-contained drag: move → press → drag right → release, all via libei.
    fn run_drag(&mut self, context: &mut ei::Context) {
        let (Some(device), Some(button), Some(abs)) =
            (self.device.clone(), self.button.clone(), self.abs.clone())
        else {
            eprintln!("Q1: missing button/abs interface — cannot test");
            self.done = true;
            return;
        };
        println!("\n>>> self-contained drag at (700,500)->(1300,500) — WATCH for selection <<<\n");
        device.start_emulating(self.serial, self.sequence);
        self.sequence += 1;

        let frame = |ctx: &mut ei::Context, t: u64| {
            device.frame(self.serial, t);
            let _ = ctx.flush();
        };

        abs.motion_absolute(700.0, 500.0);
        frame(context, self.micros());
        sleep(Duration::from_millis(200));

        button.button(BTN_LEFT, ei::button::ButtonState::Press);
        frame(context, self.micros());
        sleep(Duration::from_millis(80));

        for i in 1..=30 {
            let x = 700.0 + i as f32 * 20.0;
            abs.motion_absolute(x, 500.0);
            frame(context, self.micros());
            sleep(Duration::from_millis(25));
        }

        button.button(BTN_LEFT, ei::button::ButtonState::Released);
        frame(context, self.micros());
        sleep(Duration::from_millis(80));

        device.stop_emulating(self.serial);
        let _ = context.flush();
        println!("\n>>> done. Did text highlight / did a drag happen? <<<");
        self.done = true;
    }

    fn handle_readable(&mut self, context: &mut ei::Context) -> io::Result<calloop::PostAction> {
        if context.read().is_err() {
            self.done = true;
            return Ok(calloop::PostAction::Remove);
        }
        while let Some(result) = context.pending_event() {
            let request = match result {
                PendingRequestResult::Request(r) => r,
                PendingRequestResult::ParseError(_) => continue,
                PendingRequestResult::InvalidObject(_) => continue,
            };
            match request {
                ei::Event::Handshake(handshake, req) => match req {
                    ei::handshake::Event::HandshakeVersion { version: _ } => {
                        handshake.handshake_version(1);
                        handshake.name("libei-spike");
                        handshake.context_type(ei::handshake::ContextType::Sender);
                        for (interface, version) in INTERFACES.iter() {
                            handshake.interface_version(interface, *version);
                        }
                        handshake.finish();
                    }
                    ei::handshake::Event::Connection { connection: _, serial } => {
                        self.serial = serial;
                    }
                    _ => {}
                },
                ei::Event::Connection(_c, req) => match req {
                    ei::connection::Event::Seat { seat } => {
                        self.seats.insert(seat, SeatData::default());
                    }
                    ei::connection::Event::Ping { ping } => ping.done(0),
                    _ => {}
                },
                ei::Event::Seat(seat, req) => {
                    let data = self.seats.get_mut(&seat).unwrap();
                    match req {
                        ei::seat::Event::Capability { mask, interface } => {
                            data.capabilities.insert(interface, mask);
                        }
                        ei::seat::Event::Done => {
                            let mut bind_mask = 0u64;
                            for (iface, mask) in &data.capabilities {
                                if matches!(
                                    iface.as_str(),
                                    "ei_pointer" | "ei_pointer_absolute" | "ei_button" | "ei_scroll"
                                ) {
                                    bind_mask |= mask;
                                }
                            }
                            println!(
                                "Q2: absolute pointer {}",
                                if data.capabilities.contains_key("ei_pointer_absolute") {
                                    "AVAILABLE"
                                } else {
                                    "NOT available"
                                }
                            );
                            seat.bind(bind_mask);
                        }
                        ei::seat::Event::Device { device } => {
                            self.devices.insert(device, DeviceData::default());
                        }
                        _ => {}
                    }
                }
                ei::Event::Device(device, req) => {
                    let data = self.devices.get_mut(&device).unwrap();
                    match req {
                        ei::device::Event::DeviceType { device_type } => {
                            data.device_type = Some(device_type);
                        }
                        ei::device::Event::Interface { object } => {
                            data.interfaces.insert(object.interface().to_owned(), object);
                        }
                        ei::device::Event::Done => {
                            // Prefer the device that has BOTH button and absolute pointer.
                            if let (Some(button), Some(abs)) =
                                (data.interface::<ei::Button>(), data.interface::<ei::PointerAbsolute>())
                            {
                                println!("using device type={:?} (button + absolute)", data.device_type);
                                self.button = Some(button);
                                self.abs = Some(abs);
                                self.device = Some(device.clone());
                            }
                        }
                        ei::device::Event::Resumed { serial } => {
                            self.serial = serial;
                            if !self.tested && self.abs.is_some() {
                                self.tested = true;
                                self.run_drag(context);
                            }
                        }
                        _ => {}
                    }
                }
                _ => {}
            }
        }
        let _ = context.flush();
        Ok(calloop::PostAction::Continue)
    }
}

async fn open_connection() -> ei::Context {
    let rd = RemoteDesktop::new().await.unwrap();
    let session = rd.create_session(CreateSessionOptions::default()).await.unwrap();
    let options = SelectDevicesOptions::default()
        .set_devices(BitFlags::from(DeviceType::Pointer))
        .set_persist_mode(PersistMode::ExplicitlyRevoked);
    rd.select_devices(&session, options).await.unwrap();
    let resp = rd
        .start(&session, None, StartOptions::default())
        .await
        .unwrap()
        .response()
        .unwrap();
    println!("Q3: restore_token = {:?}", resp.restore_token());
    let fd = rd.connect_to_eis(&session, ConnectToEISOptions::default()).await.unwrap();
    ei::Context::new(UnixStream::from(fd)).unwrap()
}

fn main() {
    let rt = tokio::runtime::Runtime::new().unwrap();
    let context = rt.block_on(open_connection());
    let _handshake = context.handshake();
    let _ = context.flush();

    let mut event_loop = calloop::EventLoop::<State>::try_new().unwrap();
    let handle = event_loop.handle();
    let source = Generic::new(context, calloop::Interest::READ, calloop::Mode::Level);
    handle
        .insert_source(source, |_event, context, state: &mut State| {
            state.handle_readable(unsafe { context.get_mut() })
        })
        .unwrap();

    let mut state = State {
        seats: HashMap::new(),
        devices: HashMap::new(),
        button: None,
        abs: None,
        device: None,
        serial: u32::MAX,
        sequence: 0,
        t0: Instant::now(),
        tested: false,
        done: false,
    };

    while !state.done {
        event_loop.dispatch(Some(Duration::from_millis(100)), &mut state).unwrap();
    }
    let _ = event_loop.dispatch(Some(Duration::from_millis(200)), &mut state);
    println!("\nspike done.");
}
