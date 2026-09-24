//! `chaps api` — one authenticated request to chap-core, and its answer.
//!
//! The escape hatch under every other command that talks to CHAP: `chaps
//! status` asks two fixed questions and `chaps jobs` reads one endpoint, and
//! everything else chap-core can do is reached from here. It is deliberately
//! thin - a base URL, a token and a body - so that the things it is used for
//! (building a dataset, starting a backtest, reading the metrics back) can be
//! written down in the book as commands rather than as curl invocations with a
//! token pasted into them.
//!
//! What it adds over `curl` is exactly three things: the base URL of *this*
//! deployment, the `Authorization` header from its `.env`, and an exit code
//! that tells "CHAP is not up" (2) from "CHAP said no" (1).

use crate::api::{Answer, Api};
use crate::cli::ApiArgs;
use crate::commands::Ctx;
use crate::error::{ChapError, Result};
use crate::project::Project;
use std::io::{Read, Write};
use std::time::Duration;

/// Exit code for a request that came back with a status that is not a 2xx.
///
/// The body is still printed: a 422 from chap-core says which field it did not
/// like, and that is the answer even though the request failed.
const EXIT_HTTP_ERROR: i32 = 1;

/// `chaps api METHOD PATH`.
pub fn run(ctx: &Ctx, args: &ApiArgs) -> Result<()> {
    let method = crate::api::method_of(&args.method)?;
    let path = crate::api::path_of(&args.path)?;
    let body = args.data.as_deref().map(read_data).transpose()?;

    let project = resolve_project(ctx, args)?;
    let base = match (&args.url, &project) {
        (Some(url), _) => url.trim().to_string(),
        (None, Some(project)) => project.api_url(),
        // Unreachable: `resolve_project` fails when there is neither.
        (None, None) => return Err(no_target()),
    };
    let token = crate::api::token_for(project.as_ref().map(|p| p.dir.as_path()));
    let api = Api::new(&base, token, Duration::from_secs(args.timeout));

    let answer = api.send_json(&method, &path, body.as_deref())?;
    print_body(ctx, &answer, args.raw)?;
    report(ctx, &api, &answer);
    if !answer.is_success() {
        std::process::exit(EXIT_HTTP_ERROR);
    }
    Ok(())
}

/// The deployment this request belongs to, if there is one.
///
/// With `--url` a project is a convenience - it is where the token comes from
/// when the URL is this deployment's own - so a directory that is not one is
/// not an error. Without `--url` the project *is* the address, and there is
/// nothing to send the request to without it.
fn resolve_project(ctx: &Ctx, args: &ApiArgs) -> Result<Option<Project>> {
    if args.url.is_some() {
        return Ok(Project::find(&ctx.project_dir).ok());
    }
    ctx.project().map(Some).map_err(|e| {
        if matches!(
            e.downcast_ref::<ChapError>(),
            Some(ChapError::NotAProject(_))
        ) {
            no_target()
        } else {
            e
        }
    })
}

/// The error for a request with no deployment and no `--url` to aim at.
fn no_target() -> anyhow::Error {
    anyhow::anyhow!(
        "no deployment here to send the request to; run this inside a project, \
         or pass --url to name the chap-core API"
    )
}

/// Write the body the way it reads best.
///
/// Three renderings, in the order they are decided:
///
/// - `--raw` writes the bytes through untouched, for a body that is not text
///   or that has to be compared byte for byte.
/// - A JSON *string* is printed as its text, without the quotes and with its
///   escapes resolved. This is what makes `chaps api GET /v1/jobs/ID/logs`
///   read like a log rather than like one enormous quoted line; `--json` turns
///   it off, because a caller piping into a parser wants the document.
/// - Anything else that is JSON is pretty-printed with two spaces, and
///   anything that is not JSON is written as it arrived.
fn print_body(ctx: &Ctx, answer: &Answer, raw: bool) -> Result<()> {
    let stdout = std::io::stdout();
    let mut out = stdout.lock();
    if raw {
        out.write_all(&answer.body)?;
        out.flush()?;
        return Ok(());
    }
    match answer.json() {
        Some(serde_json::Value::String(text)) if !ctx.out.json => write_text(&mut out, &text)?,
        Some(value) => {
            let pretty = serde_json::to_string_pretty(&value)?;
            writeln!(out, "{pretty}")?;
        }
        None => write_text(&mut out, &answer.text())?,
    }
    out.flush()?;
    Ok(())
}

