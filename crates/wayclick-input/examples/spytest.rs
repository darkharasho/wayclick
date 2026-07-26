//! Diagnostic: can our zbus object server receive a Cursor callback at all?
//! Calls our own service via `busctl`, bypassing KWin completely.

use std::sync::mpsc;
use std::time::Duration;

struct Spy {
    tx: mpsc::Sender<(i32, i32, i32)>,
}

#[zbus::interface(name = "org.wayclick.spy")]
impl Spy {
    fn cursor(&self, seq: i32, x: i32, y: i32) {
        let _ = self.tx.send((seq, x, y));
    }
}

fn main() {
    let rt = tokio::runtime::Builder::new_multi_thread()
        .worker_threads(1)
        .enable_all()
        .build()
        .unwrap();

    // Async API driven by our own runtime: zbus's *blocking* API in tokio
    // flavor parks its internals on a private runtime that only advances
    // during blocking calls — incoming dispatch freezes between them.
    let (tx, rx) = mpsc::channel();
    let service = format!("org.wayclick.spytest.p{}", std::process::id());
    let conn = rt.block_on(async {
        let conn = zbus::Connection::session().await.unwrap();
        conn.object_server().at("/c", Spy { tx }).await.unwrap();
        conn.request_name(service.as_str()).await.unwrap();
        conn
    });
    let _keep = &conn;
    println!("listening as {service}");

    let status = std::process::Command::new("busctl")
        .args(["--user", "call", &service, "/c", "org.wayclick.spy", "Cursor", "iii", "5", "10", "20"])
        .status()
        .unwrap();
    println!("busctl exit: {status}");

    match rx.recv_timeout(Duration::from_secs(2)) {
        Ok(v) => println!("RECEIVED: {v:?}"),
        Err(e) => println!("NOT RECEIVED: {e}"),
    }
}
