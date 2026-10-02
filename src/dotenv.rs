//! `.env`, read and written by one set of rules: docker compose's.
//!
//! Compose reads `.env` last and is the only thing that acts on it, so its
//! rules are the only ones that matter:
//!
//! - the **last** active assignment of a variable wins, so a file carrying two
//!   `CHAP_API_TOKEN=` lines is protected by the second one;
//! - `export KEY=value` assigns the same thing as `KEY=value`, and is how a
//!   file that is also `source`d in a shell gets written;
//! - a value wrapped in matching single or double quotes does not include
//!   them, and anything after the closing quote is ignored;
//! - single quotes are literal, but for `\'`: no `$VAR` is expanded inside
//!   them, which is why [`encode`] writes a secret that way;
//! - an unquoted value ends at a ` #` - a `#` with a space before it - which
//!   starts a comment, so `PORT=8001 # mine` is `8001` while `val#ue` is all
//!   one value;
//! - a line whose first non-blank character is `#` assigns nothing.
//!
//! Measured against Compose v5 with `docker compose config`, not recalled.
//! Two of its rules are not reproduced, because chaps has no environment to
//! expand them from: `$VAR` in an unquoted or double-quoted value is read as
//! the text it is, and so is `$$`. Nothing chaps writes contains either.
//!
//! Every reader and every writer in this CLI goes through here, because a
//! reader and a writer that disagree about which line is live is exactly how
//! `chaps auth rotate` came to report a rotation it had not made: it rewrote
//! the first `CHAP_API_TOKEN=` line while compose went on reading the second.
//! [`set`] therefore rewrites the first active assignment *and deletes every
//! later duplicate*, so after any write there is exactly one active line and
//! the reader and the writer cannot drift apart.

/// The value the last active `key=` line of a `.env` body assigns, quotes
/// stripped and surrounding whitespace dropped.
///
/// `None` when no active line assigns it. `Some("")` for a bare `KEY=`, which
/// is a line that sets nothing: callers reading a value chap-core treats as
/// `os.getenv(...) or None` want [`non_empty`] instead.
pub fn value(body: &str, key: &str) -> Option<String> {
    body.lines()
        .filter_map(|line| assignment(line, key))
        .map(decode)
        .next_back()
}

/// [`value`], with an empty assignment read as unset.
///
/// `CHAP_API_TOKEN=` disables authentication exactly as a commented line does,
/// and an empty `CHAP_API_PORT=` is no port at all, so "set" has to mean "set
/// to something".
pub fn non_empty(body: &str, key: &str) -> Option<String> {
    value(body, key).filter(|value| !value.is_empty())
}

/// The value of the last commented `# key=` line that still carries one.
///
/// [`comment_out`] keeps the value behind the `#`, so this is how
/// `chaps auth enable` hands a deployment back the very token its clients are
/// already configured with, rather than a new one nobody has yet. An empty
/// placeholder carries no value and is skipped.
pub fn commented_value(body: &str, key: &str) -> Option<String> {
    body.lines()
        .filter_map(|line| assignment(line.trim_start().strip_prefix('#')?, key))
        .map(decode)
        .rfind(|value| !value.is_empty())
}

/// What [`set`] found in the file, and therefore what it did.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Wrote {
    /// The variable's first active assignment was rewritten in place, and
    /// `duplicates` further active assignments of it were removed.
    Active { duplicates: usize },
    /// No active assignment, so the commented placeholder the generated `.env`
    /// ships was uncommented in place.
    Uncommented,
    /// The file does not mention the variable at all, which is the caller's
    /// cue to append it. Nothing was changed.
    Absent,
}