/// Write text with exactly one trailing newline, and nothing at all when it is
/// empty: a blank line is not an answer.
fn write_text(out: &mut impl Write, text: &str) -> Result<()> {
    if text.is_empty() {
        return Ok(());
    }
    if text.ends_with('\n') {
        write!(out, "{text}")?;
    } else {
        writeln!(out, "{text}")?;
    }
    Ok(())
}

/// Say on stderr what came back, when the body alone does not.
///
/// stdout belongs to the answer, so everything here is stderr: the status line
/// of a request that failed, the status of one that succeeded with no body at
/// all, and the one hint that is worth giving - a 401 is about the token and
/// nothing else.
fn report(ctx: &Ctx, api: &Api, answer: &Answer) {
    if !answer.is_success() {
        eprintln!("{}", answer.status_line());
        if let Some(hint) = hint(api, answer) {
            crate::output::notice(&hint);
        }
        return;
    }
    if answer.body.is_empty() {
        ctx.out.verbose("the response has no body");
        eprintln!("{}", answer.status_line());
    }
}

/// The next step for a status code that has one.
fn hint(api: &Api, answer: &Answer) -> Option<String> {
    match answer.status {
        401 if api.has_token() => Some(
            "chap-core did not accept the API token; `chaps auth show --reveal` prints the one \
             this deployment holds"
                .to_string(),
        ),
        401 => Some(
            "chap-core wants an API token and none was sent; `chaps auth show` says whether this \
             deployment has one"
                .to_string(),
        ),
        _ => None,
    }
}

/// The `--data` value, read from wherever it points.
///
/// Three spellings, so a body can be typed, kept in a file or piped in:
/// `-` is stdin, `@path` is a file, and anything else is the JSON itself. The
/// text is parsed before it is sent - not re-serialised, so what reaches
/// chap-core is byte for byte what was given - because a body with a comma
/// missing should be a sentence here rather than a 422 from the other end.
pub fn read_data(spec: &str) -> Result<String> {
    let text = match source_of(spec) {
        Source::Stdin => {
            let mut text = String::new();
            std::io::stdin()
                .read_to_string(&mut text)
                .map_err(|e| anyhow::anyhow!("reading the request body from stdin: {e}"))?;
            text
        }
        Source::File(path) => std::fs::read_to_string(path)
            .map_err(|e| anyhow::anyhow!("reading the request body from {path}: {e}"))?,
        Source::Inline(text) => text.to_string(),
    };
    validate(&text, spec)?;
    Ok(text)
}

/// Where a `--data` value comes from.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Source<'a> {
    Inline(&'a str),
    File(&'a str),
    Stdin,
}

/// Read a `--data` value as one of the three spellings.
pub fn source_of(spec: &str) -> Source<'_> {
    if spec == "-" {
        return Source::Stdin;
    }
    match spec.strip_prefix('@') {
        Some(path) => Source::File(path),
        None => Source::Inline(spec),
    }
}

/// Refuse a body that is not JSON, naming where it came from.
fn validate(text: &str, spec: &str) -> Result<()> {
    if text.trim().is_empty() {
        return Err(ChapError::Usage(format!("--data {} is empty", whence(spec))).into());
    }
    serde_json::from_str::<serde_json::Value>(text)
        .map_err(|e| ChapError::Usage(format!("--data {} is not valid JSON: {e}", whence(spec))))?;
    Ok(())
}

