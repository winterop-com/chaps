//! `varde auth` — the API token and the service registration key.
//!
//! Four verbs over the same two `.env` lines: `show` reads them, `enable`
//! writes them, `disable` comments them out and `rotate` replaces them. The
//! values never leave `.env`; `.varde/project.yaml` only records which of them
//! are in use, and the model overlays are re-rendered from that so each
//! service sends the registration key when there is one.
//!
//! Nothing here restarts anything. chap-core and the model containers read
//! `.env` when compose creates them, so every command that changes a secret
//! ends by saying that `varde up` has to follow.

use crate::auth::{
    self, API_TOKEN_ENV_VAR, MODELING_APP_HINT, REGISTRATION_KEY_ENV_VAR, write_secrets,
};
use crate::cli::{AuthDisableArgs, AuthEnableArgs, AuthRotateArgs, AuthShowArgs};
use crate::commands::Ctx;
use crate::compose::sync::sync;
use crate::error::Result;
use crate::output::{self, Out, Report};
use crate::project::{AuthState, ENV_FILE, Project};
use std::path::Path;

/// What every `varde up` reminder says, because neither chap-core nor a model
/// re-reads `.env` while its container exists.
const RESTART_HINT: &str = "run `varde up` to restart chap-core and the models with authentication";

/// `varde auth show`: what is protected, and by which token.
pub fn show(ctx: &Ctx, args: &AuthShowArgs) -> Result<()> {
    let project = ctx.project()?;
    let body = read_env(&project)?;
    let recorded = project.state.auth;
    let effective = auth::state_of(&body);
    let token = auth::active_value(&body, API_TOKEN_ENV_VAR);
    let data_sources = data_sources(&project, &body);

    let value = serde_json::json!({
        "api_token": effective.api_token,
        "registration_key": effective.registration_key,
        "recorded": recorded,
        // The secret only appears when --reveal asked for it; `api_token`
        // above says whether there is one.
        "token": args.reveal.then(|| token.clone()).flatten(),
        "env_file": project.dir.join(ENV_FILE),
        // One entry per OCS data source variable, set or not and never the
        // value: these are third-party credentials, so `--reveal` does not
        // reach them. Absent
        // on a deployment without the ocs component, which has no data sources.
        "ocs_data_sources": data_sources
            .iter()
            .map(|(var, value)| {
                serde_json::json!({
                    "variable": var,
                    "set": value.is_some(),
                })
            })
            .collect::<Vec<_>>(),
    });
    if !ctx.out.json {
        print!(
            "{}",
            show_tables(
                &ctx.out,
                &effective,
                token.as_deref(),
                args.reveal,
                &data_sources
            )
        );
    }
    ctx.out.report(&value, |lines| {
        show_summary(&effective, recorded, &data_sources, lines)
    })
}

/// What `varde auth token` says when there is no token to print.
///
/// On stderr, so stdout holds the token and nothing else: `TOKEN=$(varde auth
/// token)` has to come back empty rather than with a sentence in it.
const NO_TOKEN: &str =
    "API authentication is off in this deployment; run `varde auth enable` to turn it on";

/// `varde auth token`: the token alone, for a script to capture.
///
/// `varde auth show --reveal` prints the token inside a report meant to be
/// read; this prints the value and nothing else, which is the difference
/// between a command a person runs and one a shell substitutes. A deployment
/// with authentication off has no token, so stdout stays empty and the exit
/// code is 1: a script that captured an empty string should stop, not carry on
/// sending an empty header.
pub fn token(ctx: &Ctx, _args: &crate::cli::AuthTokenArgs) -> Result<()> {
    let project = ctx.project()?;
    // Best-effort, like every other read of `.env`: a project written with
    // `init --no-env` has no file, which is the same answer as a deployment
    // with the line commented out.
    let token = auth::token_in(&project.dir);
    let value = serde_json::json!({ "token": token });
    ctx.out.emit(&value, || token.clone().unwrap_or_default())?;
    if token.is_none() {
        eprintln!("{NO_TOKEN}");
        std::process::exit(1);
    }
    Ok(())
}

/// Which of the OCS data source variables `.env` sets, and their values.
///
/// Empty for a deployment without the `ocs` component: the variables mean
/// nothing there, and a block of five `unset` rows would only suggest that
/// something is missing.
fn data_sources(project: &Project, body: &str) -> Vec<(&'static str, Option<String>)> {
    if !project.state.components.ocs.enabled {
        return Vec::new();
    }
    crate::components::OCS_DATA_SOURCE_ENV_VARS
        .iter()
        .map(|var| (*var, crate::dotenv::non_empty(body, var)))
        .collect()
}

