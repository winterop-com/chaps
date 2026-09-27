//! `chaps dhis2` — the three things that let the DHIS2 Modeling App reach CHAP.
//!
//! Phase 1 of the `dhis2` component put a DHIS2 and a CHAP side by side and
//! left them unable to talk. This is the other half, and it is entirely
//! requests to DHIS2: the app reaches chap-core through a DHIS2 Route, reads
//! its figures out of the `analytics_*` tables, and does not exist at all until
//! it is installed. [`crate::dhis2`] holds the protocol and the rules; this
//! module is the command surface, the waits and what is said about them.
//!
//! Four verbs that change something and one that does not:
//!
//! - `show` asks and reports, and is safe on anything;
//! - `route` creates the `chap` route, or **repoints** one that is there;
//! - `analytics` generates the tables and waits for them;
//! - `apps` installs the Modeling App and the Climate App from the App Hub;
//! - `connect` does the route, the apps and then analytics, in that order.
//!
//! Every one of them is idempotent: run twice and the second run says there was
//! nothing to do. Nothing here is done by `chaps up`, and
//! [the chapter](../../docs/dhis2.md) says why.

use crate::cli::{
    Dhis2AnalyticsArgs, Dhis2AppsArgs, Dhis2CommonArgs, Dhis2ConnectArgs, Dhis2RouteArgs,
    Dhis2ShowArgs,
};
use crate::commands::Ctx;
use crate::components::Component;
use crate::dhis2::{self, Dhis2, HubAppRef, PasswordSource, Progress, Ready, RouteAction};
use crate::docker;
use crate::error::Result;
use crate::open::{Openable, resolve};
use crate::output::Out;
use crate::project::Project;
use serde::Serialize;
use std::time::{Duration, Instant};

/// Why the route has nothing to point at on a deployment without chap-core.
///
/// `--without chap-core` is a supported shape - a deployment that is only a
/// DHIS2 and an OCS - and on one of those the route would resolve to a
/// container that does not exist, which is worse than no route at all.
const NO_CHAP_CORE: &str = "chap-core is not a component of this deployment, so the route would \
     point at a service that is not there; run `chaps components enable chap-core` first";

/// What to say when docker knows DHIS2 is not running.
const NOT_RUNNING: &str = "no dhis2 container is running, so its API cannot be asked; run \
     `chaps up` to start this deployment";

/// Why there is no DHIS2 here to ask.
///
/// [`crate::open::off_reason`] says the same thing about the same component and
/// ends in "nothing to open", which is the browser's verb; these commands make
/// requests, so they say what they cannot do.
const NO_DHIS2: &str = "dhis2 is not a component of this deployment, so there is no DHIS2 to ask; \
     run `chaps components enable dhis2` to add it";

/// Why a DHIS2 that publishes no host port cannot be asked from this machine.
fn no_host_port(inside: &str) -> String {
    format!(
        "dhis2 publishes no host port, so its API cannot be reached from this machine: it answers \
         at {inside} inside the deployment; run `chaps components enable dhis2 --port N` to \
         publish one"
    )
}

// ---------------------------------------------------------------------------
// What the reports are made of
// ---------------------------------------------------------------------------

/// The instance every verb opens with: where it is, what it is, and who it was
/// asked as.
///
/// `password_from` is where the password was found and never the password
/// itself, which is also the only thing about it a human report prints.
#[derive(Debug, Clone, Serialize)]
pub struct Instance {
    pub url: String,
    /// DHIS2's own version, or empty when it did not say.
    pub version: String,
    pub user: String,
    pub password_from: PasswordSource,
}

/// What `route` did to the `chap` route.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum RouteOutcome {
    /// There was none, and now there is one.
    Created,
    /// There was one, aimed somewhere else or broken, and it was rewritten.
    Repointed,
    /// There was one, already pointing here. Nothing was written.
    Unchanged,
}

/// What became of the route.
#[derive(Debug, Clone, Serialize)]
pub struct RouteReport {
    pub outcome: RouteOutcome,
    pub code: &'static str,
    /// Where it points now.
    pub url: String,
    /// Where it pointed before, for a route that was repointed.
    pub previous: Option<String>,
    /// Why it had to be rewritten, one clause each.
    pub reasons: Vec<String>,
    pub id: Option<String>,
    /// Whether chap-core answered a request proxied through it. This is the
    /// check worth making: it proves the whole path rather than the row.
    pub verified: bool,
    /// What chap-core said through the route, or why nothing did.
    pub answered: String,
}

/// What became of the analytics tables.
#[derive(Debug, Clone, Serialize)]
pub struct AnalyticsReport {
    /// The DHIS2 job, empty when its id could not be read off the answer.
    pub job: String,
    /// Whether this run started the job, as opposed to watching one that was
    /// already going. DHIS2 runs one at a time.
    pub started: bool,
    /// Whether it had finished when the command stopped watching.
    pub finished: bool,
    pub seconds: u64,
    /// The last step DHIS2 announced.
    pub message: String,
    /// What DHIS2 records as the last successful analytics run afterwards.
    pub last_success: String,
}

/// What became of one app.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum AppOutcome {
    Installed,
    /// A different version was installed, and this one replaced it.
    Moved,
    /// The version this instance can run was already installed.
    Unchanged,
    Failed,
}

/// One app, and what happened to it.
#[derive(Debug, Clone, Serialize)]
pub struct AppReport {
    /// The App Hub's own name for it.
    pub name: String,
    pub outcome: AppOutcome,
    /// The version that is installed now, or that was going to be.
    pub version: String,
    /// The version that was there before, for one that moved.
    pub previous: Option<String>,
    pub reason: Option<String>,
}

/// Every app this command touched.
#[derive(Debug, Clone, Serialize)]
pub struct AppsReport {
    pub apps: Vec<AppReport>,
}

