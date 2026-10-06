//! Running an external command with a deadline.

use super::*;

/// What came of running one external command.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Outcome {
    /// It ran to completion, for better or worse.
    Done {
        ok: bool,
        stdout: String,
        stderr: String,
    },
    /// The binary is not on PATH.
    Missing,
    /// It was still running when its deadline passed, and was killed.
    TimedOut,
    /// It could not be started for some other reason.
    Failed(String),
}

impl Outcome {
    /// Whether the command ran and exited zero.
    pub fn succeeded(&self) -> bool {
        matches!(self, Outcome::Done { ok: true, .. })
    }

    /// What the command printed on stdout, empty when it never ran.
    pub fn stdout(&self) -> &str {
        match self {
            Outcome::Done { stdout, .. } => stdout,
            _ => "",
        }
    }

    /// Whatever the command said about its own failure.
    pub fn stderr(&self) -> &str {
        match self {
            Outcome::Done { stderr, .. } => stderr,
            Outcome::Failed(why) => why,
            Outcome::Missing => "the binary is not on PATH",
            Outcome::TimedOut => "it did not finish in time",
        }
    }
}

/// Run `program args...` and give up on it after `timeout`.
///
/// `std::process` has no deadline of its own, so the child is polled and
/// killed rather than waited on: a wedged daemon must not be able to hold
/// `varde doctor` open. Every output here is small enough to sit in the pipe
/// buffer while the child is polled, so nothing can deadlock on an unread
/// pipe.
pub fn run_bounded(program: &str, args: &[&str], timeout: Duration) -> Outcome {
    crate::output::verbose(&format!("$ {program} {}", args.join(" ")));
    let mut child = match Command::new(program)
        .args(args)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
    {
        Ok(child) => child,
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => return Outcome::Missing,
        Err(err) => return Outcome::Failed(format!("could not run `{program}`: {err}")),
    };

    let deadline = Instant::now() + timeout;
    loop {
        match child.try_wait() {
            Ok(Some(status)) => {
                return match child.wait_with_output() {
                    Ok(output) => Outcome::Done {
                        ok: status.success(),
                        stdout: String::from_utf8_lossy(&output.stdout).trim().to_string(),
                        stderr: String::from_utf8_lossy(&output.stderr).trim().to_string(),
                    },
                    Err(err) => Outcome::Failed(format!("reading from `{program}`: {err}")),
                };
            }
            Ok(None) => {
                if Instant::now() >= deadline {
                    let _ = child.kill();
                    let _ = child.wait();
                    return Outcome::TimedOut;
                }
                std::thread::sleep(POLL);
            }
            Err(err) => return Outcome::Failed(format!("waiting for `{program}`: {err}")),
        }
    }
}

/// The first non-empty line of some output, short enough for one column.
pub(super) fn first_line(text: &str) -> String {
    let line = text
        .lines()
        .map(str::trim)
        .find(|line| !line.is_empty())
        .unwrap_or("")
        .to_string();
    if line.chars().count() <= 120 {
        return line;
    }
    format!("{}...", line.chars().take(117).collect::<String>())
}

#[cfg(test)]
mod tests;
