//! The lines under the tables: the one each table adds up to, and the hints
//! that name what to run next.

use super::{ComponentState, ComponentStatus, ModelState, ModelStatus};

/// The closing lines of a deployment without chap-core: the models, when it
/// has any, then the components, when it has any. A line names a command only
/// when something is not up; the optional next commands are in
/// [`standalone_hints`].
pub fn standalone_closing_lines(
    models: &[ModelStatus],
    components: &[ComponentStatus],
) -> Vec<String> {
    let mut lines = Vec::new();
    if !models.is_empty() {
        let total = models.len();
        let noun = if total == 1 { "model" } else { "models" };
        let stopped = models
            .iter()
            .filter(|m| m.state == ModelState::NotRunning)
            .count();
        let silent = models
            .iter()
            .filter(|m| m.state == ModelState::RunningNotAnswering)
            .count();
        lines.push(if stopped == total {
            NOTHING_RUNNING.to_string()
        } else if stopped > 0 {
            let verb = if stopped == 1 { "is" } else { "are" };
            format!("{stopped} of {total} {noun} {verb} not running; start them with `varde up`")
        } else if let Some(first) = models
            .iter()
            .find(|m| m.state == ModelState::RunningNotAnswering)
        {
            let verb = if silent == 1 { "is" } else { "are" };
            format!(
                "{silent} of {total} {noun} {verb} running and not answering on /health; a model \
                 that just started answers in a minute, so run `varde status` again, or read \
                 `varde logs {}`",
                first.id
            )
        } else if total == 1 {
            "1 model up, answering on its own host port".to_string()
        } else {
            format!("all {total} models up, each answering on its own host port")
        });
    }
    if !components.is_empty() || models.is_empty() {
        let line = components_closing_line(components);
        if !lines.contains(&line) {
            lines.push(line);
        }
    }
    lines
}

/// What a deployment with nothing running at all is told, when chap-core is not
/// one of its components.
///
/// The mirror of `varde status`'s `Chap is not running`: there is no Chap here
/// to be running or not, only the components the deployment is made of, so the
/// line names the deployment rather than a product it does not contain.
pub const NOTHING_RUNNING: &str = "nothing in this deployment is running; start it with `varde up`";

/// The one line the component rows add up to, for a deployment chap-core is not
/// a component of.
///
/// [`closing_line`] cannot answer for one: it counts models, such a deployment
/// can have none, and the line it gives for none names `varde models enable`,
/// which is refused there. The components are the whole of the deployment, so
/// they are the whole of its verdict.
///
/// A component that is not running is what the reader has to do something
/// about, so it is what the line counts and `varde up` is what it names - the
/// same rows [`ComponentState::is_problem`] makes the exit code out of.
pub fn components_closing_line(rows: &[ComponentStatus]) -> String {
    let total = rows.len();
    if total == 0 {
        return EMPTY.to_string();
    }
    let down = rows
        .iter()
        .filter(|row| row.state == ComponentState::NotRunning)
        .count();
    if down == total {
        return NOTHING_RUNNING.to_string();
    }
    let noun = if total == 1 {
        "component"
    } else {
        "components"
    };
    if down > 0 {
        let verb = if down == 1 { "is" } else { "are" };
        let them = if down == 1 { "it" } else { "them" };
        return format!(
            "{down} of {total} {noun} {verb} not running; start {them} with `varde up`"
        );
    }
    let broken: Vec<&str> = rows
        .iter()
        .filter(|row| row.state == ComponentState::Unhealthy)
        .map(|row| row.name.as_str())
        .collect();
    if let Some(first) = broken.first() {
        let verb = if broken.len() == 1 { "is" } else { "are" };
        return format!(
            "{} of {total} {noun} {verb} unhealthy; `varde logs {first}` says why",
            broken.len()
        );
    }
    let starting = rows
        .iter()
        .filter(|row| row.state == ComponentState::Starting)
        .count();
    if starting > 0 {
        let verb = if starting == 1 { "is" } else { "are" };
        return format!(
            "{starting} of {total} {noun} {verb} still starting; \
             run `varde status` again in a moment"
        );
    }
    match rows {
        [only] => format!("{} is up", only.name),
        [_, _] => "both components are up".to_string(),
        _ => format!("all {total} {noun} are up"),
    }
}

