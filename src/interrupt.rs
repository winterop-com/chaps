//! Ctrl-C for the commands that start something and must take it back out
//! when they are stopped part-way: the first Ctrl-C asks the command to stop
//! and clean up, and a second one exits at once.
//!
//! Only a command that calls [`install`] handles it. Every other command
//! keeps the default, where the first Ctrl-C ends it.

use std::sync::atomic::{AtomicBool, Ordering};

static REQUESTED: AtomicBool = AtomicBool::new(false);

/// The exit code of a command that Ctrl-C stopped, as a shell gives it.
pub const EXIT_CODE: i32 = 130;

/// Handle Ctrl-C from here on. Best-effort: a handler that cannot be set
/// leaves the default, where Ctrl-C ends the command at once.
pub fn install() {
    let _ = ctrlc::set_handler(|| {
        if REQUESTED.swap(true, Ordering::SeqCst) {
            std::process::exit(EXIT_CODE);
        }
    });
}

/// Whether Ctrl-C was pressed since [`install`].
pub fn requested() -> bool {
    REQUESTED.load(Ordering::SeqCst)
}