/// What one run of `route`, `analytics`, `apps` or `connect` did.
///
/// One shape for all four, so `--json` reads the same whichever was typed and a
/// step that was not part of the command is `null` rather than absent.
#[derive(Debug, Clone, Serialize)]
pub struct Dhis2Report {
    pub instance: Instance,
    pub route: Option<RouteReport>,
    pub apps: Option<AppsReport>,
    pub analytics: Option<AnalyticsReport>,
    /// Steps that were not attempted, and why. A skip is reported, never
    /// swallowed.
    pub skipped: Vec<String>,
    /// The command to run next.
    pub next: String,
}

/// The `chap` route as it stands, for `show`.
#[derive(Debug, Clone, Serialize)]
pub struct ShownRoute {
    pub url: String,
    pub disabled: bool,
    pub authorised: bool,
    /// Whether it points at this deployment's chap-core.
    pub ours: bool,
    pub verified: bool,
    pub answered: String,
}

/// One installed app, for `show`.
#[derive(Debug, Clone, Serialize)]
pub struct ShownApp {
    pub name: String,
    pub version: String,
}

/// What `show` found.
#[derive(Debug, Clone, Serialize)]
pub struct ShowReport {
    pub instance: Instance,
    /// Where the route has to point for this deployment.
    pub target: String,
    /// The route that is there, or `null` when there is none.
    pub route: Option<ShownRoute>,
    /// When analytics last succeeded, as DHIS2 records it. Empty for an
    /// instance that has never run it.
    pub last_analytics: String,
    pub apps: Vec<ShownApp>,
    /// Whichever of the three is missing, one clause each.
    pub missing: Vec<String>,
    pub next: String,
}

// ---------------------------------------------------------------------------
// The session every verb opens with
// ---------------------------------------------------------------------------

/// A DHIS2 that has answered, and the deployment it belongs to.
struct Session {
    project: Project,
    dhis2: Dhis2,
    ready: Ready,
}

impl Session {
    /// The instance line every report opens with.
    fn instance(&self) -> Instance {
        Instance {
            url: self.dhis2.base().to_string(),
            version: self.ready.version.clone(),
            user: self.dhis2.user().to_string(),
            password_from: self.dhis2.password_source(),
        }
    }

    /// Where this deployment's route has to point.
    fn target(&self) -> String {
        dhis2::route_target(&self.project.root_path())
    }
}

/// Resolve the instance, check that asking it is possible, and wait for it.
///
/// Four refusals before a single request is made, each naming what is true
/// instead and the command that changes it: a deployment without the component,
/// one whose DHIS2 publishes no host port and so cannot be reached from this
/// machine at all, one whose container is not running, and one that never
/// answers within the wait. A docker that cannot be asked is not a refusal -
/// that would refuse on every machine docker is absent from - so the container
/// check degrades to a trace line and the request is made anyway.
fn open_session(ctx: &Ctx, common: &Dhis2CommonArgs) -> Result<Session> {
    let project = ctx.project()?;
    let components = &project.state.components;
    let base = match resolve(Component::Dhis2, components, &project.api_base()) {
        Openable::Url { url, .. } => url,
        Openable::Internal { inside } => return Err(anyhow::anyhow!(no_host_port(&inside))),
        Openable::Off | Openable::NoWeb => return Err(anyhow::anyhow!(NO_DHIS2)),
    };
    match docker::running_containers_or_why(&project) {
        Ok(containers) => {
            if !docker::running_of(&containers).contains(Component::Dhis2.service()) {
                return Err(anyhow::anyhow!(NOT_RUNNING));
            }
        }
        Err(why) => ctx.out.verbose(&format!(
            "docker could not be asked whether dhis2 is running ({why}), so DHIS2 is asked directly"
        )),
    }

    let credentials = dhis2::credentials_for(&project.dir, common.user.as_deref());
    let client = Dhis2::new(&base, credentials, dhis2::DEFAULT_TIMEOUT);
    let ready = client.wait_for_api(
        Duration::from_secs(common.wait),
        dhis2::POLL_INTERVAL,
        // A command that is about to sit here for twenty minutes says so, on
        // stderr, so a `--json` stdout stays one document. A DHIS2 that is
        // already answering says nothing at all.
        &mut |wait| {
            crate::output::notice(&format!(
                "DHIS2 at {base} is not answering {} yet; waiting up to {}, and `chaps logs dhis2` \
                 is where the migration shows",
                dhis2::PING_PATH,
                crate::output::human_age(wait)
            ));
        },
    )?;
    ctx.out.verbose(&format!(
        "DHIS2 {} answered in {}",
        ready.version,
        crate::output::human_age(ready.waited)
    ));
    Ok(Session {
        project,
        dhis2: client,
        ready,
    })
}

// ---------------------------------------------------------------------------
// show
// ---------------------------------------------------------------------------

/// `chaps dhis2 show` — what this DHIS2 has, changing nothing.
pub fn show(ctx: &Ctx, args: &Dhis2ShowArgs) -> Result<()> {
    let session = open_session(ctx, &args.common)?;
    let target = session.target();
    let route = dhis2::route_in(&session.dhis2.get_json(&dhis2::routes_query())?);
    let shown = route.as_ref().map(|route| {
        let (verified, answered) = match route.url == target {
            // Only worth proxying through a route that points here: one aimed
            // at somebody else's chap-core would answer, and the answer would
            // mean nothing about this deployment.
            true => verify_route(&session.dhis2),
            false => (false, "it points at another chap-core".to_string()),
        };
        ShownRoute {
            url: route.url.clone(),
            disabled: route.disabled,
            authorised: route
                .authorities
                .iter()
                .any(|authority| authority == dhis2::ROUTE_AUTHORITY),
            ours: route.url == target,
            verified,
            answered,
        }
    });
    let apps = installed_apps(&session)?;

    let mut missing = Vec::new();
    match &shown {
        None => missing.push("there is no `chap` route".to_string()),
        Some(route) if !route.ours => missing.push(format!(
            "the `chap` route points at {}, not at this deployment",
            route.url
        )),
        Some(route) if route.disabled => missing.push("the `chap` route is disabled".to_string()),
        Some(route) if !route.authorised => missing.push(format!(
            "the `chap` route is missing the {} authority",
            dhis2::ROUTE_AUTHORITY
        )),
        Some(_) => {}
    }
    if session.ready.last_analytics.trim().is_empty() {
        missing.push("analytics has never run".to_string());
    }
    missing.extend(missing_apps(&apps));

    let report = ShowReport {
        instance: session.instance(),
        target,
        route: shown,
        last_analytics: session.ready.last_analytics.clone(),
        apps,
        next: String::new(),
        missing,
    };
    let report = ShowReport {
        next: match report.missing.is_empty() {
            true => {
                "the Modeling App can reach CHAP; open DHIS2 with `chaps open dhis2`".to_string()
            }
            false => "run `chaps dhis2 connect` to do the rest".to_string(),
        },
        ..report
    };
    ctx.out.emit(&report, || human_show(&report, &ctx.out))
}