/// The optional next commands under the closing lines of a deployment without
/// chap-core: a test when every model is up, and `varde open` when every
/// component is up.
pub fn standalone_hints(models: &[ModelStatus], components: &[ComponentStatus]) -> Vec<String> {
    let mut hints = Vec::new();
    if !models.is_empty() && models.iter().all(|m| m.state == ModelState::Up) {
        hints.push(match models.len() {
            1 => TEST_HINT_ONE.to_string(),
            _ => TEST_HINT.to_string(),
        });
    }
    if !components.is_empty() && components.iter().all(|c| c.state == ComponentState::Up) {
        // An instance with a page of its own is what a person opens next; the
        // object store has none.
        let openable: Vec<String> = components
            .iter()
            .map(|row| row.name.as_str())
            .filter(|name| *name != crate::compose::S3_SERVICE)
            .map(|name| format!("`varde open {name}`"))
            .collect();
        match openable.as_slice() {
            [] => {}
            [one] => hints.push(format!("{one} opens it")),
            many => hints.push(format!("{} open them", many.join(" and "))),
        }
    }
    hints
}

/// What a deployment with nothing in it at all is told, by `varde status` and
/// by `varde up`: there is nothing to start, and these are the ways to add
/// something.
pub const EMPTY: &str = "this deployment has no components and no models; add one with \
     `varde models add URL`, `varde models enable ID` or `varde components enable NAME`";
/// The one line the table adds up to.
///
/// Rows the project does not manage are left out of the count: an unmanaged
/// registration is not a model this deployment has to get running.
pub fn closing_line(rows: &[ModelStatus]) -> String {
    let mine: Vec<&ModelStatus> = rows
        .iter()
        .filter(|r| r.state != ModelState::Unmanaged)
        .collect();
    let total = mine.len();
    if total == 0 {
        // Registrations from outside - a model run from its checkout - are
        // models chap-core has, so "no models" would be the wrong answer.
        let strangers = rows.len();
        return match strangers {
            0 => "no models enabled; run `varde models enable ID` to add one".to_string(),
            1 => "no models enabled here; the unmanaged one above registered from outside this \
                  deployment"
                .to_string(),
            n => format!(
                "no models enabled here; the {n} unmanaged above registered from outside this \
                 deployment"
            ),
        };
    }
    let unreachable = mine
        .iter()
        .filter(|r| r.state == ModelState::Unreachable)
        .count();
    let problems = mine.iter().filter(|r| r.state.is_problem()).count() - unreachable;
    let noun = if total == 1 { "model" } else { "models" };
    let verb = |n: usize| if n == 1 { "is" } else { "are" };
    match (problems, unreachable) {
        (0, 0) => {
            // "all" is for more than one; a lone model is simply registered.
            let all = if total == 1 { "" } else { "all " };
            format!("{all}{total} {noun} registered")
        }
        (0, u) => format!(
            "{u} of {total} {noun} {} registered and unreachable from chap-core.",
            verb(u)
        ),
        (p, 0) => format!("{p} of {total} {noun} {} not registered.", verb(p)),
        (p, u) => format!(
            "{p} of {total} {noun} {} not registered, and {u} {} unreachable from chap-core.",
            verb(p),
            verb(u)
        ),
    }
}

/// One line per row that needs a fix, in table order.
///
/// A model whose container is up but which chap-core does not know about is
/// not a crash to read the logs for: chapkit tries to register five times
/// while it starts and then gives up for good, so a model that came up before
/// chap-core was healthy stays invisible until it is restarted.
///
/// `--all` because nothing about the service has changed: it is running the
/// image and the configuration it should be, and the restart is only there to
/// make it introduce itself again. A plain `varde restart` would recreate
/// what moved, which here is nothing.
///
/// `auth` adds the other reason a model never appears on a protected
/// deployment: chap-core rejects a registration that carries no key, and a
/// chap-core created before `compose.varde.yml` passed the key through never
/// had one to check against.
/// The hint under a clean status: registration is a heartbeat, and the only
/// way to know a model can work is to make it work.
pub const TEST_HINT: &str = "run `varde models test --all` to check they can run";

/// [`TEST_HINT`] for a deployment with one model.
pub const TEST_HINT_ONE: &str = "run `varde models test --all` to check it can run";

/// The extra hint for a model that has not registered with a chap-core
/// elsewhere. Its log says which of the two causes it is: it cannot reach
/// chap-core, or servicekit found no app on the port it checks before it
/// registers, which is a port varde read wrong off the image's command.
pub fn external_registration_hints(rows: &[ModelStatus]) -> Vec<String> {
    rows.iter()
        .filter(|row| row.state == ModelState::RunningNotRegistered && !row.young)
        .map(|row| {
            format!(
                "{id}: `varde logs {id}` says why it does not register; then enable it again \
                 with a network (`varde models enable {id}`)",
                id = row.id
            )
        })
        .collect()
}