/// `varde auth enable`: put both secrets in `.env` and render the overlays.
pub fn enable(ctx: &Ctx, args: &AuthEnableArgs) -> Result<()> {
    let (mut project, _lock) = ctx.project_mut()?;
    let body = read_env(&project)?;
    let effective = auth::state_of(&body);

    // Already on: rotating is the operation that replaces a working secret,
    // and doing it by accident locks every configured client out.
    if effective.api_token && effective.registration_key {
        record(&mut project, effective)?;
        let value = serde_json::json!({
            "changed": false,
            "api_token": true,
            "registration_key": true,
        });
        return ctx.out.report(&value, |lines| {
            lines
                .info("API authentication is already on")
                .hint("`varde auth rotate` replaces both secrets")
                .hint("`varde auth show --reveal` prints the token");
        });
    }

    // Half on: keep whichever secret is already in use, so clients that
    // already hold the token keep working.
    let mut warnings = Vec::new();
    let token = match args.token.as_deref().map(str::trim) {
        Some(given) if !given.is_empty() => {
            if let Some(problem) = crate::dotenv::literal_problem(given) {
                return Err(crate::error::ChapError::Usage(format!(
                    "--token contains {problem}; choose another, or leave --token out and one is \
                     generated"
                ))
                .into());
            }
            if given.chars().count() < auth::MIN_TOKEN_LENGTH {
                warnings.push(auth::weak_token_warning(given.chars().count()));
            }
            given.to_string()
        }
        Some(_) => {
            warnings.push("--token was given an empty value; generated one instead".to_string());
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
    apply(
        ctx,
        &mut project,
        &body,
        &token,
        &key,
        Change::Enabled,
        warnings,
    )
}

/// `varde auth disable`: comment both secrets out and render the overlays.
pub fn disable(ctx: &Ctx, _args: &AuthDisableArgs) -> Result<()> {
    let (mut project, _lock) = ctx.project_mut()?;
    let body = read_env(&project)?;
    let effective = auth::state_of(&body);

    if !effective.is_on() {
        record(&mut project, effective)?;
        let value =
            serde_json::json!({ "changed": false, "api_token": false, "registration_key": false });
        return ctx.out.report(&value, |lines| {
            lines
                .info("API authentication is already off")
                .hint("`varde auth enable` turns it on");
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
    ctx.out.report(&value, |lines| {
        lines.info("API authentication is off").hint(format!(
            "both values are kept as comments in `{ENV_FILE}`, so `varde auth enable` \
             recovers them"
        ));
        written_lines(&project.dir, &report, lines);
        lines.info("run `varde up` to restart chap-core and the models without authentication");
    })
}

/// `varde auth rotate`: replace both secrets with new ones.
pub fn rotate(ctx: &Ctx, _args: &AuthRotateArgs) -> Result<()> {
    let (mut project, _lock) = ctx.project_mut()?;
    let body = read_env(&project)?;
    let token = auth::random_secret()?;
    let key = auth::random_secret()?;
    apply(
        ctx,
        &mut project,
        &body,
        &token,
        &key,
        Change::Rotated,
        Vec::new(),
    )
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
    warnings: Vec<String>,
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
        "written": report.written,
    });
    let dir = project.dir.clone();
    ctx.out.report(&value, |lines| {
        lines.info(match change {
            Change::Enabled => "API authentication is on",
            Change::Rotated => "replaced the API token and the registration key",
        });
        for warning in &warnings {
            lines.warning(warning.as_str());
        }
        lines
            .hint(format!(
                "wrote the API token to `{ENV_FILE}`; `varde auth show --reveal` prints it"
            ))
            .hint(format!(
                "wrote the registration key to `{ENV_FILE}`; every model overlay now sends it"
            ));
        written_lines(&dir, &report, lines);
        lines.info(RESTART_HINT);
        match change {
            Change::Enabled => lines.hint(format!(
                "{MODELING_APP_HINT}; any other client needs it from `varde auth show --reveal`"
            )),
            Change::Rotated => lines.info(
                "every client keeps sending the old token until it is updated: run \
                 `varde dhis2 connect` after `varde up` for the DHIS2 route, and update \
                 anything else calling this API",
            ),
        };
    })
}

/// A secret `.env` already knows: one that is in use, or one
/// `varde auth disable` left behind as a comment.
///
/// Reusing it is what makes `disable` followed by `enable` a round trip: a
/// deployment that was turned off by mistake comes back with the token its
/// clients still hold, rather than one nobody has been told about.
fn recover(body: &str, var: &str) -> Option<String> {
    auth::active_value(body, var).or_else(|| auth::commented_value(body, var))
}

/// Re-render the compose files from the new state; `sync` saves `.varde/`.
fn render(ctx: &Ctx, project: &mut Project) -> Result<crate::compose::SyncReport> {
    let registry = super::registry_for(ctx, Some(project))?;
    sync(project, &registry, false)
}

/// Save `.varde/project.yaml` with the state `.env` actually describes.
///
/// The no-change paths still do this: a project whose recorded booleans drifted
/// from its `.env` - a hand-edited file, a restore -
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
    crate::dotenv::write(&path, body)
}

/// The files the render wrote, as hints, and the warnings it gave.
fn written_lines(dir: &Path, report: &crate::compose::SyncReport, lines: &mut Report) {
    for warning in &report.warnings {
        lines.warning(warning.as_str());
    }
    for path in &report.written {
        let label = path.strip_prefix(dir).unwrap_or(path).display();
        lines.hint(format!("wrote {label}"));
    }
}

/// The tables of `varde auth show`: the two switches and the token, then the
/// OCS data sources.
fn show_tables(
    out: &Out,
    effective: &AuthState,
    token: Option<&str>,
    reveal: bool,
    data_sources: &[(&str, Option<String>)],
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
                out.value("set in .env"),
                out.dim("(--reveal prints it)")
            )
        };
        rows.push(("API token", shown));
    }
    let mut text = output::fields_with(0, &rows, &|label| out.key(label));
    text.push_str(&data_sources_block(out, data_sources));
    text
}