/// Assign `value` to `key` in a `.env` split into `lines`, leaving exactly one
/// active assignment of it behind.
///
/// The value goes through [`encode`], so what compose reads back is `value`
/// exactly - never an expansion of a `$` inside it. An inline comment on the
/// line that is rewritten stays on it.
///
/// In order: the first active assignment is rewritten and every later
/// duplicate removed, else the commented placeholder is uncommented in place,
/// else nothing happens and the caller is told to append. Comments, blank
/// lines, ordering and every other variable survive byte for byte - and so
/// does an `export ` prefix, which still assigns the same thing to compose and
/// is what makes `. ./.env` work in a shell.
pub fn set(lines: &mut Vec<String>, key: &str, value: &str) -> Wrote {
    let active: Vec<usize> = lines
        .iter()
        .enumerate()
        .filter(|(_, line)| assignment(line, key).is_some())
        .map(|(at, _)| at)
        .collect();

    if let Some((&first, rest)) = active.split_first() {
        let prefix = if is_export(&lines[first]) {
            "export "
        } else {
            ""
        };
        let comment = assignment(&lines[first], key)
            .map(trailing_comment)
            .unwrap_or_default();
        lines[first] = format!("{prefix}{key}={}{comment}", encode(value));
        // A later duplicate is the line compose would have read instead, so
        // leaving one behind is leaving the old value in force.
        for &at in rest.iter().rev() {
            lines.remove(at);
        }
        return Wrote::Active {
            duplicates: rest.len(),
        };
    }

    match lines.iter().position(|line| is_commented(line, key)) {
        Some(at) => {
            lines[at] = format!("{key}={}", encode(value));
            Wrote::Uncommented
        }
        None => Wrote::Absent,
    }
}

/// Comment out every active `key=` line of a `.env` body, keeping the value
/// behind the `#`.
///
/// Every one of them, because any single line left active still sets the
/// variable. The value is kept so `chaps auth enable` can recover it, and so
/// an operator who turned authentication off by mistake still has the token
/// their clients were configured with.
pub fn comment_out(body: &str, key: &str) -> String {
    let lines: Vec<String> = body
        .lines()
        .map(|line| match assignment(line, key) {
            Some(_) => format!("# {line}"),
            None => line.to_string(),
        })
        .collect();
    join(&lines)
}

/// Lines back into a file body, one newline each.
pub fn join(lines: &[String]) -> String {
    let mut out = String::new();
    for line in lines {
        out.push_str(line);
        out.push('\n');
    }
    out
}

/// The value an active `key=` line assigns, trimmed but still quoted; `None`
/// for a comment, a blank line, a line with no `=` and any other variable.
fn assignment<'a>(line: &'a str, key: &str) -> Option<&'a str> {
    let line = line.trim();
    if line.is_empty() || line.starts_with('#') {
        return None;
    }
    let (name, value) = strip_export(line).split_once('=')?;
    (name.trim() == key).then(|| value.trim())
}

/// A line without its `export ` prefix, if it had one.
fn strip_export(line: &str) -> &str {
    line.strip_prefix("export ")
        .map_or(line, |rest| rest.trim_start())
}

fn is_export(line: &str) -> bool {
    line.trim_start().starts_with("export ")
}

/// The characters a value may carry unquoted and mean the same thing to
/// compose and to every shell that sources the file.
fn is_plain(c: char) -> bool {
    c.is_ascii_alphanumeric() || "-_.:/+=@,~%".contains(c)
}

/// `value` as the text after `KEY=` that compose reads back as exactly
/// `value`.
///
/// Plain as it is when every character is one that means nothing to compose
/// or a shell, which is every secret chaps generates. Otherwise in single
/// quotes, which compose reads literally: a token of `$HOME` written bare is
/// expanded - to an empty string for a variable that is not set, which is a
/// deployment whose API token is empty and therefore off. [`literal_problem`]
/// is what keeps out the characters single quotes cannot carry.
pub fn encode(value: &str) -> String {
    if value.chars().all(is_plain) {
        return value.to_string();
    }
    format!("'{}'", value.replace('\'', "\\'"))
}

