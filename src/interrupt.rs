//! Ctrl-C for the commands that start something and must take it back out
//! when they are stopped part-way: the first Ctrl-C asks the command to stop
//! and clean up, and a second one exits at once.
//!
//! Only a command that calls [`install`] handles it. Every other command
//! keeps the default, where the first Ctrl-C ends it.
//!
//! A terminal sends Ctrl-C to the whole process group, so the `docker run`
//! of a throwaway container stops too. When that happens before the
//! container starts, `--rm` never applies: the container stays in state
//! `Created` and holds its volume. A container started under a
//! [`Throwaway`] guard has a known name, and is removed on the way out.

use std::process::{Command, Stdio};
use std::sync::Mutex;
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};

static REQUESTED: AtomicBool = AtomicBool::new(false);

/// The names of the throwaway containers that run now.
static THROWAWAYS: Mutex<Vec<String>> = Mutex::new(Vec::new());

/// The exit code of a command that Ctrl-C stopped, as a shell gives it.
pub const EXIT_CODE: i32 = 130;

/// Handle Ctrl-C from here on. Best-effort: a handler that cannot be set
/// leaves the default, where Ctrl-C ends the command at once.
pub fn install() {
    let _ = ctrlc::set_handler(|| {
        if REQUESTED.swap(true, Ordering::SeqCst) {
            // The second Ctrl-C exits at once, so no guard is dropped: the
            // containers they track are removed here instead.
            let names = THROWAWAYS
                .lock()
                .map(|names| names.clone())
                .unwrap_or_default();
            for name in names {
                remove_container(&name);
            }
            std::process::exit(EXIT_CODE);
        }
    });
}

/// Whether Ctrl-C was pressed since [`install`].
pub fn requested() -> bool {
    REQUESTED.load(Ordering::SeqCst)
}

/// A throwaway container this process starts with `docker run --rm --name`.
///
/// When the run did not end cleanly, or Ctrl-C was pressed, drop removes the
/// container with `docker rm -f`. A container that ran to the end is already
/// gone through `--rm`, so a clean run costs no extra docker call.
#[derive(Debug)]
pub struct Throwaway {
    name: String,
    clean: bool,
}

impl Throwaway {
    /// A new, unique name with `prefix`, tracked until the guard is dropped.
    pub fn new(prefix: &str) -> Self {
        static NEXT: AtomicU32 = AtomicU32::new(0);
        let name = format!(
            "{prefix}-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::SeqCst)
        );
        if let Ok(mut names) = THROWAWAYS.lock() {
            names.push(name.clone());
        }
        Self { name, clean: false }
    }

    /// The name to pass to `docker run --name`.
    pub fn name(&self) -> &str {
        &self.name
    }

    /// Record that the run ended cleanly, so drop has nothing to remove.
    pub fn finished(&mut self) {
        self.clean = true;
    }
}

impl Drop for Throwaway {
    fn drop(&mut self) {
        if let Ok(mut names) = THROWAWAYS.lock() {
            names.retain(|name| name != &self.name);
        }
        if !self.clean || requested() {
            remove_container(&self.name);
        }
    }
}

/// `docker rm -f <name>`, best-effort and silent: a container that is not
/// there is the result this asks for.
fn remove_container(name: &str) {
    crate::output::verbose(&format!("$ docker rm -f {name}"));
    let _ = Command::new("docker")
        .args(["rm", "-f", name])
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status();
}

#[cfg(test)]
mod tests;
