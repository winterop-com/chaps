//! The lines under the tables: the one each table adds up to, and the hints
//! that name what to run next.

use super::{ComponentState, ComponentStatus, ModelState, ModelStatus};

/// The closing lines of a deployment without chap-core: the models, when it
/// has any, then the components, when it has any. Each names what to run.
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
            format!("{stopped} of {total} {noun} {verb} not running; start them with `chaps up`")
        } else if let Some(first) = models
            .iter()
            .find(|m| m.state == ModelState::RunningNotAnswering)
        {
            let verb = if silent == 1 { "is" } else { "are" };
            format!(
                "{silent} of {total} {noun} {verb} running and not answering on /health; a model \
                 that just started answers in a minute, so run `chaps status` again, or read \
                 `chaps logs {}`",
                first.id
            )
        } else if total == 1 {
            "1 model up, answering on its own host port; `chaps models test --all` checks it \
             can run"
                .to_string()
        } else {
            format!(
                "all {total} models up, each answering on its own host port; `chaps models test \
                 --all` checks they can run"
            )
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
/// The mirror of `chaps status`'s `Chap is not running`: there is no Chap here
/// to be running or not, only the components the deployment is made of, so the
/// line names the deployment rather than a product it does not contain.
pub const NOTHING_RUNNING: &str = "nothing in this deployment is running; start it with `chaps up`";

/// The one line the component rows add up to, for a deployment chap-core is not
/// a component of.
///
/// [`closing_line`] cannot answer for one: it counts models, such a deployment
/// can have none, and the line it gives for none names `chaps models enable`,
/// which is refused there. The components are the whole of the deployment, so
/// they are the whole of its verdict.
///
/// A component that is not running is what the reader has to do something
/// about, so it is what the line counts and `chaps up` is what it names - the
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
            "{down} of {total} {noun} {verb} not running; start {them} with `chaps up`"
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
             run `chaps status` again in a moment"
        );
    }
    // An instance with a page of its own is what a person opens next; the
    // object store has none.
    let openable: Vec<&str> = rows
        .iter()
        .map(|row| row.name.as_str())
        .filter(|name| *name != crate::compose::S3_SERVICE)
        .collect();
    let opens = match openable.as_slice() {
        [] => String::new(),
        [one] => format!("; `chaps open {one}` opens it"),
        many => format!(
            "; {} open them",
            many.iter()
                .map(|name| format!("`chaps open {name}`"))
                .collect::<Vec<_>>()
                .join(" and ")
        ),
    };
    match rows {
        [only] => format!("{} is up{opens}", only.name),
        [_, _] => format!("both components are up{opens}"),
        _ => format!("all {total} {noun} are up{opens}"),
    }
}

/// What a deployment with nothing in it at all is told, by `chaps status` and
/// by `chaps up`: there is nothing to start, and these are the ways to add
/// something.
pub const EMPTY: &str = "this deployment has no components and no models; add one with \
     `chaps models add URL`, `chaps models enable ID` or `chaps components enable NAME`";
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
            0 => "no models enabled; run `chaps models enable ID` to add one".to_string(),
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

/// One hint per row that needs doing something about, in table order.
///
/// A model whose container is up but which chap-core does not know about is
/// not a crash to read the logs for: chapkit tries to register five times
/// while it starts and then gives up for good, so a model that came up before
/// chap-core was healthy stays invisible until it is restarted.
///
/// `--all` because nothing about the service has changed: it is running the
/// image and the configuration it should be, and the restart is only there to
/// make it introduce itself again. A plain `chaps restart` would recreate
/// what moved, which here is nothing.
///
/// `auth` adds the other reason a model never appears on a protected
/// deployment: chap-core rejects a registration that carries no key, and a
/// chap-core created before `compose.chaps.yml` passed the key through never
/// had one to check against.
/// The line under a clean status: registration is a heartbeat, and the only
/// way to know a model can work is to make it work.
pub const TEST_HINT: &str = "run `chaps models test --all` to check they can run";