/// The apps the instance lists, as `show` prints them.
fn installed_apps(session: &Session) -> Result<Vec<ShownApp>> {
    let listing = session.dhis2.get_json(dhis2::APPS_PATH)?;
    Ok(listing
        .as_array()
        .map(|rows| {
            rows.iter()
                .filter_map(|row| serde_json::from_value::<dhis2::InstalledApp>(row.clone()).ok())
                .map(|app| ShownApp {
                    name: match app.name.trim().is_empty() {
                        true => app.key,
                        false => app.name,
                    },
                    version: app.version,
                })
                .collect()
        })
        .unwrap_or_default())
}

/// The name one of [`dhis2::HUB_APPS`] is installed under, if it is.
///
/// Matched on the App Hub's own name rather than on the label a sentence uses,
/// because the two differ: the app published as `DHIS2 Climate App` is `the
/// Climate App` in prose.
fn app_name_of(installed: &[ShownApp], app: &HubAppRef) -> String {
    let listing = serde_json::Value::Array(
        installed
            .iter()
            .map(|app| serde_json::json!({"name": app.name, "version": app.version}))
            .collect(),
    );
    dhis2::installed_app(&listing, app.name)
        .map(|found| found.name)
        .unwrap_or_default()
}

/// Which of the two apps this instance does not have.
fn missing_apps(installed: &[ShownApp]) -> Vec<String> {
    dhis2::HUB_APPS
        .iter()
        .filter(|app| app_name_of(installed, app).is_empty())
        .map(|app| format!("{} is not installed", app.label))
        .collect()
}

// ---------------------------------------------------------------------------
// route
// ---------------------------------------------------------------------------

/// `chaps dhis2 route` — create the `chap` route, or repoint the one that is
/// there.
pub fn route(ctx: &Ctx, args: &Dhis2RouteArgs) -> Result<()> {
    let session = open_session(ctx, &args.common)?;
    let route = write_route(ctx, &session)?;
    let report = Dhis2Report {
        instance: session.instance(),
        next: "run `chaps dhis2 apps` next, or `chaps dhis2 show` to see what is still missing"
            .to_string(),
        route: Some(route),
        apps: None,
        analytics: None,
        skipped: Vec::new(),
    };
    ctx.out.emit(&report, || human_report(&report, &ctx.out))
}

/// Put the route where it belongs, and prove the path through it.
fn write_route(ctx: &Ctx, session: &Session) -> Result<RouteReport> {
    if !session
        .project
        .state
        .components
        .is_enabled(Component::ChapCore)
    {
        return Err(anyhow::anyhow!(NO_CHAP_CORE));
    }
    let target = session.target();
    let existing = dhis2::route_in(&session.dhis2.get_json(&dhis2::routes_query())?);
    let action = dhis2::route_action(existing.as_ref(), &target);
    ctx.out.verbose(&format!("the chap route: {action:?}"));

    let payload = dhis2::route_payload(&target);
    let (outcome, reasons) = match &action {
        RouteAction::Keep => (RouteOutcome::Unchanged, Vec::new()),
        RouteAction::Create => {
            send_route(session, "POST", dhis2::ROUTES_PATH, &payload, &target)?;
            (RouteOutcome::Created, Vec::new())
        }
        RouteAction::Rewrite(reasons) => {
            // A route is replaced in place rather than deleted and recreated:
            // the id is what DHIS2's own sharing and audit rows point at.
            let id = existing
                .as_ref()
                .map(|route| route.id.clone())
                .unwrap_or_default();
            let path = format!("{}/{}", dhis2::ROUTES_PATH, crate::api::encode(&id));
            send_route(session, "PUT", &path, &payload, &target)?;
            (RouteOutcome::Repointed, reasons.clone())
        }
    };

    let (verified, answered) = verify_route(&session.dhis2);
    // The route is correct whatever chap-core did, so a chap-core that is not
    // answering is said rather than raised: it is `chaps up`'s problem, not
    // this command's, and the report carries `verified: false` for a script.
    if !verified {
        crate::output::warn(&format!(
            "the `{}` route is in place but nothing answered through it: {answered}; run \
             `chaps status` to see whether chap-core is up",
            dhis2::ROUTE_CODE
        ));
    }
    // A create has just made an id nothing here has seen, so that one case
    // re-reads the listing; the other two already know it, and a second
    // request for something already in hand is a request not worth making.
    let id = match outcome {
        RouteOutcome::Created => {
            dhis2::route_in(&session.dhis2.get_json(&dhis2::routes_query())?).map(|route| route.id)
        }
        _ => existing.as_ref().map(|route| route.id.clone()),
    }
    .filter(|id| !id.is_empty());
    Ok(RouteReport {
        outcome,
        code: dhis2::ROUTE_CODE,
        previous: existing
            .map(|route| route.url)
            .filter(|url| *url != target && outcome != RouteOutcome::Unchanged),
        url: target,
        reasons,
        id,
        verified,
        answered,
    })
}