/// Why `value` cannot be written into `.env` so that compose reads it back
/// unchanged, or `None` when it can.
///
/// A quote, a backslash, or a control character such as a newline: inside
/// single quotes compose reads `\'` as a quote, so a value ending in a
/// backslash would swallow its own closing quote.
pub fn literal_problem(value: &str) -> Option<&'static str> {
    value
        .chars()
        .any(|c| c == '\'' || c == '"' || c == '\\' || c.is_control())
        .then_some(
            "a quote, a backslash or a control character, which `.env` cannot hold as written",
        )
}

/// The value compose assigns for the text after `KEY=`, already trimmed.
fn decode(raw: &str) -> String {
    decoded(raw).0
}

/// [`decode`], and how many bytes of `raw` the value took up, so the caller
/// can tell what came after it.
///
/// A quote that is never closed is part of the value, the way compose's own
/// parser leaves it to the rest of the line.
fn decoded(raw: &str) -> (String, usize) {
    let mut chars = raw.char_indices().peekable();
    match chars.next() {
        Some((_, quote @ ('\'' | '"'))) => {
            let mut out = String::new();
            while let Some((at, c)) = chars.next() {
                match c {
                    c if c == quote => return (out, at + 1),
                    '\\' => match (quote, chars.peek().map(|&(_, next)| next)) {
                        ('\'', Some('\'')) => {
                            chars.next();
                            out.push('\'');
                        }
                        ('"', Some(next @ ('"' | '\\'))) => {
                            chars.next();
                            out.push(next);
                        }
                        ('"', Some('n')) => {
                            chars.next();
                            out.push('\n');
                        }
                        _ => out.push('\\'),
                    },
                    c => out.push(c),
                }
            }
            (raw.to_string(), raw.len())
        }
        _ => match raw.find(" #") {
            Some(at) => (raw[..at].trim_end().to_string(), at),
            None => (raw.to_string(), raw.len()),
        },
    }
}

/// The ` # comment` after a value, with its leading space, or `""`.
fn trailing_comment(raw: &str) -> String {
    let (_, used) = decoded(raw);
    let rest = raw[used..].trim();
    match rest.starts_with('#') {
        true => format!(" {rest}"),
        false => String::new(),
    }
}

/// `# KEY=...`, with any amount of whitespace around the `#`.
fn is_commented(line: &str, key: &str) -> bool {
    line.trim_start()
        .strip_prefix('#')
        .and_then(|rest| assignment(rest, key))
        .is_some()
}

/// Write `.env` the one way every command does: to a temporary file of this
/// process's own, readable by its owner only, renamed over the old one. A
/// crash or a full disk then leaves the old file whole - it holds the
/// database password, which nothing else records - and the API token, the
/// registration key and the passwords in it are never readable by other
/// users of the machine.
pub fn write(path: &std::path::Path, body: &str) -> crate::error::Result<()> {
    use std::io::Write;
    let name = path
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_default();
    let tmp = path.with_file_name(format!(".{name}.{}.tmp", std::process::id()));
    let _ = std::fs::remove_file(&tmp);
    let mut options = std::fs::OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    let written = options.open(&tmp).and_then(|mut file| {
        file.write_all(body.as_bytes())?;
        file.sync_all()
    });
    if let Err(e) = written {
        let _ = std::fs::remove_file(&tmp);
        return Err(anyhow::anyhow!("writing {}: {e}", tmp.display()));
    }
    std::fs::rename(&tmp, path).map_err(|e| {
        let _ = std::fs::remove_file(&tmp);
        anyhow::anyhow!("renaming {} to {}: {e}", tmp.display(), path.display())
    })
}

/// Make an existing `.env` readable by its owner only, as [`write`] creates
/// it: for one an older chaps wrote, or one a restore copied in. Best-effort;
/// a file that is not there is left alone.
pub fn protect(path: &std::path::Path) {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        if let Ok(meta) = std::fs::metadata(path)
            && meta.permissions().mode() & 0o077 != 0
        {
            let _ = std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600));
        }
    }
    #[cfg(not(unix))]
    let _ = path;
}

#[cfg(test)]
mod tests;