/// [`TEST_HINT`] for a deployment with one model.
pub const TEST_HINT_ONE: &str = "run `chaps models test --all` to check it can run";

/// The extra hint for a model that has not registered with a chap-core
/// elsewhere: registering there needs the image to listen on the port it
/// advertises, and an image that ignores `PORT` never gets past servicekit's
/// readiness check.
pub fn external_registration_hints(rows: &[ModelStatus]) -> Vec<String> {
    rows.iter()
        .filter(|row| row.state == ModelState::RunningNotRegistered && !row.young)
        .map(|row| {
            format!(
                "{id}: with a chap-core elsewhere the image has to listen on `PORT`; if \
                 `chaps logs {id}` shows `App never became ready`, it does not, so run it \
                 with chaps' own chap-core (`chaps components enable chap-core`)",
                id = row.id
            )
        })
        .collect()
}

/// `elsewhere` is the URL of a chap-core this deployment does not run, which
/// changes what an unreachable model most likely means.
pub fn hints(rows: &[ModelStatus], auth: bool, elsewhere: Option<&str>) -> Vec<String> {
    let registration_key = if auth {
        concat!(
            "; if its log shows 401, chap-core is missing the registration key: ",
            "run `chaps sync`, then `chaps restart`"
        )
    } else {
        ""
    };
    let mine: Vec<&ModelStatus> = rows
        .iter()
        .filter(|row| row.state != ModelState::Unmanaged)
        .collect();
    let hints: Vec<String> = mine
        .iter()
        .filter_map(|row| match row.state {
            ModelState::RunningNotRegistered if row.registered_as.is_some() => {
                let actual = row.registered_as.as_deref().unwrap_or_default();
                Some(match &row.added_from {
                    Some((model, source)) => format!(
                        "{}: its container registered as `{actual}`, the unmanaged row above; \
                         run `chaps models remove {model}`, then `chaps models add {source} \
                         --service-id {actual}`",
                        row.id
                    ),
                    None => format!(
                        "{}: its container registered as `{actual}`, the unmanaged row above; \
                         `chaps models remove` the model and add it again with `--service-id \
                         {actual}`",
                        row.id
                    ),
                })
            }
            // Started moments ago: registering is part of starting, and the
            // restart below would only start the wait over.
            ModelState::RunningNotRegistered if row.young => Some(format!(
                "{}: started under two minutes ago and registers once it is ready; run `chaps \
                 status` again in a minute",
                row.id
            )),
            ModelState::RunningNotRegistered => Some(format!(
                "{}: restart it with `chaps restart --all {}`{registration_key}",
                row.id, row.id
            )),
            ModelState::NotRunning => Some(format!(
                "{}: start Chap with `chaps up`, then `chaps logs {}`",
                row.id, row.id
            )),
            ModelState::RunningNotAnswering => Some(format!(
                "{}: read `chaps logs {}`; a model still starting answers in a moment",
                row.id, row.id
            )),
            ModelState::Unreachable => Some(unreachable_hint(row, elsewhere)),
            ModelState::Registered | ModelState::Unmanaged | ModelState::Up => None,
        })
        .collect();
    // Nothing to fix is not nothing to do: every model answered its
    // heartbeat, which is as far as `chaps status` can see.
    if hints.is_empty() && !mine.is_empty() {
        let hint = if mine.len() == 1 {
            TEST_HINT_ONE
        } else {
            TEST_HINT
        };
        return vec![hint.to_string()];
    }
    hints
}

/// The hint for a model chap-core has registered and cannot reach.
///
/// With a chap-core elsewhere the cause is nearly always the address: models
/// register as `localhost:<port>` by default, which is this machine for a
/// chap-core running as a process here and the chap-core container itself for
/// one running in Docker. With chaps' own chap-core both sit on the compose
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
             container, run `chaps components enable chap-core --url {api} --models-host \
             host.docker.internal`, then `chaps up`",
            id = row.id
        ),
        None => format!(
            "{id}: chap-core cannot reach it at {url} ({answer}); read `chaps logs {id}` and \
             `chaps logs chap`",
            id = row.id
        ),
    }
}
