/// Errors produced by the Wayclick input engine.
#[derive(Debug, thiserror::Error)]
pub enum InputError {
    #[error("opening /dev/uinput (need rw access via the udev rule + group, or a uaccess ACL): {0}")]
    OpenUinput(#[source] std::io::Error),

    #[error("writing to the virtual device: {0}")]
    Write(#[source] std::io::Error),

    #[error("creating the virtual device: {0}")]
    Create(#[source] std::io::Error),

    #[error("reading the cursor position from the compositor: {0}")]
    CursorRead(String),

    #[error(
        "could not position the pointer: target ({tx},{ty}), reached ({x},{y}) after {steps} steps. \
         A focused fullscreen app holding a pointer grab can block positioning."
    )]
    NotConverged {
        tx: i32,
        ty: i32,
        x: i32,
        y: i32,
        steps: u32,
    },
}

pub type Result<T> = std::result::Result<T, InputError>;
