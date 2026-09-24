//! `chaps auth` — the API token and the service registration key.
//!
//! Four verbs over the same two `.env` lines: `show` reads them, `enable`
//! writes them, `disable` comments them out and `rotate` replaces them. The
//! values never leave `.env`; `.chaps/project.yaml` only records which of them
//! are in use, and the model overlays are re-rendered from that so each
//! service sends the registration key when there is one.
//!
//! Nothing here restarts anything. chap-core and the model containers read
//! `.env` when compose creates them, so every command that changes a secret
//! ends by saying that `chaps up` has to follow.

use crate::auth::{
    self, API_TOKEN_ENV_VAR, MODELING_APP_HINT, REGISTRATION_KEY_ENV_VAR, mask, write_secrets,
};
use crate::cli::{AuthDisableArgs, AuthEnableArgs, AuthRotateArgs, AuthShowArgs};
use crate::commands::Ctx;
use crate::compose::sync::sync;
use crate::error::Result;
use crate::output::{self, Out};
use crate::project::{AuthState, ENV_FILE, Project};
use std::path::{Path, PathBuf};

/// What every `chaps up` reminder says, because neither chap-core nor a model
/// re-reads `.env` while its container exists.
const RESTART_HINT: &str = "run `chaps up` to restart chap-core and the models with authentication";

/// `chaps auth show`: what is protected, and by which token.
pub fn show(ctx: &Ctx, args: &AuthShowArgs) -> Result<()> {
    let project = ctx.project()?;
    let body = read_env(&project)?;
    let recorded = project.state.auth;
    let effective = auth::state_of(&body);
    let token = auth::active_value(&body, API_TOKEN_ENV_VAR);

    let value = serde_json::json!({
        "api_token": effective.api_token,
        "registration_key": effective.registration_key,
        "recorded": recorded,
        // The masked form is always safe to print; the secret itself only
        // appears when --reveal asked for it.
        "token_masked": token.as_deref().map(mask),
        "token": args.reveal.then(|| token.clone()).flatten(),
        "env_file": project.dir.join(ENV_FILE),
    });
    ctx.out.emit(&value, || {
        show_human(
            &ctx.out,
            &effective,
            recorded,
            token.as_deref(),
            args.reveal,
        )
    })
}

/// `chaps auth enable`: put both secrets in `.env` and render the overlays.
pub fn enable(ctx: &Ctx, args: &AuthEnableArgs) -> Result<()> {
    let mut project = ctx.project()?;
    let body = read_env(&project)?;
    let effective = auth::state_of(&body);

    // Already on: rotating is the operation that replaces a working secret,
    // and doing it by accident locks every configured client out.
    if effective.api_token && effective.registration_key {
        let token = auth::active_value(&body, API_TOKEN_ENV_VAR);
        record(&mut project, effective)?;
        let value = serde_json::json!({
            "changed": false,
            "api_token": true,
            "registration_key": true,
            "token_masked": token.as_deref().map(mask),
        });
        return ctx.out.emit(&value, || {
            format!(
                "API authentication is already on ({}); \
                 `chaps auth rotate` replaces both secrets, \
                 `chaps auth show --reveal` prints the token\n",
                token.as_deref().map(mask).unwrap_or_default()
            )
        });
    }

    // Half on: keep whichever secret is already in use, so clients that
    // already hold the token keep working.
    let token = match args.token.as_deref().map(str::trim) {
        Some(given) if !given.is_empty() => {
            if given.chars().count() < auth::MIN_TOKEN_LENGTH {
                output::warn(&auth::weak_token_warning(given.chars().count()));
            }
            given.to_string()
        }
        Some(_) => {
            output::warn("--token was given an empty value; generated one instead");
            auth::random_secret()?
        }
        None => match recover(&body, API_TOKEN_ENV_VAR) {
            Some(known) => known,
            None => auth::random_secret()?,
        },
    };
    let key = match recover(&body, REGISTRATION_KEY_ENV_VAR) {
        Some(known) => known,
        None => auth::random_secret()?,
    };
    apply(ctx, &mut project, &body, &token, &key, Change::Enabled)
}