/// One route write, with the allowlist explained where that is what refused it.
fn send_route(
    session: &Session,
    method: &str,
    path: &str,
    payload: &str,
    target: &str,
) -> Result<()> {
    let answer = session.dhis2.send(method, path, Some(payload))?;
    if answer.is_success() {
        return Ok(());
    }
    if dhis2::is_allowlist_refusal(&answer) {
        return Err(anyhow::anyhow!(dhis2::allowlist_hint(target)));
    }
    Err(session.dhis2.status_error(path, &answer))
}

/// Ask chap-core for its health through the route, the way the app would.
///
/// The check worth making: a row in DHIS2's database proves nothing, and this
/// proves DHIS2 resolved the hostname, was allowed to reach it, and got an
/// answer. The path is the code rather than the id because that is the address
/// the Modeling App itself uses.
fn verify_route(client: &Dhis2) -> (bool, String) {
    let path = dhis2::route_run_path(crate::status::HEALTH_PATH);
    let answer = match client.send("GET", &path, None) {
        Ok(answer) => answer,
        Err(err) => return (false, err.to_string()),
    };
    if !answer.is_success() {
        // The status line alone where DHIS2's message is the reason phrase
        // again: "HTTP 502 Bad Gateway: Bad Gateway" says it twice.
        let said = dhis2::said(&answer);
        return match said.is_empty() || answer.status_line().ends_with(&said) {
            true => (false, answer.status_line()),
            false => (false, format!("{}: {said}", answer.status_line())),
        };
    }
    match answer.json().and_then(|body| {
        body.get("message")
            .or_else(|| body.get("status"))
            .and_then(serde_json::Value::as_str)
            .map(str::to_string)
    }) {
        Some(message) => (true, message),
        // A 200 that is not chap-core's health document is somebody else on
        // that hostname, which is exactly the thing this route gets wrong.
        None => (
            false,
            format!(
                "a {} that is not chap-core's health document",
                answer.body_description()
            ),
        ),
    }
}

// ---------------------------------------------------------------------------
// analytics
// ---------------------------------------------------------------------------

/// `chaps dhis2 analytics` — generate the analytics tables.
pub fn analytics(ctx: &Ctx, args: &Dhis2AnalyticsArgs) -> Result<()> {
    let session = open_session(ctx, &args.common)?;
    let analytics = run_analytics(ctx, &session, args.timeout, args.no_wait)?;
    let report = Dhis2Report {
        instance: session.instance(),
        next: match analytics.finished {
            true => "run `chaps dhis2 show` to see what is still missing".to_string(),
            false => "run `chaps dhis2 analytics` again to watch the same run".to_string(),
        },
        route: None,
        apps: None,
        analytics: Some(analytics),
        skipped: Vec::new(),
    };
    ctx.out.emit(&report, || human_report(&report, &ctx.out))
}

/// Start an analytics run, or adopt the one that is already going, and wait.
fn run_analytics(
    ctx: &Ctx,
    session: &Session,
    timeout: u64,
    no_wait: bool,
) -> Result<AnalyticsReport> {
    let client = &session.dhis2;
    // DHIS2 runs one analytics job at a time. A second POST queues behind the
    // first and its notifier stays empty, so a wait on it would report nothing
    // for as long as the first one takes - which is why the one already going
    // is watched instead of a new one being asked for.
    let running = dhis2::running_job(&client.get_json(&dhis2::jobs_path())?);
    let (job, started) = match running {
        Some(job) => {
            ctx.out
                .verbose(&format!("analytics job {job} is already running"));
            (job, false)
        }
        None => {
            // No body: the endpoint takes its two answers in the query string,
            // and this is the request that was measured against a live DHIS2.
            let answer = client.send("POST", dhis2::ANALYTICS_PATH, None)?;
            if !answer.is_success() {
                return Err(client.status_error(dhis2::ANALYTICS_PATH, &answer));
            }
            let response = answer.json().unwrap_or(serde_json::Value::Null);
            // An id that could not be read is not a failure: DHIS2 has started
            // the run, and the run it is running is the one to watch.
            let job = dhis2::job_id_of(&response)
                .or_else(|| dhis2::running_job(&client.get_json(&dhis2::jobs_path()).ok()?))
                .unwrap_or_default();
            (job, true)
        }
    };

    if no_wait || job.is_empty() {
        return Ok(AnalyticsReport {
            job,
            started,
            finished: false,
            seconds: 0,
            message: String::new(),
            last_success: session.ready.last_analytics.clone(),
        });
    }

    let began = Instant::now();
    // Not `now() - interval`: a run that finishes on the first poll would print
    // its one step above the report it is part of, which reads as noise.
    let mut last = Instant::now();
    let progress = client.wait_for_job(
        &job,
        Duration::from_secs(timeout),
        dhis2::POLL_INTERVAL,
        // A run that takes an hour must not be an hour of silence. Every step
        // under `-v`, and otherwise one line every half minute, on stderr.
        &mut |message| {
            if ctx.out.is_verbose() {
                ctx.out.verbose(&format!("analytics: {message}"));
            } else if last.elapsed() >= Duration::from_secs(30) {
                last = Instant::now();
                crate::output::notice(&format!("analytics: {message}"));
            }
        },
    )?;
    let seconds = began.elapsed().as_secs();
    if let Progress::Failed(reason) = &progress {
        return Err(anyhow::anyhow!(
            "the analytics run failed after {}: {reason}; `chaps logs dhis2` has the rest, and \
             DHIS2 wants about 4 to 5 GB for the populate phase",
            crate::output::human_age(began.elapsed())
        ));
    }
    Ok(AnalyticsReport {
        job,
        started,
        finished: true,
        seconds,
        message: progress.message().to_string(),
        // Re-read, because this is the fact that says the run landed: DHIS2
        // records the time of its last successful analytics generation.
        last_success: client
            .get_json(dhis2::SYSTEM_INFO_PATH)
            .ok()
            .as_ref()
            .and_then(|info| info.get("lastAnalyticsTableSuccess"))
            .and_then(serde_json::Value::as_str)
            .unwrap_or_default()
            .to_string(),
    })
}

// ---------------------------------------------------------------------------
// apps
// ---------------------------------------------------------------------------