/// How an error names the place a body came from.
fn whence(spec: &str) -> String {
    match source_of(spec) {
        Source::Stdin => "read from stdin".to_string(),
        Source::File(path) => format!("read from {path}"),
        Source::Inline(_) => "given inline".to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_three_spellings_of_data_are_told_apart() {
        assert_eq!(source_of("-"), Source::Stdin);
        assert_eq!(source_of("@body.json"), Source::File("body.json"));
        assert_eq!(source_of("@/tmp/body.json"), Source::File("/tmp/body.json"));
        assert_eq!(source_of(r#"{"a":1}"#), Source::Inline(r#"{"a":1}"#));
        // A body that happens to start with a dash is still inline: only the
        // bare `-` is stdin.
        assert_eq!(source_of("-1"), Source::Inline("-1"));
    }

    #[test]
    fn an_inline_body_is_read_and_checked() {
        assert_eq!(read_data(r#"{"a":1}"#).unwrap(), r#"{"a":1}"#);
        // Byte for byte: the spacing a caller chose is the spacing chap-core
        // receives.
        assert_eq!(read_data(r#"{ "a" : 1 }"#).unwrap(), r#"{ "a" : 1 }"#);
        // A bare JSON value is a body too.
        assert_eq!(read_data("[1,2]").unwrap(), "[1,2]");
        assert_eq!(read_data("\"text\"").unwrap(), "\"text\"");

        let err = read_data("{a:1}").expect_err("not JSON");
        assert!(err.to_string().contains("given inline"), "{err}");
        assert!(err.to_string().contains("not valid JSON"), "{err}");

        let err = read_data("   ").expect_err("nothing to send");
        assert!(err.to_string().contains("is empty"), "{err}");
    }

    #[test]
    fn a_file_body_is_read_from_the_path_after_the_at_sign() {
        let dir = tempfile::tempdir().expect("a temp dir");
        let path = dir.path().join("body.json");
        std::fs::write(&path, "{\n  \"name\": \"eval\"\n}\n").unwrap();
        let spec = format!("@{}", path.display());
        assert_eq!(read_data(&spec).unwrap(), "{\n  \"name\": \"eval\"\n}\n");

        let bad = dir.path().join("broken.json");
        std::fs::write(&bad, "{nope}").unwrap();
        let err = read_data(&format!("@{}", bad.display())).expect_err("not JSON");
        assert!(err.to_string().contains("broken.json"), "{err}");

        let err = read_data("@/nowhere/at/all.json").expect_err("no such file");
        assert!(err.to_string().contains("/nowhere/at/all.json"), "{err}");
    }

    #[test]
    fn a_body_that_is_not_json_is_a_usage_error() {
        let err = read_data("{a:1}").expect_err("not JSON");
        assert!(
            matches!(err.downcast_ref::<ChapError>(), Some(ChapError::Usage(_))),
            "{err}"
        );
    }

    /// The rendering rules, without a server: the same match `print_body`
    /// makes, asserted on the bodies it makes it for.
    #[test]
    fn a_json_string_reads_as_text_and_everything_else_as_json() {
        let string: serde_json::Value = serde_json::from_str(r#""line one\nline two\n""#).unwrap();
        assert_eq!(string.as_str().unwrap(), "line one\nline two\n");

        let object: serde_json::Value = serde_json::from_str(r#"{"id":"abc","n":1}"#).unwrap();
        assert_eq!(
            serde_json::to_string_pretty(&object).unwrap(),
            "{\n  \"id\": \"abc\",\n  \"n\": 1\n}"
        );
    }

    #[test]
    fn text_is_written_with_exactly_one_trailing_newline() {
        let mut out = Vec::new();
        write_text(&mut out, "one").unwrap();
        assert_eq!(String::from_utf8(out).unwrap(), "one\n");

        let mut out = Vec::new();
        write_text(&mut out, "one\n").unwrap();
        assert_eq!(String::from_utf8(out).unwrap(), "one\n");

        let mut out = Vec::new();
        write_text(&mut out, "").unwrap();
        assert!(out.is_empty(), "an empty body prints nothing at all");
    }

    #[test]
    fn a_401_says_which_half_of_the_token_is_missing() {
        let with = Api::new("http://x", Some("t".into()), Duration::from_secs(1));
        let without = Api::new("http://x", None, Duration::from_secs(1));
        let answer = |status| Answer {
            status,
            reason: "Unauthorized".to_string(),
            content_type: crate::api::JSON.to_string(),
            body: br#"{"detail":"Missing or invalid API token"}"#.to_vec(),
        };
        assert!(
            hint(&with, &answer(401))
                .unwrap()
                .contains("did not accept")
        );
        assert!(
            hint(&without, &answer(401))
                .unwrap()
                .contains("none was sent")
        );
        assert_eq!(hint(&with, &answer(404)), None);
    }
}