/// `chaps auth disable`: comment both secrets out and render the overlays.
pub fn disable(ctx: &Ctx, _args: &AuthDisableArgs) -> Result<()> {
    let mut project = ctx.project()?;
    let body = read_env(&project)?;
    let effective = auth::state_of(&body);

    if !effective.is_on() {
        record(&mut project, effective)?;
        let value =
            serde_json::json!({ "changed": false, "api_token": false, "registration_key": false });
        return ctx.out.emit(&value, || {
            "API authentication is already off; `chaps auth enable` turns it on\n".to_string()
        });
    }

    let mut out = auth::comment_out(&body, API_TOKEN_ENV_VAR);
    out = auth::comment_out(&out, REGISTRATION_KEY_ENV_VAR);
    write_env(&project, &out)?;
    project.state.auth = AuthState::default();
    let report = render(ctx, &mut project)?;

    let value = serde_json::json!({
        "changed": true,
        "api_token": false,
        "registration_key": false,
        "written": report.written,
    });
    ctx.out.emit(&value, || {
        let mut text = String::from("API authentication is off\n");
        text.push_str(&format!(
            "  both values are kept as comments in {ENV_FILE}, so `chaps auth enable` \
             recovers them\n"
        ));
        text.push_str(&written_block(&project.dir, &report.written));
        text.push_str(
            "\nrun `chaps up` to restart chap-core and the models without \
                       authentication\n",
        );
        text
    })
}

/// `chaps auth rotate`: replace both secrets with new ones.
pub fn rotate(ctx: &Ctx, _args: &AuthRotateArgs) -> Result<()> {
    let mut project = ctx.project()?;
    let body = read_env(&project)?;
    let token = auth::random_secret()?;
    let key = auth::random_secret()?;
    apply(ctx, &mut project, &body, &token, &key, Change::Rotated)
}

/// Which verb wrote the secrets, so the closing lines can say the right thing.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Change {
    Enabled,
    Rotated,
}

/// Write both secrets into `.env`, record them and re-render the overlays.
fn apply(
    ctx: &Ctx,
    project: &mut Project,
    body: &str,
    token: &str,
    key: &str,
    change: Change,
) -> Result<()> {
    let out = write_secrets(
        body,
        &[(API_TOKEN_ENV_VAR, token), (REGISTRATION_KEY_ENV_VAR, key)],
    );
    write_env(project, &out)?;
    project.state.auth = AuthState {
        api_token: true,
        registration_key: true,
    };
    let report = render(ctx, project)?;

    let value = serde_json::json!({
        "changed": true,
        "api_token": true,
        "registration_key": true,
        "token_masked": mask(token),
        "written": report.written,
    });
    let dir = project.dir.clone();
    ctx.out.emit(&value, || {
        let mut text = match change {
            Change::Enabled => String::from("API authentication is on\n"),
            Change::Rotated => String::from("API authentication rotated\n"),
        };
        text.push_str(&format!(
            "  API token         {} (`chaps auth show --reveal` prints it)\n",
            mask(token)
        ));
        text.push_str("  Registration key  written to .env; every model overlay now sends it\n");
        text.push_str(&written_block(&dir, &report.written));
        text.push('\n');
        text.push_str(&format!("{RESTART_HINT}\n"));
        match change {
            Change::Enabled => text.push_str(&format!(
                "{MODELING_APP_HINT} (`chaps auth show --reveal`)\n"
            )),
            Change::Rotated => text.push_str(
                "every client keeps sending the old token until it is updated: \
                 the DHIS2 Modeling App's CHAP settings, and anything else \
                 calling this API\n",
            ),
        }
        text
    })
}