/// `chaps dhis2 apps` — install the Modeling App and the Climate App.
pub fn apps(ctx: &Ctx, args: &Dhis2AppsArgs) -> Result<()> {
    if ctx.registry.offline {
        return Err(anyhow::anyhow!(dhis2::OFFLINE_APPS));
    }
    let session = open_session(ctx, &args.common)?;
    let apps = install_apps(ctx, &session);
    let failed = failed_apps(&apps);
    let report = Dhis2Report {
        instance: session.instance(),
        next: "open DHIS2 with `chaps open dhis2`; the Modeling App is in its apps menu"
            .to_string(),
        route: None,
        apps: Some(apps),
        analytics: None,
        skipped: Vec::new(),
    };
    ctx.out.emit(&report, || human_report(&report, &ctx.out))?;
    match failed {
        Some(why) => Err(anyhow::anyhow!(why)),
        None => Ok(()),
    }
}

/// Install both apps, taking each one as far as it goes.
///
/// One app's failure does not stop the other: they are independent, and an
/// operator who can get the Modeling App installed while the Climate App is
/// having a bad day is better off than one who gets neither.
fn install_apps(ctx: &Ctx, session: &Session) -> AppsReport {
    let listing = session
        .dhis2
        .get_json(dhis2::APPS_PATH)
        .unwrap_or(serde_json::Value::Null);
    let hub = dhis2::app_hub_base();
    AppsReport {
        apps: dhis2::HUB_APPS
            .iter()
            .map(|app| match install_app(ctx, session, &hub, app, &listing) {
                Ok(report) => report,
                Err(why) => AppReport {
                    name: app.name.to_string(),
                    outcome: AppOutcome::Failed,
                    version: String::new(),
                    previous: None,
                    reason: Some(first_line(&why.to_string())),
                },
            })
            .collect(),
    }
}

/// Resolve one app's version on the App Hub and have DHIS2 install it.
fn install_app(
    ctx: &Ctx,
    session: &Session,
    hub: &str,
    app: &HubAppRef,
    listing: &serde_json::Value,
) -> Result<AppReport> {
    let published = dhis2::fetch_hub_app(hub, app.id, dhis2::DEFAULT_TIMEOUT)?;
    let name = match published.name.trim().is_empty() {
        true => app.name.to_string(),
        false => published.name.clone(),
    };
    let Some(version) = dhis2::pick_version(&published.versions, &session.ready.version) else {
        return Err(anyhow::anyhow!(
            "the App Hub publishes no version of {name} that DHIS2 {} can run; install it from \
             DHIS2's own App Management page",
            display_version(&session.ready.version)
        ));
    };
    let installed = dhis2::installed_app(listing, &name);
    if let Some(installed) = &installed
        && dhis2::compare_versions(&installed.version, &version.version)
            == std::cmp::Ordering::Equal
    {
        return Ok(AppReport {
            name,
            outcome: AppOutcome::Unchanged,
            version: version.version.clone(),
            previous: None,
            reason: None,
        });
    }

    let path = dhis2::install_path(&version.id);
    ctx.out
        .verbose(&format!("installing {name} {}", version.version));
    // The instance downloads the app itself, so this one request is a transfer.
    let client = session.dhis2.with_timeout(dhis2::INSTALL_TIMEOUT);
    let answer = client.send("POST", &path, None)?;
    if !answer.is_success() {
        return Err(client.status_error(&path, &answer));
    }
    Ok(AppReport {
        name,
        outcome: match &installed {
            Some(_) => AppOutcome::Moved,
            None => AppOutcome::Installed,
        },
        version: version.version.clone(),
        previous: installed.map(|app| app.version),
        reason: None,
    })
}

/// The one line the command fails with when an app could not be installed.
fn failed_apps(report: &AppsReport) -> Option<String> {
    let failed: Vec<&AppReport> = report
        .apps
        .iter()
        .filter(|app| app.outcome == AppOutcome::Failed)
        .collect();
    let first = failed.first()?;
    Some(format!(
        "{} of {} could not be installed: {}",
        failed.len(),
        report.apps.len(),
        first.reason.clone().unwrap_or_default()
    ))
}

// ---------------------------------------------------------------------------
// connect
// ---------------------------------------------------------------------------

/// `chaps dhis2 connect` — the route, the apps, then analytics.
///
/// That order, and not the order the chapter lists them in. The two cheap steps
/// come first so a failure in either is reported in seconds rather than after
/// the analytics wait, and analytics is last because it is the long one and
/// nothing else waits on it: with the route and the apps already in place, the
/// deployment is usable the moment the tables land.
pub fn connect(ctx: &Ctx, args: &Dhis2ConnectArgs) -> Result<()> {
    let session = open_session(ctx, &args.common)?;
    let mut skipped = Vec::new();

    let route = write_route(ctx, &session)?;

    // `--offline` and the App Hub ask for opposite things, so the step is a
    // reported skip rather than a failure: the route and the analytics tables
    // need nothing but this deployment, and both still happen.
    let apps = match ctx.registry.offline {
        true => {
            skipped.push(dhis2::OFFLINE_APPS.to_string());
            None
        }
        false => Some(install_apps(ctx, &session)),
    };
    let failed = apps.as_ref().and_then(failed_apps);

    let analytics = run_analytics(ctx, &session, args.timeout, args.no_wait)?;

    let done = route.verified && failed.is_none() && analytics.finished && skipped.is_empty();
    let report = Dhis2Report {
        instance: session.instance(),
        next: match done {
            true => {
                "the Modeling App can reach CHAP; open DHIS2 with `chaps open dhis2`".to_string()
            }
            false => "run `chaps dhis2 show` to see what is still missing".to_string(),
        },
        route: Some(route),
        apps,
        analytics: Some(analytics),
        skipped,
    };
    ctx.out.emit(&report, || human_report(&report, &ctx.out))?;
    match failed {
        Some(why) => Err(anyhow::anyhow!(why)),
        None => Ok(()),
    }
}

// ---------------------------------------------------------------------------
// The human rendering
// ---------------------------------------------------------------------------