/// The background to [`external_registration_hints`]: what the two log lines
/// mean.
pub const EXTERNAL_REGISTRATION_LOG: &str = "in the log of a model, \
     `registration.attempt_failed` means it cannot reach chap-core, and `App never became \
     ready` means it does not listen on the port varde read off its image";

/// `elsewhere` is the URL of a chap-core this deployment does not run, which
/// changes what an unreachable model most likely means.
pub fn hints(rows: &[ModelStatus], auth: bool, elsewhere: Option<&str>) -> Vec<String> {
    let registration_key = if auth {
        concat!(
            "; if its log shows 401, chap-core is missing the registration key: ",
            "run `varde sync`, then `varde restart`"
        )
    } else {
        ""
    };
    let mine: Vec<&ModelStatus> = rows
        .iter()
        .filter(|row| row.state != ModelState::Unmanaged)
        .collect();
    mine.iter()
        .filter_map(|row| match row.state {
            ModelState::RunningNotRegistered if row.registered_as.is_some() => {
                let actual = row.registered_as.as_deref().unwrap_or_default();
                Some(match &row.added_from {
                    Some((model, source)) => format!(
                        "{}: its container registered as `{actual}`, the unmanaged row above; \
                         run `varde models remove {model}`, then `varde models add {source} \
                         --service-id {actual}`",
                        row.id
                    ),
                    None => format!(
                        "{}: its container registered as `{actual}`, the unmanaged row above; \
                         `varde models remove` the model and add it again with `--service-id \
                         {actual}`",
                        row.id
                    ),
                })
            }
            // Started moments ago: registering is part of starting, and the
            // restart below would only start the wait over.
            ModelState::RunningNotRegistered if row.young => Some(format!(
                "{}: started under two minutes ago and registers once it is ready; run `varde \
                 status` again in a minute",
                row.id
            )),
            ModelState::RunningNotRegistered => Some(format!(
                "{}: restart it with `varde restart --all {}`{registration_key}",
                row.id, row.id
            )),
            ModelState::NotRunning => Some(format!(
                "{}: start Chap with `varde up`, then `varde logs {}`",
                row.id, row.id
            )),
            ModelState::RunningNotAnswering => Some(format!(
                "{}: read `varde logs {}`; a model still starting answers in a moment",
                row.id, row.id
            )),
            ModelState::Unreachable => Some(unreachable_hint(row, elsewhere)),
            ModelState::Registered | ModelState::Unmanaged | ModelState::Up => None,
        })
        .collect()
}

/// The optional next command when no row needs a fix: every model answered
/// its heartbeat, which is as far as `varde status` can see.
pub fn test_hint(rows: &[ModelStatus]) -> Option<&'static str> {
    let mine: Vec<&ModelStatus> = rows
        .iter()
        .filter(|row| row.state != ModelState::Unmanaged)
        .collect();
    let clean = mine
        .iter()
        .all(|row| matches!(row.state, ModelState::Registered | ModelState::Up));
    match (mine.len(), clean) {
        (0, _) | (_, false) => None,
        (1, true) => Some(TEST_HINT_ONE),
        (_, true) => Some(TEST_HINT),
    }
}

/// The hint for a model chap-core has registered and cannot reach.
///
/// With a chap-core elsewhere the cause is nearly always the address: models
/// register as `localhost:<port>` by default, which is this machine for a
/// chap-core running as a process here and the chap-core container itself for
/// one running in Docker. With varde' own chap-core both sit on the compose
/// network, so the model's log is where the answer is.
fn unreachable_hint(row: &ModelStatus, elsewhere: Option<&str>) -> String {
    let (url, answer) = row
        .unreachable
        .as_ref()
        .map(|u| (u.registered_url.as_str(), u.answer.as_str()))
        .unwrap_or_default();
    match elsewhere {
        Some(api) => format!(
            "{id}: chap-core cannot reach it at {url} ({answer}); if your chap-core runs in a \
             container, run `varde components enable chap-core --url {api} --models-host \
             host.docker.internal`, then `varde up`",
            id = row.id
        ),
        None => format!(
            "{id}: chap-core cannot reach it at {url} ({answer}); read `varde logs {id}` and \
             `varde logs chap`",
            id = row.id
        ),
    }
}