/// A secret `.env` already knows: one that is in use, or one
/// `chaps auth disable` left behind as a comment.
///
/// Reusing it is what makes `disable` followed by `enable` a round trip: a
/// deployment that was turned off by mistake comes back with the token its
/// clients still hold, rather than one nobody has been told about.
fn recover(body: &str, var: &str) -> Option<String> {
    auth::active_value(body, var).or_else(|| auth::commented_value(body, var))
}

/// Re-render the compose files from the new state; `sync` saves `.chaps/`.
fn render(ctx: &Ctx, project: &mut Project) -> Result<crate::compose::SyncReport> {
    let registry = super::registry_for(ctx, Some(project))?;
    let report = sync(project, &registry, ctx.cli_version, false)?;
    for warning in &report.warnings {
        output::warn(warning);
    }
    Ok(report)
}

/// Save `.chaps/project.yaml` with the state `.env` actually describes.
///
/// The no-change paths still do this: a project whose recorded booleans drifted
/// from its `.env` - a hand-edited file, a restore from an older archive -
/// should not need a second command to line up again.
fn record(project: &mut Project, effective: AuthState) -> Result<()> {
    if project.state.auth == effective {
        return Ok(());
    }
    project.state.auth = effective;
    project.save()
}

/// The `.env` body, or the error that says why there is none to read.
fn read_env(project: &Project) -> Result<String> {
    let path = project.dir.join(ENV_FILE);
    match std::fs::read_to_string(&path) {
        Ok(body) => Ok(body),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Err(anyhow::anyhow!(
            "{} has no {ENV_FILE} (a project created with `init --no-env`); the API token has \
             to live in a {ENV_FILE} next to the compose files, because that is the file \
             compose reads",
            project.dir.display()
        )),
        Err(e) => Err(anyhow::Error::new(e).context(format!("reading {}", path.display()))),
    }
}

fn write_env(project: &Project, body: &str) -> Result<()> {
    let path = project.dir.join(ENV_FILE);
    std::fs::write(&path, body).map_err(|e| anyhow::anyhow!("writing {}: {e}", path.display()))
}

/// The `written  <file>` block, empty when the render changed nothing.
fn written_block(dir: &Path, written: &[PathBuf]) -> String {
    if written.is_empty() {
        return String::new();
    }
    let mut text = String::from("\n");
    for path in written {
        let label = path.strip_prefix(dir).unwrap_or(path).display();
        text.push_str(&format!("written  {label}\n"));
    }
    text
}

/// The human rendering of `chaps auth show`.
fn show_human(
    out: &Out,
    effective: &AuthState,
    recorded: AuthState,
    token: Option<&str>,
    reveal: bool,
) -> String {
    // On is the safe state and off is the one to act on, so off is the colour
    // that asks for attention rather than the one that says "broken".
    let on_off = |on: bool| {
        if on { out.ok("on") } else { out.warn("off") }
    };
    let mut rows = vec![
        ("API authentication", on_off(effective.api_token)),
        ("Registration key", on_off(effective.registration_key)),
    ];
    if let Some(token) = token {
        let shown = if reveal {
            out.value(token)
        } else {
            format!(
                "{} {}",
                out.value(&mask(token)),
                out.dim("(--reveal prints it in full)")
            )
        };
        rows.push(("API token", shown));
    }
    let mut text = output::fields_with(0, &rows, &|label| out.key(label));

    if !effective.is_on() {
        text.push('\n');
        text.push_str(&out.backticks(
            "nothing protects this API: anyone who can reach the port can use it. \
             `chaps auth enable` turns authentication on.",
        ));
        text.push('\n');
        return text;
    }
    if effective.api_token {
        text.push_str(&format!(
            "\n{}\n",
            out.backticks(&format!(
                "clients send it as `Authorization: Bearer <token>`; {MODELING_APP_HINT}."
            ))
        ));
    }
    if effective.registration_key {
        text.push_str(&out.backticks(
            "model services send the registration key as `X-Service-Key` when they register.",
        ));
        text.push('\n');
    }
    // `.env` is what the deployment does; the booleans in `.chaps/` are only a
    // record of it, and a mismatch means one of them was edited by hand.
    if recorded != *effective {
        text.push_str(&format!(
            "\n{} {}\n",
            out.warn("warning:"),
            out.backticks(&format!(
                ".chaps/project.yaml records api_token: {}, registration_key: {}, \
                 which is not what {ENV_FILE} sets; `chaps auth enable` or `chaps auth disable` \
                 lines them up again",
                recorded.api_token, recorded.registration_key
            ))
        ));
    }
    text
}