/// The line every verb opens with: where DHIS2 is, what it is, and who it was
/// asked as. Never the password, only where it was found.
fn instance_line(instance: &Instance, out: &Out) -> String {
    format!(
        "{} at {}, as {} ({})\n",
        out.value(&format!("DHIS2 {}", display_version(&instance.version))),
        out.value(&instance.url),
        out.value(&format!("`{}`", instance.user)),
        out.dim(instance.password_from.describe()),
    )
}

/// A version to print, or the words for an instance that did not say.
fn display_version(version: &str) -> String {
    match version.trim().is_empty() {
        true => "(version unknown)".to_string(),
        false => version.trim().to_string(),
    }
}

/// What `route`, `analytics`, `apps` and `connect` print.
fn human_report(report: &Dhis2Report, out: &Out) -> String {
    let mut text = instance_line(&report.instance, out);
    if let Some(route) = &report.route {
        text.push_str(&route_lines(route, out));
    }
    if let Some(apps) = &report.apps {
        text.push_str(&app_lines(apps, out));
    }
    if let Some(analytics) = &report.analytics {
        text.push_str(&analytics_lines(analytics, out));
    }
    for skip in &report.skipped {
        text.push_str(&format!(
            "{} {}\n",
            out.dim("skipped:"),
            out.backticks(skip)
        ));
    }
    text.push_str(&out.backticks(&report.next));
    text.push('\n');
    text
}

fn route_lines(route: &RouteReport, out: &Out) -> String {
    let where_ = out.value(&route.url);
    let mut text = match route.outcome {
        RouteOutcome::Created => format!(
            "{} the `{}` route at {where_}\n",
            out.ok("created"),
            route.code
        ),
        RouteOutcome::Repointed => format!(
            "{} the `{}` route at {where_}\n",
            out.ok("repointed"),
            route.code
        ),
        RouteOutcome::Unchanged => format!(
            "the `{}` route already points at {where_}; nothing to change\n",
            route.code
        ),
    };
    for reason in &route.reasons {
        text.push_str(&format!("  {}\n", out.dim(reason)));
    }
    text.push_str(&match route.verified {
        true => format!(
            "  {} chap-core answered through it: {}\n",
            out.ok("verified"),
            route.answered
        ),
        false => format!(
            "  {} nothing answered through it: {}\n",
            out.warn("unverified"),
            route.answered
        ),
    });
    text
}

fn app_lines(apps: &AppsReport, out: &Out) -> String {
    let mut text = String::new();
    for app in &apps.apps {
        let name = out.value(&app.name);
        text.push_str(&match app.outcome {
            AppOutcome::Installed => {
                format!("{} {name} {}\n", out.ok("installed"), app.version)
            }
            AppOutcome::Moved => format!(
                "{} {name} from {} to {}\n",
                out.ok("moved"),
                app.previous.clone().unwrap_or_default(),
                app.version
            ),
            AppOutcome::Unchanged => {
                format!("{name} {} is already installed\n", app.version)
            }
            AppOutcome::Failed => format!(
                "{} {name} was not installed: {}\n",
                out.bad("failed"),
                app.reason.clone().unwrap_or_default()
            ),
        });
    }
    text
}

fn analytics_lines(analytics: &AnalyticsReport, out: &Out) -> String {
    let job = out.dim(&format!("(job {})", analytics.job));
    if !analytics.finished {
        return match analytics.started {
            true => format!("{} the analytics run {job}\n", out.ok("started")),
            false => format!("an analytics run was already going {job}\n"),
        };
    }
    let mut text = format!(
        "{} in {} {job}\n",
        out.ok(match analytics.started {
            true => "analytics finished",
            // The distinction matters: this run watched somebody else's job,
            // so the duration is how long it watched, not how long it took.
            false => "the analytics run that was already going finished",
        }),
        crate::output::human_age(Duration::from_secs(analytics.seconds)),
    );
    if !analytics.message.is_empty() {
        text.push_str(&format!("  {}\n", out.dim(&analytics.message)));
    }
    if !analytics.last_success.is_empty() {
        text.push_str(&format!(
            "  {}\n",
            out.dim(&format!(
                "DHIS2 records its last analytics success as {}",
                analytics.last_success
            ))
        ));
    }
    text
}

/// What `show` prints: three rows and whatever is missing.
fn human_show(report: &ShowReport, out: &Out) -> String {
    let mut text = instance_line(&report.instance, out);
    let route = match &report.route {
        None => out.warn("none"),
        Some(route) => {
            let mut cell = out.value(&route.url);
            if !route.ours {
                cell = format!("{cell} {}", out.warn("(another chap-core)"));
            }
            if route.disabled {
                cell = format!("{cell} {}", out.warn("(disabled)"));
            }
            if !route.authorised {
                cell = format!(
                    "{cell} {}",
                    out.warn(&format!("(no {} authority)", dhis2::ROUTE_AUTHORITY))
                );
            }
            if route.verified {
                cell = format!("{cell} {}", out.ok(&format!("({})", route.answered)));
            }
            cell
        }
    };
    let apps = match report.apps.is_empty() {
        true => out.warn("none"),
        false => report
            .apps
            .iter()
            .map(|app| format!("{} {}", app.name, app.version))
            .collect::<Vec<String>>()
            .join(", "),
    };
    text.push_str(&crate::output::fields_with(
        0,
        &[
            ("route", route),
            ("target", out.dim(&report.target)),
            (
                "analytics",
                match report.last_analytics.is_empty() {
                    true => out.warn("never run"),
                    false => out.value(&report.last_analytics),
                },
            ),
            ("apps", apps),
        ],
        &|label| out.key(label),
    ));
    for missing in &report.missing {
        text.push_str(&format!("{} {}\n", out.dim("missing:"), missing));
    }
    text.push_str(&out.backticks(&report.next));
    text.push('\n');
    text
}

