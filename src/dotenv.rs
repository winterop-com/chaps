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
//!   them;
//! - a line whose first non-blank character is `#` assigns nothing.
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
        .map(|value| unquote(value).to_string())
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
        .map(|value| unquote(value).to_string())
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
        lines[first] = format!("{prefix}{key}={value}");
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
            lines[at] = format!("{key}={value}");
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

/// A value without its matching surrounding quotes, if it had a pair.
fn unquote(value: &str) -> &str {
    for quote in ['"', '\''] {
        if let Some(inner) = value
            .strip_prefix(quote)
            .and_then(|rest| rest.strip_suffix(quote))
        {
            return inner;
        }
    }
    value
}

/// `# KEY=...`, with any amount of whitespace around the `#`.
fn is_commented(line: &str, key: &str) -> bool {
    line.trim_start()
        .strip_prefix('#')
        .and_then(|rest| assignment(rest, key))
        .is_some()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn lines(body: &str) -> Vec<String> {
        body.lines().map(str::to_string).collect()
    }

    /// Compose reads the file top to bottom and keeps overwriting, so the last
    /// active assignment is the one in force. Reading the first is the bug
    /// this module exists to make impossible.
    #[test]
    fn the_last_active_assignment_wins() {
        assert_eq!(
            value("CHAP_API_TOKEN=one\nCHAP_API_TOKEN=two\n", "CHAP_API_TOKEN").as_deref(),
            Some("two")
        );
        // Three of them, and a commented one that counts for nothing.
        let body = "K=a\n# K=b\nK=c\nK=d\n";
        assert_eq!(value(body, "K").as_deref(), Some("d"));
        // A later commented line does not take the value away.
        assert_eq!(value("K=a\n# K=b\n", "K").as_deref(), Some("a"));
    }

    #[test]
    fn an_export_prefix_assigns_the_same_thing() {
        assert_eq!(value("export K=v\n", "K").as_deref(), Some("v"));
        assert_eq!(value("  export   K=v\n", "K").as_deref(), Some("v"));
        // `export` is not part of the name, and a variable merely called
        // `exportK` is a different one.
        assert_eq!(value("exportK=v\n", "K"), None);
        assert_eq!(value("export K=v\nK=w\n", "K").as_deref(), Some("w"));
    }

    #[test]
    fn matching_quotes_are_not_part_of_the_value() {
        assert_eq!(value("K=\"v w\"\n", "K").as_deref(), Some("v w"));
        assert_eq!(value("K='v w'\n", "K").as_deref(), Some("v w"));
        assert_eq!(value("export K=\"v\"\n", "K").as_deref(), Some("v"));
        // Only a matching pair: a lone quote belongs to the value.
        assert_eq!(value("K=\"v\n", "K").as_deref(), Some("\"v"));
        assert_eq!(value("K=\"\n", "K").as_deref(), Some("\""));
        assert_eq!(value("K='v\"\n", "K").as_deref(), Some("'v\""));
        // And an empty pair is an empty value.
        assert_eq!(value("K=\"\"\n", "K").as_deref(), Some(""));
        assert_eq!(non_empty("K=\"\"\n", "K"), None);
    }

    #[test]
    fn comments_blanks_and_other_variables_assign_nothing() {
        let body = "# K=commented\n\n   # K=indented\nOTHER=v\nK_SUFFIX=v\nPREFIX_K=v\nJUNK\n";
        assert_eq!(value(body, "K"), None);
        assert_eq!(value("", "K"), None);
        // A bare assignment is a line that sets nothing, which is not the same
        // as no line at all.
        assert_eq!(value("K=\n", "K").as_deref(), Some(""));
        assert_eq!(non_empty("K=\n", "K"), None);
        assert_eq!(non_empty("K=  \n", "K"), None);
    }

    /// The rotation bug, from the other side: a second active line must not
    /// survive the write, or compose goes on reading the old secret.
    #[test]
    fn writing_rewrites_the_first_assignment_and_removes_the_duplicates() {
        let mut body = lines("K=old\n# a comment\nK=older\nOTHER=v\nK=oldest\n");
        assert_eq!(set(&mut body, "K", "new"), Wrote::Active { duplicates: 2 });
        assert_eq!(join(&body), "K=new\n# a comment\nOTHER=v\n");
        assert_eq!(value(&join(&body), "K").as_deref(), Some("new"));
        assert_eq!(
            join(&body).lines().filter(|l| l.starts_with("K=")).count(),
            1
        );

        // One line is the ordinary case, and it changes nothing else.
        let mut body = lines("K=old\n# a comment\nOTHER=v\n");
        assert_eq!(set(&mut body, "K", "new"), Wrote::Active { duplicates: 0 });
        assert_eq!(join(&body), "K=new\n# a comment\nOTHER=v\n");
    }

    #[test]
    fn writing_keeps_an_export_prefix_and_leaves_commented_lines_alone() {
        let mut body = lines("export K=old\n");
        assert_eq!(set(&mut body, "K", "new"), Wrote::Active { duplicates: 0 });
        assert_eq!(join(&body), "export K=new\n");

        // A commented line is documentation, not an assignment: it is left as
        // it is when there is an active line to write.
        let mut body = lines("# K=documented\nK=old\n");
        assert_eq!(set(&mut body, "K", "new"), Wrote::Active { duplicates: 0 });
        assert_eq!(join(&body), "# K=documented\nK=new\n");
    }

    #[test]
    fn writing_uncomments_a_placeholder_when_there_is_no_active_line() {
        let mut body = lines("# before\n# K=\n# after\n");
        assert_eq!(set(&mut body, "K", "new"), Wrote::Uncommented);
        assert_eq!(join(&body), "# before\nK=new\n# after\n");

        // The first placeholder is the one that gets the value.
        let mut body = lines("# K=\n# K=\n");
        assert_eq!(set(&mut body, "K", "new"), Wrote::Uncommented);
        assert_eq!(join(&body), "K=new\n# K=\n");
    }

    #[test]
    fn a_variable_the_file_never_mentions_is_the_callers_to_append() {
        let mut body = lines("OTHER=v\n");
        assert_eq!(set(&mut body, "K", "new"), Wrote::Absent);
        assert_eq!(join(&body), "OTHER=v\n", "nothing was touched");
    }

    #[test]
    fn commenting_out_takes_every_active_line_and_keeps_the_values() {
        let out = comment_out("K=a\n# K=b\nOTHER=v\nexport K=c\n", "K");
        assert_eq!(out, "# K=a\n# K=b\nOTHER=v\n# export K=c\n");
        assert_eq!(value(&out, "K"), None);
        // The last one is what `enable` recovers, the way compose read it.
        assert_eq!(commented_value(&out, "K").as_deref(), Some("c"));
        // An already commented line is not commented twice.
        assert!(!comment_out(&out, "K").contains("# # K"));
        // Nothing to comment out is no change at all.
        assert_eq!(comment_out("OTHER=v\n", "K"), "OTHER=v\n");
    }

    #[test]
    fn a_commented_placeholder_carries_no_value_to_recover() {
        assert_eq!(commented_value("# K=\n", "K"), None);
        assert_eq!(commented_value("K=active\n", "K"), None);
        assert_eq!(
            commented_value("#   K=\"v w\"\n", "K").as_deref(),
            Some("v w")
        );
    }
}