#[cfg(test)]
mod tests {
    use super::*;

    fn on() -> AuthState {
        AuthState {
            api_token: true,
            registration_key: true,
        }
    }

    #[test]
    fn show_says_off_and_what_to_do_about_it() {
        let text = show_human(
            &Out::default(),
            &AuthState::default(),
            AuthState::default(),
            None,
            false,
        );
        assert!(text.contains("API authentication  off"), "{text}");
        assert!(text.contains("Registration key    off"), "{text}");
        assert!(text.contains("nothing protects this API"), "{text}");
        assert!(text.contains("chaps auth enable"), "{text}");
        assert!(!text.contains("API token"), "there is none: {text}");
    }

    #[test]
    fn show_masks_the_token_until_reveal_asks_for_it() {
        let token = "0123456789abcdef0123456789abcdef";
        let masked = show_human(&Out::default(), &on(), on(), Some(token), false);
        assert!(masked.contains("API token           012345..."), "{masked}");
        assert!(!masked.contains(token), "the secret leaked: {masked}");
        assert!(masked.contains("--reveal prints it in full"), "{masked}");
        assert!(masked.contains(MODELING_APP_HINT), "{masked}");
        assert!(masked.contains("X-Service-Key"), "{masked}");

        let revealed = show_human(&Out::default(), &on(), on(), Some(token), true);
        assert!(
            revealed.contains(&format!("API token           {token}")),
            "{revealed}"
        );
        assert!(!revealed.contains("--reveal"), "{revealed}");
    }

    #[test]
    fn show_flags_a_state_file_that_disagrees_with_the_env() {
        // `.env` has both, `.chaps/` remembers neither: a hand edit, or a
        // restore from an archive written before `auth` existed.
        let text = show_human(
            &Out::default(),
            &on(),
            AuthState::default(),
            Some("sekret-token-value"),
            false,
        );
        assert!(
            text.contains("warning: .chaps/project.yaml records"),
            "{text}"
        );
        assert!(text.contains("api_token: false"), "{text}");

        // Agreement says nothing at all.
        assert!(
            !show_human(
                &Out::default(),
                &on(),
                on(),
                Some("sekret-token-value"),
                false
            )
            .contains("warning:")
        );
    }

    #[test]
    fn show_reports_one_secret_on_its_own() {
        let key_only = AuthState {
            api_token: false,
            registration_key: true,
        };
        let text = show_human(&Out::default(), &key_only, key_only, None, false);
        assert!(text.contains("API authentication  off"), "{text}");
        assert!(text.contains("Registration key    on"), "{text}");
        assert!(text.contains("X-Service-Key"), "{text}");
        assert!(!text.contains("nothing protects"), "{text}");
    }

    #[test]
    fn the_written_block_is_empty_when_nothing_changed() {
        let dir = Path::new("/p");
        assert_eq!(written_block(dir, &[]), "");
        assert_eq!(
            written_block(dir, &[PathBuf::from("/p/compose.x.yml")]),
            "\nwritten  compose.x.yml\n"
        );
    }

    #[test]
    fn the_restart_hint_names_the_command_that_applies_it() {
        assert!(RESTART_HINT.contains("chaps up"));
    }
}