/// The first line of an error, for a cell that has one line to say it in.
fn first_line(text: &str) -> String {
    text.lines().next().unwrap_or_default().trim().to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn out() -> Out {
        Out::detect(false, true)
    }

    fn instance() -> Instance {
        Instance {
            url: "http://localhost:18080".to_string(),
            version: "2.42.6".to_string(),
            user: "admin".to_string(),
            password_from: PasswordSource::Default,
        }
    }

    fn route_report(outcome: RouteOutcome) -> RouteReport {
        RouteReport {
            outcome,
            code: dhis2::ROUTE_CODE,
            url: "http://chap:8000/**".to_string(),
            previous: None,
            reasons: Vec::new(),
            id: Some("abc123".to_string()),
            verified: true,
            answered: "healthy".to_string(),
        }
    }

    fn report(route: Option<RouteReport>) -> Dhis2Report {
        Dhis2Report {
            instance: instance(),
            route,
            apps: None,
            analytics: None,
            skipped: Vec::new(),
            next: "run `chaps dhis2 show` to see what is still missing".to_string(),
        }
    }

    /// The opening line says where, what and who - and nothing whatsoever
    /// about the password beyond where it was found.
    #[test]
    fn the_instance_line_never_carries_the_password() {
        let text = instance_line(&instance(), &out());
        assert_eq!(
            text,
            "DHIS2 2.42.6 at http://localhost:18080, as `admin` (the DHIS2 default)\n"
        );
        assert!(!text.contains("district"), "{text}");

        let from_file = Instance {
            password_from: PasswordSource::EnvFile,
            ..instance()
        };
        assert!(instance_line(&from_file, &out()).contains("(from `.env`)"));

        // An instance that did not say its version still gets a line.
        let bare = Instance {
            version: String::new(),
            ..instance()
        };
        assert!(
            instance_line(&bare, &out()).starts_with("DHIS2 (version unknown) at"),
            "{}",
            instance_line(&bare, &out())
        );
    }

    /// A repoint says where it pointed before: that is the whole reason the
    /// step is not "create if absent".
    #[test]
    fn a_repointed_route_prints_the_reason_it_was_wrong() {
        let mut route = route_report(RouteOutcome::Repointed);
        route.previous = Some("http://158.39.75.126/stable/**".to_string());
        route.reasons = vec!["it pointed at http://158.39.75.126/stable/**".to_string()];
        let text = human_report(&report(Some(route)), &out());
        assert!(
            text.contains("repointed the `chap` route at http://chap:8000/**"),
            "{text}"
        );
        assert!(
            text.contains("it pointed at http://158.39.75.126/stable/**"),
            "{text}"
        );
        assert!(
            text.contains("verified chap-core answered through it: healthy"),
            "{text}"
        );
    }

    /// Nothing to do is still said out loud, and named as nothing to do.
    #[test]
    fn a_route_that_is_already_right_says_there_was_nothing_to_change() {
        let text = human_report(&report(Some(route_report(RouteOutcome::Unchanged))), &out());
        assert!(
            text.contains("already points at http://chap:8000/**; nothing to change"),
            "{text}"
        );
        assert!(text.contains("run `chaps dhis2 show`"), "{text}");
    }

    /// A route that was written but proved nothing says so rather than
    /// claiming the path works.
    #[test]
    fn an_unverified_route_does_not_claim_chap_core_answered() {
        let mut route = route_report(RouteOutcome::Created);
        route.verified = false;
        route.answered = "HTTP 502 Bad Gateway".to_string();
        let text = human_report(&report(Some(route)), &out());
        assert!(
            text.contains("unverified nothing answered through it: HTTP 502 Bad Gateway"),
            "{text}"
        );
        assert!(!text.contains("chap-core answered through it"), "{text}");
    }

    #[test]
    fn the_app_lines_say_which_of_the_four_happened() {
        let apps = AppsReport {
            apps: vec![
                AppReport {
                    name: "Modeling App".to_string(),
                    outcome: AppOutcome::Installed,
                    version: "7.1.0".to_string(),
                    previous: None,
                    reason: None,
                },
                AppReport {
                    name: "Climate".to_string(),
                    outcome: AppOutcome::Moved,
                    version: "1.16.2".to_string(),
                    previous: Some("1.15.0".to_string()),
                    reason: None,
                },
                AppReport {
                    name: "Old".to_string(),
                    outcome: AppOutcome::Unchanged,
                    version: "1.0.0".to_string(),
                    previous: None,
                    reason: None,
                },
                AppReport {
                    name: "Broken".to_string(),
                    outcome: AppOutcome::Failed,
                    version: String::new(),
                    previous: None,
                    reason: Some("HTTP 409".to_string()),
                },
            ],
        };
        let text = app_lines(&apps, &out());
        assert!(text.contains("installed Modeling App 7.1.0"), "{text}");
        assert!(
            text.contains("moved Climate from 1.15.0 to 1.16.2"),
            "{text}"
        );
        assert!(text.contains("Old 1.0.0 is already installed"), "{text}");
        assert!(
            text.contains("failed Broken was not installed: HTTP 409"),
            "{text}"
        );

        let why = failed_apps(&apps).expect("one app failed");
        assert!(why.contains("1 of 4 could not be installed"), "{why}");
        assert!(why.contains("HTTP 409"), "{why}");
        assert_eq!(
            failed_apps(&AppsReport { apps: Vec::new() }),
            None,
            "nothing attempted is not a failure"
        );
    }

    #[test]
    fn the_analytics_lines_tell_a_started_run_from_a_finished_one() {
        let running = AnalyticsReport {
            job: "abc".to_string(),
            started: true,
            finished: false,
            seconds: 0,
            message: String::new(),
            last_success: String::new(),
        };
        assert!(
            analytics_lines(&running, &out()).contains("started the analytics run (job abc)"),
            "{}",
            analytics_lines(&running, &out())
        );

        let adopted = AnalyticsReport {
            started: false,
            ..running.clone()
        };
        assert!(
            analytics_lines(&adopted, &out()).contains("an analytics run was already going"),
            "{}",
            analytics_lines(&adopted, &out())
        );

        let done = AnalyticsReport {
            finished: true,
            seconds: 65,
            message: "Analytics tables updated".to_string(),
            last_success: "2026-09-25T10:01:00.000".to_string(),
            ..running
        };
        let text = analytics_lines(&done, &out());
        assert!(text.contains("analytics finished in 1 minute"), "{text}");
        assert!(text.contains("Analytics tables updated"), "{text}");
        assert!(
            text.contains("DHIS2 records its last analytics success as 2026-09-25T10:01:00.000"),
            "{text}"
        );
    }

    /// A skipped step is printed, never swallowed.
    #[test]
    fn a_skipped_step_is_reported_with_the_reason() {
        let mut report = report(Some(route_report(RouteOutcome::Created)));
        report.skipped = vec![dhis2::OFFLINE_APPS.to_string()];
        let text = human_report(&report, &out());
        assert!(text.contains("skipped:"), "{text}");
        assert!(text.contains("App Hub"), "{text}");
    }

    #[test]
    fn show_names_every_missing_piece_and_the_command_that_fixes_them() {
        let report = ShowReport {
            instance: instance(),
            target: "http://chap:8000/**".to_string(),
            route: None,
            last_analytics: String::new(),
            apps: Vec::new(),
            missing: vec![
                "there is no `chap` route".to_string(),
                "analytics has never run".to_string(),
                "the Modeling App is not installed".to_string(),
            ],
            next: "run `chaps dhis2 connect` to do the rest".to_string(),
        };
        let text = human_show(&report, &out());
        assert!(text.contains("route      none"), "{text}");
        assert!(text.contains("analytics  never run"), "{text}");
        assert!(text.contains("apps       none"), "{text}");
        assert!(text.contains("missing: there is no `chap` route"), "{text}");
        assert!(text.contains("run `chaps dhis2 connect`"), "{text}");
    }

    #[test]
    fn show_on_a_connected_instance_says_it_is_connected() {
        let report = ShowReport {
            instance: instance(),
            target: "http://chap:8000/**".to_string(),
            route: Some(ShownRoute {
                url: "http://chap:8000/**".to_string(),
                disabled: false,
                authorised: true,
                ours: true,
                verified: true,
                answered: "healthy".to_string(),
            }),
            last_analytics: "2026-09-25T10:01:00.000".to_string(),
            apps: vec![ShownApp {
                name: "Modeling App".to_string(),
                version: "7.1.0".to_string(),
            }],
            missing: Vec::new(),
            next: "the Modeling App can reach CHAP; open DHIS2 with `chaps open dhis2`".to_string(),
        };
        let text = human_show(&report, &out());
        assert!(text.contains("http://chap:8000/** (healthy)"), "{text}");
        assert!(text.contains("Modeling App 7.1.0"), "{text}");
        assert!(!text.contains("missing:"), "{text}");
        assert!(text.contains("`chaps open dhis2`"), "{text}");
    }

    /// A route aimed at somebody else's chap-core is the trap the whole step
    /// exists for, so `show` says so in the row as well as in the list.
    #[test]
    fn show_marks_a_route_that_points_at_another_chap_core() {
        let report = ShowReport {
            instance: instance(),
            target: "http://chap:8000/**".to_string(),
            route: Some(ShownRoute {
                url: "http://158.39.75.126/stable/**".to_string(),
                disabled: false,
                authorised: true,
                ours: false,
                verified: false,
                answered: "it points at another chap-core".to_string(),
            }),
            last_analytics: "2026-09-25T10:01:00.000".to_string(),
            apps: Vec::new(),
            missing: vec![
                "the `chap` route points at http://158.39.75.126/stable/**, not at this deployment"
                    .to_string(),
            ],
            next: "run `chaps dhis2 connect` to do the rest".to_string(),
        };
        let text = human_show(&report, &out());
        assert!(text.contains("(another chap-core)"), "{text}");
        assert!(text.contains("not at this deployment"), "{text}");
    }

    #[test]
    fn the_missing_apps_are_the_ones_the_listing_does_not_have() {
        let none: Vec<ShownApp> = Vec::new();
        let missing = missing_apps(&none);
        assert_eq!(missing.len(), 2, "{missing:?}");
        assert!(missing[0].contains("the Modeling App"), "{missing:?}");

        let both = vec![
            ShownApp {
                name: "Modeling App".to_string(),
                version: "7.1.0".to_string(),
            },
            ShownApp {
                name: "DHIS2 Climate App".to_string(),
                version: "1.16.2".to_string(),
            },
        ];
        assert!(missing_apps(&both).is_empty(), "{:?}", missing_apps(&both));

        // The installed key is the name in another spelling, and still counts.
        let keyed = vec![ShownApp {
            name: "dhis2-climate-app".to_string(),
            version: "1.16.2".to_string(),
        }];
        let missing = missing_apps(&keyed);
        assert_eq!(missing.len(), 1, "{missing:?}");
        assert!(missing[0].contains("the Modeling App"), "{missing:?}");
    }

    /// Every refusal names what is true instead and the command that changes
    /// it - and none of them says "open", which is the browser's verb and not
    /// what these commands do.
    #[test]
    fn the_refusals_name_what_is_true_instead_and_the_way_out() {
        assert!(NO_CHAP_CORE.contains("`chaps components enable chap-core`"));
        assert!(NOT_RUNNING.contains("`chaps up`"));
        assert!(NO_DHIS2.contains("`chaps components enable dhis2`"));
        let internal = no_host_port("http://dhis2:8080");
        assert!(internal.contains("http://dhis2:8080"), "{internal}");
        assert!(
            internal.contains("`chaps components enable dhis2 --port N`"),
            "{internal}"
        );
        for text in [NO_CHAP_CORE, NOT_RUNNING, NO_DHIS2, &internal] {
            assert!(!text.contains("open"), "{text}");
        }
    }

    #[test]
    fn an_error_is_reported_on_one_line() {
        assert_eq!(first_line("one\ntwo"), "one");
        assert_eq!(first_line("  one  "), "one");
        assert_eq!(first_line(""), "");
    }
}
