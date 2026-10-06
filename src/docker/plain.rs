//! Compose output with the terminal escapes taken out, for a reader that is
//! not a terminal.

use super::{compose_args, exit_code, spawn_error, trace_command};
use crate::error::Result;
use crate::project::Project;
use std::io::{BufRead, BufReader, Write};
use std::process::{Command, Stdio};

/// Run `docker compose <args> <extra>` with stdout passed on line by line,
/// every ANSI escape removed.
///
/// `docker compose logs --no-color` only drops compose's own colouring of the
/// service prefix; what the containers print is passed through untouched, and
/// a model that colours its own log lines does so into a file or a pipe too.
/// Line by line rather than all at once, so `logs -f` still streams.
pub fn run_compose_plain(project: &Project, extra: &[String]) -> Result<i32> {
    let mut args = compose_args(project);
    super::with_quiet_progress(&mut args);
    args.extend(extra.iter().cloned());
    trace_command(&args);

    let mut child = Command::new("docker")
        .args(&args)
        .current_dir(&project.dir)
        .stdin(Stdio::inherit())
        .stdout(Stdio::piped())
        .stderr(Stdio::inherit())
        .spawn()
        .map_err(|e| spawn_error(&e))?;

    if let Some(stream) = child.stdout.take() {
        let mut reader = BufReader::new(stream);
        let mut line = Vec::new();
        let mut out = std::io::stdout().lock();
        loop {
            line.clear();
            match reader.read_until(b'\n', &mut line) {
                Ok(0) | Err(_) => break,
                Ok(_) => {
                    // A reader that went away (`| head`) ends the copy, not
                    // the command: compose is still waited for below.
                    if out.write_all(&strip_ansi(&line)).is_err() || out.flush().is_err() {
                        break;
                    }
                }
            }
        }
    }
    let status = child
        .wait()
        .map_err(|e| anyhow::anyhow!("waiting for docker: {e}"))?;
    Ok(exit_code(status))
}

/// `bytes` without its ANSI escape sequences: CSI (`ESC [ ... final`), OSC
/// (`ESC ] ... BEL` or `ESC ] ... ESC \`) and the short `ESC x` and `ESC ( B` forms.
///
/// An escape cut off at the end of the input is dropped with the rest of it.
pub fn strip_ansi(bytes: &[u8]) -> Vec<u8> {
    const ESC: u8 = 0x1b;
    const BEL: u8 = 0x07;
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] != ESC {
            out.push(bytes[i]);
            i += 1;
            continue;
        }
        match bytes.get(i + 1) {
            Some(b'[') => {
                i += 2;
                while i < bytes.len() && !(0x40..=0x7e).contains(&bytes[i]) {
                    i += 1;
                }
                i += 1;
            }
            Some(b']') => {
                i += 2;
                while i < bytes.len() {
                    if bytes[i] == BEL {
                        i += 1;
                        break;
                    }
                    if bytes[i] == ESC && bytes.get(i + 1) == Some(&b'\\') {
                        i += 2;
                        break;
                    }
                    i += 1;
                }
            }
            // `ESC x`, or `ESC ( B` and the like: intermediates, then a final.
            Some(_) => {
                i += 1;
                while i < bytes.len() && (0x20..=0x2f).contains(&bytes[i]) {
                    i += 1;
                }
                i += 1;
            }
            None => i += 1,
        }
    }
    out
}