/// The lines after the tables of `varde auth show`.
fn show_summary(
    effective: &AuthState,
    recorded: AuthState,
    data_sources: &[(&str, Option<String>)],
    lines: &mut Report,
) {
    if !effective.is_on() {
        lines
            .info("nothing protects this API: anyone who can reach the port can use it")
            .hint("`varde auth enable` turns authentication on");
    }
    if effective.api_token {
        lines.hint(format!(
            "clients send the token as `Authorization: Bearer <token>`; {MODELING_APP_HINT}"
        ));
    }
    if effective.registration_key {
        lines
            .hint("model services send the registration key as `X-Service-Key` when they register");
    }
    // `.env` is what the deployment does; the booleans in `.varde/` are only a
    // record of it, and a mismatch means one of them was edited by hand.
    if effective.is_on() && recorded != *effective {
        lines.warning(format!(
            "`.varde/project.yaml` records api_token: {}, registration_key: {}, which is not \
             what `{ENV_FILE}` sets; `varde auth enable` or `varde auth disable` lines them up \
             again",
            recorded.api_token, recorded.registration_key
        ));
    }
    if !data_sources.is_empty() {
        lines.hint(
            "ERA5-Land needs one or both of ECMWF_DATASTORES_* and EDH_API_KEY, per dataset; \
             WorldPop and CHIRPS3 need none; set them in `.env` and run `varde up`",
        );
    }
}

/// The `OCS data sources` block: one row per credential variable, saying
/// whether `.env` sets it.
///
/// These are not this deployment's secrets - they are accounts with Copernicus
/// and Earth Data Hub - so only whether it is set is shown, whatever `--reveal` asked for:
/// there is nothing to paste into a client here, only the question of whether
/// a dataset will ingest. Empty for a deployment without the `ocs` component.
fn data_sources_block(out: &Out, data_sources: &[(&str, Option<String>)]) -> String {
    if data_sources.is_empty() {
        return String::new();
    }
    let rows: Vec<(&str, String)> = data_sources
        .iter()
        .map(|(var, value)| {
            (
                *var,
                match value {
                    Some(_) => out.ok("set"),
                    None => out.dim("unset"),
                },
            )
        })
        .collect();
    let mut text = format!("\n{}\n", out.heading("OCS data sources"));
    text.push_str(&output::fields_with(2, &rows, &|label| out.key(label)));
    text
}

#[cfg(test)]
mod tests;
