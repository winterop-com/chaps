//! `chaps dhis2` — the three things that let the DHIS2 Modeling App reach CHAP.
//!
//! Phase 1 of the `dhis2` component put a DHIS2 and a CHAP side by side and
//! left them unable to talk. This is the other half, and it is entirely
//! requests to DHIS2: the app reaches chap-core through a DHIS2 Route, reads
//! its figures out of the `analytics_*` tables, and does not exist at all until
//! it is installed. [`crate::dhis2`] holds the protocol and the rules; this
//! module is the command surface, the waits and what is said about them.
//!
//! Four verbs that change DHIS2, one that does not, and one that changes
//! only this deployment's record of which DHIS2 it is:
//!
//! - `show` asks and reports, and is safe on anything;
//! - `route` creates the `chap` route, or **repoints** one that is there;
//! - `analytics` generates the tables and waits for them;
//! - `apps` installs the Modeling App and the Climate App from the App Hub;
//! - `connect` does the route, the apps and then analytics, in that order;
//! - `use` records a DHIS2 that runs elsewhere, which the others then talk to
//!   in place of the `dhis2` component.
//!
//! Every one of them is idempotent: run twice and the second run says there was
//! nothing to do. Nothing here is done by `chaps up`, and
//! [the chapter](../../docs/dhis2.md) says why.

use crate::cli::{
    Dhis2AnalyticsArgs, Dhis2AppsArgs, Dhis2CommonArgs, Dhis2ConnectArgs, Dhis2RouteArgs,
    Dhis2ShowArgs, Dhis2UseArgs,
};
use crate::commands::Ctx;
use crate::components::Component;
use crate::dhis2::{
    self, AuthKind, CredentialSource, Dhis2, HubAppRef, Progress, Ready, RouteAction,
};
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
/// `credential_from` is where the password or token was found and never the
/// secret itself, which is also the only thing about it a human report prints.
#[derive(Debug, Clone, Serialize)]
pub struct Instance {
    pub url: String,
    /// DHIS2's own version, or empty when it did not say.
    pub version: String,
    /// Who the requests went as. For a token, whoever DHIS2 said owns it.
    pub user: String,
    /// Whether this is an external DHIS2 recorded by `chaps dhis2 use`,
    /// rather than the deployment's own `dhis2` component.
    pub external: bool,
    /// A password or a personal access token.
    pub auth: AuthKind,
    pub credential_from: CredentialSource,
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

/// What one `connect` left in `.chaps/components.yaml` about itself.
///
/// The record is [`crate::components::Dhis2Component::connected_at`], and its
/// only job is to stop `chaps up` and `chaps status` asking for a connect that
/// has already happened. It is never read as evidence that the route is right:
/// `chaps dhis2 show` is the one command that asks DHIS2.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum ConnectRecord {
    /// Written now: the route is verified and both apps are in place.
    Recorded,
    /// An earlier record was cleared, because this run found something broken.
    /// The hint comes back, which is the answer that errs the safe way.
    Cleared,
    /// The file was not touched: either the run judged nothing (see
    /// [`Judgement::Unknown`]) or there was nothing recorded to clear.
    Unchanged,
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
    /// What this run left recorded about itself. `null` for `route`,
    /// `analytics` and `apps`, none of which records anything: one step is not
    /// a connection, and a route with no apps is a DHIS2 nobody can use.
    pub record: Option<ConnectRecord>,
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
    /// Whether chap-core answered `/health` through it and turned the next
    /// call away for want of its API token.
    pub token_refused: bool,
}

/// One of the two apps chaps installs, as `show` found it.
///
/// One row per app chaps is about, present or not - never a row per app the
/// instance happens to have. A real DHIS2 ships 29 bundled apps of its own, and
/// listing them buried the two this command answers for.
#[derive(Debug, Clone, Serialize)]
pub struct ShownApp {
    /// The name chaps knows it by, which is also what the docs call it.
    pub name: String,
    /// Whether this instance has it at all.
    pub installed: bool,
    /// The version it has, empty when it does not have it.
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
    /// What that timestamp is worth. A seeded deployment inherits it from the
    /// dump, so on its own it says nothing about this deployment's tables.
    pub analytics: dhis2::AnalyticsEvidence,
    /// The two apps chaps installs, in order, present or not.
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
            user: self.ready.user.clone(),
            external: self.external().is_some(),
            auth: self.dhis2.credentials().kind(),
            credential_from: self.dhis2.credentials().source,
        }
    }

    /// The external DHIS2 this session talks to, if that is what it is.
    fn external(&self) -> Option<&crate::components::ExternalDhis2> {
        self.project.state.components.dhis2_external.as_ref()
    }

    /// Where this deployment's route has to point: the compose network's name
    /// for chap-core for a DHIS2 on that network, and the recorded URL for one
    /// that is not.
    ///
    /// A chap-core elsewhere is reached from the DHIS2 container over the host
    /// gateway, which is how the compose file maps it for that DHIS2.
    fn target(&self) -> String {
        match (
            self.external(),
            &self.project.state.components.chap_core_external,
        ) {
            (Some(external), _) => dhis2::external_route_target(&external.chap_url),
            (None, Some(chap_core)) => {
                dhis2::external_route_target(&chap_core.url_from_containers())
            }
            (None, None) => dhis2::route_target(&self.project.root_path()),
        }
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
    // An external DHIS2 has no container to look for: its URL is what was
    // recorded, and the wait below is the whole of the check.
    let client = match &components.dhis2_external {
        Some(external) => {
            let credentials = dhis2::credentials_for(&project.dir, common.user.as_deref(), false)?;
            Dhis2::new(&external.url, credentials, dhis2::DEFAULT_TIMEOUT).external()
        }
        None => {
            let base = match resolve(Component::Dhis2, components, &project.api_base()) {
                Openable::Url { url, .. } => url,
                Openable::Internal { inside } => {
                    return Err(anyhow::anyhow!(no_host_port(&inside)));
                }
                Openable::Off | Openable::NoWeb => return Err(anyhow::anyhow!(NO_DHIS2)),
            };
            match docker::running_containers_or_why(&project) {
                Ok(containers) => {
                    if !docker::running_of(&containers).contains(Component::Dhis2.service()) {
                        return Err(anyhow::anyhow!(NOT_RUNNING));
                    }
                }
                Err(why) => ctx.out.verbose(&format!(
                    "docker could not be asked whether dhis2 is running ({why}), so DHIS2 is \
                     asked directly"
                )),
            }
            let credentials = dhis2::credentials_for(&project.dir, common.user.as_deref(), true)?;
            Dhis2::new(&base, credentials, dhis2::DEFAULT_TIMEOUT)
        }
    };
    let base = client.base().to_string();
    let external = components.dhis2_external.is_some();
    let ready = client.wait_for_api(
        Duration::from_secs(common.wait),
        dhis2::POLL_INTERVAL,
        // A command that is about to sit here for twenty minutes says so, on
        // stderr, so a `--json` stdout stays one document. A DHIS2 that is
        // already answering says nothing at all.
        &mut |wait| {
            crate::output::notice(&format!(
                "DHIS2 at {base} is not answering {} yet; waiting up to {}{}",
                dhis2::PING_PATH,
                crate::output::human_age(wait),
                match external {
                    true => "",
                    false => ", and `chaps logs dhis2` is where the migration shows",
                }
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
    let token_needed = crate::api::token_for(Some(&session.project.dir)).is_some();
    let shown = route.as_ref().map(|route| {
        let (verified, answered, token_refused) = match route.url == target {
            // Only worth proxying through a route that points here: one aimed
            // at somebody else's chap-core would answer, and the answer would
            // mean nothing about this deployment.
            true => verify_route(&session.dhis2, token_needed),
            false => (false, "it points at another chap-core".to_string(), false),
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
            token_refused,
        }
    });
    let apps = chap_apps(&session)?;
    let analytics = analytics_evidence(ctx, &session);

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
        Some(route) if route.token_refused => missing.push(format!(
            "the `chap` route does not carry chap-core's API token: {}; run `chaps dhis2 connect` \
             to put it there",
            route.answered
        )),
        // A row that is right in every field and that nothing answers through
        // is the one case the proxied request exists to catch: the app would
        // be told CHAP is reachable and then fail on its first request.
        Some(route) if !route.verified => missing.push(format!(
            "nothing answered through the `chap` route: {}; run `chaps status` to see whether \
             chap-core is up",
            route.answered
        )),
        Some(_) => {}
    }
    match analytics {
        dhis2::AnalyticsEvidence::Never => missing.push("analytics has never run".to_string()),
        // Not "analytics has never run": it may well have, and chaps cannot
        // see the tables to tell. What it can say is that the timestamp beside
        // it proves nothing, and that one command settles the question.
        dhis2::AnalyticsEvidence::Unconfirmed => missing.push(
            "analytics may never have run on this deployment: a seeded database can carry the \
             dump's timestamp, and no run has finished since DHIS2 started; run \
             `chaps dhis2 analytics` to settle it"
                .to_string(),
        ),
        dhis2::AnalyticsEvidence::RanHere | dhis2::AnalyticsEvidence::Recorded => {}
    }
    missing.extend(missing_apps(&apps));

    let report = ShowReport {
        instance: session.instance(),
        target,
        route: shown,
        last_analytics: session.ready.last_analytics.clone(),
        analytics,
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

/// The two apps chaps installs, as this instance has them.
///
/// **The two, and not the rest.** `show` exists to say whether DHIS2 has what
/// CHAP needs; the climate demo ships 29 bundled apps of its own, the same 29
/// every DHIS2 ships, and printing them turned the one row that answers the
/// question into an inventory with the answer somewhere in the middle of it.
/// A count of the others would be a number that is always about thirty and
/// never means anything, so there is none: `GET /api/apps` is one request away
/// for anybody who wants the list.
///
/// Matched on the App Hub's own name rather than on the label a sentence uses,
/// because the two differ: the app published as `DHIS2 Climate App` is `the
/// Climate App` in prose. [`dhis2::installed_app`] is what copes with the third
/// spelling, the instance's own - it lists the Modeling App as `Modeling`.
fn chap_apps(session: &Session) -> Result<Vec<ShownApp>> {
    let listing = session.dhis2.get_json(dhis2::APPS_PATH)?;
    Ok(dhis2::HUB_APPS
        .iter()
        .map(|app| shown_app(app, dhis2::installed_app(&listing, app.name)))
        .collect())
}

/// One [`dhis2::HUB_APPS`] row, found or not.
fn shown_app(app: &HubAppRef, found: Option<dhis2::InstalledApp>) -> ShownApp {
    ShownApp {
        name: app.name.to_string(),
        installed: found.is_some(),
        version: found.map(|found| found.version).unwrap_or_default(),
    }
}

/// Which of the two apps this instance does not have.
fn missing_apps(apps: &[ShownApp]) -> Vec<String> {
    dhis2::HUB_APPS
        .iter()
        .filter(|app| {
            !apps
                .iter()
                .any(|shown| shown.installed && shown.name == app.name)
        })
        .map(|app| format!("{} is not installed", app.label))
        .collect()
}

/// What this deployment's `lastAnalyticsTableSuccess` is actually worth.
///
/// Two things it is not read against: DHIS2's notifier, which says whether a
/// run has finished on *this* instance since it started, and
/// `.chaps/components.yaml`, which says whether the database was restored from
/// a seed dump and so came with somebody else's timestamp in it.
///
/// A notifier that cannot be read is a trace line and not a failure - `show`
/// reaches DHIS2 and stops there - and it leaves the verdict on the cautious
/// side, which is the side that does not claim analytics is done.
fn analytics_evidence(ctx: &Ctx, session: &Session) -> dhis2::AnalyticsEvidence {
    let ran_here = match session.dhis2.get_json(&dhis2::jobs_path()) {
        Ok(tasks) => dhis2::finished_here(&tasks),
        Err(why) => {
            ctx.out.verbose(&format!(
                "{} could not be read ({why}), so no analytics run is counted as seen here",
                dhis2::jobs_path()
            ));
            false
        }
    };
    // A seed dump is something chaps restores into its own `dhis2_db`; an
    // external DHIS2 came with nothing of chaps', so its timestamp is its own.
    let seeded = session.external().is_none()
        && session
            .project
            .state
            .components
            .dhis2_seed_source()
            .is_some();
    dhis2::analytics_evidence(&session.ready.last_analytics, ran_here, seeded)
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
        record: None,
    };
    ctx.out.emit(&report, || human_report(&report, &ctx.out))
}

/// Put the route where it belongs, and prove the path through it.
fn write_route(ctx: &Ctx, session: &Session) -> Result<RouteReport> {
    if !session.project.state.components.has_chap_core_api() {
        return Err(anyhow::anyhow!(NO_CHAP_CORE));
    }
    let target = session.target();
    let existing = dhis2::route_in(&session.dhis2.get_json(&dhis2::routes_query())?);
    // chap-core's token, when it has one, is what the route has to carry for
    // the app to get past `/health`.
    let token = crate::api::token_for(Some(&session.project.dir));
    let mut action = dhis2::route_action(existing.as_ref(), &target, token.is_some());
    // A route that carries a header may carry an old one, and DHIS2 never
    // shows it, so the only way to know is to ask through the route.
    if action == RouteAction::Keep && token.is_some() && token_refused(&session.dhis2) {
        action = RouteAction::Rewrite(vec![
            "chap-core refused the API token it carried".to_string(),
        ]);
    }
    ctx.out.verbose(&format!("the chap route: {action:?}"));

    let payload = dhis2::route_payload(&target, token.as_deref());
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

    let (verified, answered, token_refused) = verify_route(&session.dhis2, token.is_some());
    // The route is correct whatever chap-core did, so a chap-core that is not
    // answering is said rather than raised: it is `chaps up`'s problem, not
    // this command's, and the report carries `verified: false` for a script.
    if token_refused {
        crate::output::warn(&format!(
            "the `{}` route carries this deployment's API token and chap-core refused it: \
             {answered}; `chaps auth show` says which token chaps has, and chap-core has to be \
             running with the same one",
            dhis2::ROUTE_CODE
        ));
    } else if !verified {
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
        return Err(anyhow::anyhow!(dhis2::allowlist_hint(
            target,
            session.external().is_none()
        )));
    }
    if answer.status == 403 {
        return Err(anyhow::anyhow!(session.dhis2.route_refusal(&answer)));
    }
    Err(session.dhis2.status_error(path, &answer))
}

/// Ask chap-core for its health through the route, the way the app would.
///
/// The check worth making: a row in DHIS2's database proves nothing, and this
/// proves DHIS2 resolved the hostname, was allowed to reach it, and got an
/// answer. The path is the code rather than the id because that is the address
/// the Modeling App itself uses.
///
/// `/health` is one of the paths chap-core leaves open, so with a token on it
/// proves the hostname and nothing about the token. `token_needed` adds a
/// second request at [`crate::status::SERVICES_PATH`], which is behind the
/// token, so a route that would hand the app a 401 is not reported as working.
fn verify_route(client: &Dhis2, token_needed: bool) -> (bool, String, bool) {
    let (verified, answered) = verify_health(client);
    if !verified || !token_needed {
        return (verified, answered, false);
    }
    let path = dhis2::route_run_path(crate::status::SERVICES_PATH);
    match client.send("GET", &path, None) {
        Ok(answer) if answer.is_success() => (true, answered, false),
        Ok(answer) => (
            false,
            format!(
                "{answered} on /health, but {} on {}",
                answer.status_line(),
                crate::status::SERVICES_PATH
            ),
            matches!(answer.status, 401 | 403),
        ),
        Err(err) => (false, err.to_string(), false),
    }
}

/// Whether chap-core turns away a call through the route that needs its token.
///
/// Any other failure is not a token problem and is left to [`verify_route`] to
/// report, so only a 401 or a 403 counts.
fn token_refused(client: &Dhis2) -> bool {
    let path = dhis2::route_run_path(crate::status::SERVICES_PATH);
    client
        .send("GET", &path, None)
        .is_ok_and(|answer| matches!(answer.status, 401 | 403))
}

/// The `/health` half of [`verify_route`].
fn verify_health(client: &Dhis2) -> (bool, String) {
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
        record: None,
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
        record: None,
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
    // One name in the report, whichever spelling each side uses: the App Hub
    // publishes the Modeling App as `Modeling` and an instance lists it under a
    // third name again, so a report that used all three would read as three
    // apps. The App Hub's own is still what the instance's listing is matched
    // on, because that is the spelling the instance got its name from.
    let name = app.name.to_string();
    let published_as = match published.name.trim().is_empty() {
        true => app.name,
        false => published.name.trim(),
    };
    let Some(version) = dhis2::pick_version(&published.versions, &session.ready.version) else {
        return Err(anyhow::anyhow!(
            "the App Hub publishes no version of {name} that DHIS2 {} can run; install it from \
             DHIS2's own App Management page",
            display_version(&session.ready.version)
        ));
    };
    let installed = dhis2::installed_app(listing, published_as);
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
    let mut session = open_session(ctx, &args.common)?;
    let mut skipped = Vec::new();

    let route = write_route(ctx, &session)?;

    // A DHIS2 someone else runs gets the route and nothing more. Installing
    // apps and generating analytics tables change a server chaps does not own,
    // and on a national instance an analytics run takes hours of its CPU; its
    // admin decides, with `chaps dhis2 apps` and `chaps dhis2 analytics`.
    if session.external().is_some() {
        let mut left = Vec::new();
        match chap_apps(&session) {
            Ok(apps) => left.extend(missing_apps(&apps)),
            Err(err) => ctx.out.verbose(&format!("could not list the apps: {err}")),
        }
        let notes = external_left_alone(&left);
        let judgement = match route.verified {
            true => Judgement::Connected,
            false => Judgement::Broken,
        };
        let record = record_connect(ctx, &mut session.project, judgement)?;
        let report = Dhis2Report {
            instance: session.instance(),
            next: match route.verified {
                true => "the Modeling App can reach CHAP once it is installed; open DHIS2 with \
                         `chaps open dhis2`"
                    .to_string(),
                false => "run `chaps dhis2 show` to see what is still missing".to_string(),
            },
            route: Some(route),
            apps: None,
            analytics: None,
            skipped: notes,
            record: Some(record),
        };
        return ctx.out.emit(&report, || human_report(&report, &ctx.out));
    }

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
    // Written before the report is printed, so what the screen says and what
    // `.chaps/components.yaml` holds cannot disagree.
    let record = record_connect(ctx, &mut session.project, judge(&route, apps.as_ref()))?;
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
        record: Some(record),
    };
    ctx.out.emit(&report, || human_report(&report, &ctx.out))?;
    match failed {
        Some(why) => Err(anyhow::anyhow!(why)),
        None => Ok(()),
    }
}

/// The two steps `connect` leaves to the admin of an external DHIS2, with
/// the apps it found missing named, as the `skipped:` lines of its report.
fn external_left_alone(missing: &[String]) -> Vec<String> {
    let apps = match missing.is_empty() {
        true => "installing apps: both are there already".to_string(),
        false => format!(
            "installing apps on a DHIS2 chaps does not run ({}); `chaps dhis2 apps` installs \
             them if its admin agrees",
            missing.join(", ")
        ),
    };
    vec![
        apps,
        "generating analytics tables on a DHIS2 chaps does not run; `chaps dhis2 analytics` \
         starts a run if its admin agrees"
            .to_string(),
    ]
}

/// What one `connect` run is entitled to say about the deployment it ran
/// against.
///
/// Three states rather than two, because a run can also fail to look. Clearing
/// a record is an assertion that something is broken, and an assertion needs
/// evidence: a step this run skipped is not evidence of anything, and a skip is
/// a reported skip everywhere else in chaps rather than a failure.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Judgement {
    /// The route was proved and both apps are in place. Stamp the time.
    Connected,
    /// This run found something wrong - a route nothing answered through, or an
    /// app missing or failed. Forget any earlier record, so the hint comes
    /// back.
    Broken,
    /// This run did not get far enough to have an opinion: the route was proved
    /// but the apps were never attempted, which is `--offline`. Leave the
    /// record exactly as it was found, in either direction.
    Unknown,
}

/// What this run proved about the shape the hint exists to ask for: a route
/// chap-core answered through, and both apps installed.
///
/// **Two of the three steps, and deliberately so.** The hint is there because a
/// DHIS2 beside a CHAP cannot be used at all - the Modeling App redirects to
/// `/get-started` without the route, and there is no CHAP user interface in
/// DHIS2 without the apps. Neither is true of analytics: the app is installed,
/// reaches chap-core and works, and empty `analytics_*` tables are a deployment
/// with no data rather than one that cannot talk to itself. `chaps dhis2 show`
/// reports the analytics question separately and with the evidence it is worth,
/// which is where it belongs, and `chaps dhis2 analytics --no-wait` is a
/// supported shape in which nothing here could ever wait for a finished run.
///
/// The route is judged by `verified` rather than by `outcome`: a row written
/// into DHIS2 proves nothing, and the proxied request proves DHIS2 resolved the
/// hostname, was allowed to reach it and got chap-core's health document back.
/// A route that did not verify is [`Judgement::Broken`] whether or not the apps
/// were attempted, because the failure is this run's own evidence.
///
/// The apps are judged as a set, and only when there was a set to judge. An
/// `--offline` run installs none of them and reports the skip, which says
/// nothing whatever about the apps in DHIS2 - they are very likely the two this
/// command put there - so it is [`Judgement::Unknown`] rather than a verdict of
/// breakage built out of a step that was never run.
fn judge(route: &RouteReport, apps: Option<&AppsReport>) -> Judgement {
    match (route.verified, apps) {
        (false, _) => Judgement::Broken,
        (true, None) => Judgement::Unknown,
        (true, Some(apps)) => {
            let all_there = apps.apps.len() == dhis2::HUB_APPS.len()
                && apps
                    .apps
                    .iter()
                    .all(|app| app.outcome != AppOutcome::Failed);
            match all_there {
                true => Judgement::Connected,
                false => Judgement::Broken,
            }
        }
    }
}

/// Write what this run is worth into `.chaps/components.yaml`, and say which of
/// the three things happened to the file.
///
/// A run that got there stamps the time; a run that found it broken forgets
/// whatever was recorded before. Forgetting is the half worth arguing for: it
/// brings the hint back on a deployment whose connect has stopped working, and
/// the cost of being wrong is one line naming an idempotent command, where the
/// cost of the other mistake is a deployment that reports itself healthy while
/// the Modeling App cannot reach CHAP. That trade only holds when the run
/// actually found something wrong, which is why [`Judgement::Unknown`] writes
/// nothing in either direction: absence of evidence would otherwise make
/// `chaps up` say "chaps has not connected this DHIS2 to CHAP" about a
/// deployment chaps did connect, because of a flag on an unrelated step.
///
/// The file is only written when the value moves, so re-running `connect` on a
/// deployment that was never connected touches nothing.
fn record_connect(ctx: &Ctx, project: &mut Project, judgement: Judgement) -> Result<ConnectRecord> {
    let was = project.state.components.dhis2_connected_at_mut().is_some();
    let at = match (judgement, was) {
        (Judgement::Unknown, _) | (Judgement::Broken, false) => {
            return Ok(ConnectRecord::Unchanged);
        }
        (Judgement::Broken, true) => None,
        (Judgement::Connected, _) => Some(crate::backup::timestamp(crate::backup::now())),
    };
    ctx.out.verbose(&match &at {
        Some(at) => format!("recording `connected_at: {at}` in `.chaps/components.yaml`"),
        None => "clearing `connected_at` in `.chaps/components.yaml`".to_string(),
    });
    let record = match at.is_some() {
        true => ConnectRecord::Recorded,
        false => ConnectRecord::Cleared,
    };
    *project.state.components.dhis2_connected_at_mut() = at;
    project.save()?;
    Ok(record)
}

// ---------------------------------------------------------------------------
// use
// ---------------------------------------------------------------------------

/// How long `use` gives DHIS2 to answer: it asks once, and a DHIS2 that is
/// still migrating is `chaps dhis2 show`'s to wait for.
const USE_PROBE_TIMEOUT: Duration = Duration::from_secs(10);

/// Why an external DHIS2 cannot be recorded beside the component.
const COMPONENT_IS_ON: &str = "the `dhis2` component is on, and it is this deployment's DHIS2; \
     run `chaps components disable dhis2` first, then `chaps dhis2 use` again";

/// What `use` did to the record.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum UseOutcome {
    /// There was no external DHIS2, and now there is one.
    Recorded,
    /// There was one, and its URL or its chap-core URL moved.
    Changed,
    /// What was asked for is what was recorded. Nothing was written.
    Unchanged,
    /// The record was removed.
    Cleared,
    /// Nothing was asked for, so the record was reported and not touched.
    Shown,
}

/// What `use` found when it asked the DHIS2 it recorded.
#[derive(Debug, Clone, Serialize)]
pub struct UseProbe {
    /// Whether [`dhis2::PING_PATH`] answered.
    pub answered: bool,
    /// What it said, or why nothing did.
    pub answer: String,
    /// The credential `chaps dhis2` would send, by what it is and where it was
    /// found; `null` when there is none.
    pub credential: Option<String>,
    /// Whether DHIS2 accepted it. `null` when it was not tried: no
    /// credential, or no DHIS2 answering to try it on.
    pub accepted: Option<bool>,
    /// Why there is no credential, or why DHIS2 did not accept it.
    pub problem: Option<String>,
}

/// What `chaps dhis2 use` did.
#[derive(Debug, Clone, Serialize)]
pub struct UseReport {
    pub outcome: UseOutcome,
    /// The external DHIS2 recorded now, or `null` for none.
    pub external: Option<crate::components::ExternalDhis2>,
    /// The one recorded before, for a change or a clear.
    pub previous: Option<crate::components::ExternalDhis2>,
    /// Where the `chap` route has to point for it.
    pub target: Option<String>,
    /// Whether the `dhis2` component is on, which is what `chaps dhis2` talks
    /// to when nothing is recorded.
    pub component: bool,
    /// What asking it just now found; `null` when there is nothing to ask.
    pub probe: Option<UseProbe>,
    pub notes: Vec<String>,
    pub next: String,
}

/// `chaps dhis2 use` — record a DHIS2 that runs elsewhere, change it, forget
/// it, or say which DHIS2 `chaps dhis2` talks to.
///
/// Every shape that records something also asks: [`dhis2::PING_PATH`] to
/// prove the URL, and one authenticated request to prove the credential. A
/// record that is wrong is still written - the URL can be right for a DHIS2
/// that is down for maintenance - and the report says what did not answer.
pub fn use_external(ctx: &Ctx, args: &Dhis2UseArgs) -> Result<()> {
    let mut project = ctx.project()?;
    let before = project.state.components.dhis2_external.clone();
    let component = project.state.components.dhis2.enabled;

    if args.clear {
        let outcome = match before {
            Some(_) => {
                project.state.components.dhis2_external = None;
                project.save()?;
                UseOutcome::Cleared
            }
            None => UseOutcome::Unchanged,
        };
        let report = UseReport {
            outcome,
            external: None,
            previous: before,
            target: None,
            component,
            probe: None,
            notes: Vec::new(),
            next: match component {
                true => "`chaps dhis2` talks to the `dhis2` component; run `chaps dhis2 show`",
                false => {
                    "run `chaps dhis2 use URL --chap-url URL` to record a DHIS2, or `chaps \
                     components enable dhis2` to deploy one"
                }
            }
            .to_string(),
        };
        return ctx.out.emit(&report, || human_use(&report, &ctx.out));
    }

    let (external, outcome) = match (&args.url, &args.chap_url) {
        (None, None) => (before.clone(), UseOutcome::Shown),
        (url, chap_url) => {
            if component {
                return Err(anyhow::anyhow!(COMPONENT_IS_ON));
            }
            let url = match (url, &before) {
                (Some(url), _) => external_url(url)?,
                (None, Some(before)) => before.url.clone(),
                (None, None) => {
                    return Err(anyhow::anyhow!(
                        "no external DHIS2 is recorded yet, so there is no URL to keep; run \
                         `chaps dhis2 use URL --chap-url URL`"
                    ));
                }
            };
            let chap_url = match (chap_url, &before) {
                (Some(chap_url), _) => external_url(chap_url)?,
                (None, Some(before)) => before.chap_url.clone(),
                (None, None) => {
                    return Err(anyhow::anyhow!(
                        "`--chap-url URL` is needed the first time: the route points at \
                         chap-core as DHIS2 reaches it, which chaps cannot know; run `chaps \
                         dhis2 use {url} --chap-url https://chap.example.org`"
                    ));
                }
            };
            let same = before
                .as_ref()
                .is_some_and(|before| before.url == url && before.chap_url == chap_url);
            let external = crate::components::ExternalDhis2 {
                url,
                chap_url,
                // A connect is a fact about one DHIS2 and one route target;
                // either moving is a DHIS2 no connect has been recorded for.
                connected_at: match same {
                    true => before.as_ref().and_then(|b| b.connected_at.clone()),
                    false => None,
                },
            };
            let outcome = match (&before, same) {
                (None, _) => UseOutcome::Recorded,
                (Some(_), true) => UseOutcome::Unchanged,
                (Some(_), false) => UseOutcome::Changed,
            };
            if outcome != UseOutcome::Unchanged {
                ctx.out
                    .verbose("recording `dhis2-external` in `.chaps/components.yaml`");
                project.state.components.dhis2_external = Some(external.clone());
                project.save()?;
            }
            (Some(external), outcome)
        }
    };

    let Some(recorded) = &external else {
        let report = UseReport {
            outcome,
            external: None,
            previous: None,
            target: None,
            component,
            probe: None,
            notes: Vec::new(),
            next: match component {
                true => "`chaps dhis2` talks to the `dhis2` component; run `chaps dhis2 show`",
                false => {
                    "run `chaps dhis2 use URL --chap-url URL` to record a DHIS2, or `chaps \
                     components enable dhis2` to deploy one"
                }
            }
            .to_string(),
        };
        return ctx.out.emit(&report, || human_use(&report, &ctx.out));
    };

    let probe = probe_external(&project, recorded);
    let mut notes = Vec::new();
    if is_loopback(&recorded.chap_url) {
        notes.push(format!(
            "{} is this machine's own address, and DHIS2 resolves it to its own server; give \
             `--chap-url` the address chap-core is served at on the network",
            recorded.chap_url
        ));
    }
    if !project.state.components.has_chap_core_api() {
        notes.push(NO_CHAP_CORE.to_string());
    }
    let next = match (&probe, &recorded.connected_at) {
        (probe, _) if !probe.answered => {
            "check the URL, then run `chaps dhis2 use` again to ask it".to_string()
        }
        (probe, _) if probe.credential.is_none() => format!(
            "{}, then run `chaps dhis2 connect`",
            dhis2::CREDENTIALS_WAY_OUT
        ),
        (probe, _) if probe.accepted == Some(false) => {
            "fix the credential named above, then run `chaps dhis2 use` again to ask it".to_string()
        }
        (_, None) => "run `chaps dhis2 connect` to point its route at this CHAP".to_string(),
        (_, Some(_)) => "run `chaps dhis2 show` to see what it has".to_string(),
    };
    let report = UseReport {
        outcome,
        target: Some(dhis2::external_route_target(&recorded.chap_url)),
        external: external.clone(),
        previous: before.filter(|_| outcome == UseOutcome::Changed),
        component,
        probe: Some(probe),
        notes,
        next,
    };
    ctx.out.emit(&report, || human_use(&report, &ctx.out))
}

/// A URL as it is recorded: `http://` or `https://` with a host, and without a
/// trailing slash.
fn external_url(raw: &str) -> Result<String> {
    let url = raw.trim().trim_end_matches('/');
    let host = url
        .strip_prefix("https://")
        .or_else(|| url.strip_prefix("http://"))
        .map(|rest| rest.split('/').next().unwrap_or_default())
        .unwrap_or_default();
    if host.is_empty() {
        return Err(anyhow::anyhow!(
            "`{raw}` is not a URL chaps can record; give it with http:// or https://, like \
             `https://dhis2.example.org`"
        ));
    }
    Ok(url.to_string())
}

/// Whether a URL names this machine, which is a different machine for DHIS2.
fn is_loopback(url: &str) -> bool {
    let host = url
        .split("://")
        .nth(1)
        .unwrap_or_default()
        .split(['/', ':'])
        .next()
        .unwrap_or_default();
    host == "localhost" || host.starts_with("127.") || host == "[::1]"
}

/// Ask the recorded DHIS2 whether it answers, and whether it takes the
/// credential `chaps dhis2` would send it.
fn probe_external(project: &Project, external: &crate::components::ExternalDhis2) -> UseProbe {
    let credentials = dhis2::credentials_for(&project.dir, None, false);
    // `send_open` carries no credential, so a placeholder serves for the ping
    // when there is none to carry.
    let client = Dhis2::new(
        &external.url,
        credentials
            .as_ref()
            .ok()
            .cloned()
            .unwrap_or_else(|| dhis2::Credentials::token("", CredentialSource::Default)),
        USE_PROBE_TIMEOUT,
    )
    .external();
    let (answered, answer) = match client.send_open(dhis2::PING_PATH) {
        Ok(answer) if answer.is_success() => (true, format!("answers {}", dhis2::PING_PATH)),
        Ok(answer) => (
            false,
            format!("{} answered {}", dhis2::PING_PATH, answer.status_line()),
        ),
        Err(err) => (false, first_line(&err.to_string())),
    };
    let (credential, accepted, problem) = match credentials {
        Err(why) => (None, None, Some(why.to_string())),
        Ok(credentials) if !answered => (Some(credentials.describe()), None, None),
        Ok(credentials) => {
            let described = credentials.describe();
            match client.send("GET", dhis2::ME_PATH, None) {
                Ok(answer) if answer.is_success() => (Some(described), Some(true), None),
                Ok(answer) => (
                    Some(described),
                    Some(false),
                    Some(client.refusal(&answer).unwrap_or_else(|| {
                        client.status_error(dhis2::ME_PATH, &answer).to_string()
                    })),
                ),
                Err(err) => (Some(described), None, Some(first_line(&err.to_string()))),
            }
        }
    };
    UseProbe {
        answered,
        answer,
        credential,
        accepted,
        problem,
    }
}

// ---------------------------------------------------------------------------
// The human rendering
// ---------------------------------------------------------------------------

/// The line every verb opens with: where DHIS2 is, what it is, and who it was
/// asked as. Never the password, only where it was found.
fn instance_line(instance: &Instance, out: &Out) -> String {
    let what = match instance.external {
        true => format!("external DHIS2 {}", display_version(&instance.version)),
        false => format!("DHIS2 {}", display_version(&instance.version)),
    };
    format!(
        "{} at {}, as {} ({})\n",
        out.value(&what),
        out.value(&instance.url),
        out.value(&format!("`{}`", instance.user)),
        out.dim(&dhis2::describe_credential(
            instance.auth,
            instance.credential_from
        )),
    )
}

/// What `use` prints.
fn human_use(report: &UseReport, out: &Out) -> String {
    let mut text = String::new();
    let headline = match (report.outcome, &report.external, &report.previous) {
        (UseOutcome::Cleared, _, Some(previous)) => {
            format!("forgot the external DHIS2 at {}", out.value(&previous.url))
        }
        (UseOutcome::Unchanged, None, _) => {
            "no external DHIS2 is recorded; nothing to clear".to_string()
        }
        (UseOutcome::Recorded, Some(external), _) => format!(
            "recorded the external DHIS2 at {} in `.chaps/components.yaml`",
            out.value(&external.url)
        ),
        (UseOutcome::Changed, Some(external), _) => format!(
            "changed the external DHIS2 to {} in `.chaps/components.yaml`",
            out.value(&external.url)
        ),
        (UseOutcome::Unchanged, Some(external), _) => format!(
            "the external DHIS2 at {} is already recorded; nothing changed",
            out.value(&external.url)
        ),
        (_, Some(external), _) => format!(
            "`chaps dhis2` talks to the external DHIS2 at {}",
            out.value(&external.url)
        ),
        (_, None, _) => match report.component {
            true => "no external DHIS2 is recorded; `chaps dhis2` talks to the `dhis2` component"
                .to_string(),
            false => "no external DHIS2 is recorded, and the `dhis2` component is off".to_string(),
        },
    };
    text.push_str(&out.backticks(&headline));
    text.push('\n');
    if let (Some(external), Some(probe)) = (&report.external, &report.probe) {
        let dhis2 = match probe.answered {
            true => format!(
                "{} {}",
                out.value(&external.url),
                out.ok(&format!("({})", probe.answer))
            ),
            false => format!(
                "{} {}",
                out.value(&external.url),
                out.warn(&format!("({})", probe.answer))
            ),
        };
        let login = match (&probe.credential, probe.accepted) {
            (None, _) => out.warn("none"),
            (Some(credential), Some(true)) => format!("{credential} {}", out.ok("(accepted)")),
            (Some(credential), Some(false)) => format!("{credential} {}", out.warn("(refused)")),
            (Some(credential), None) => format!("{credential} {}", out.dim("(not tried)")),
        };
        let connected = match &external.connected_at {
            Some(at) => out.value(at),
            None => out.dim("never recorded"),
        };
        text.push_str(&crate::output::fields_with(
            0,
            &[
                ("dhis2", dhis2),
                ("chap-url", out.value(&external.chap_url)),
                (
                    "route",
                    out.dim(report.target.as_deref().unwrap_or_default()),
                ),
                ("login", login),
                ("connected", connected),
            ],
            &|label| out.key(label),
        ));
        if let Some(problem) = &probe.problem {
            text.push_str(&format!(
                "{} {}\n",
                out.dim("problem:"),
                out.backticks(problem)
            ));
        }
    }
    for note in &report.notes {
        text.push_str(&format!("{} {}\n", out.dim("note:"), out.backticks(note)));
    }
    text.push_str(&out.backticks(&report.next));
    text.push('\n');
    text
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
    if let Some(record) = report.record {
        text.push_str(&record_lines(record, out));
    }
    text.push_str(&out.backticks(&report.next));
    text.push('\n');
    text
}

/// What `connect` says about the note it left behind, in the shape the route
/// block uses: what happened, then the indented line that says what it is
/// worth.
///
/// The caveat is printed where the record is, rather than left to the chapter,
/// because this is the one moment a reader could take it for a verdict. It is a
/// note that this command ran; `chaps dhis2 show` is what asks DHIS2.
fn record_lines(record: ConnectRecord, out: &Out) -> String {
    match record {
        ConnectRecord::Unchanged => String::new(),
        ConnectRecord::Recorded => format!(
            "{} in `.chaps/components.yaml`, so `chaps up` and `chaps status` stop asking\n  {}\n",
            out.ok("recorded"),
            out.dim(
                "a note that this ran, not proof the route is still right; \
                 `chaps dhis2 show` asks DHIS2"
            )
        ),
        ConnectRecord::Cleared => format!(
            "{} the earlier `chaps dhis2 connect` from `.chaps/components.yaml`\n  {}\n",
            out.warn("cleared"),
            out.dim("`chaps up` and `chaps status` ask for it again")
        ),
    }
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
            } else if route.ours {
                cell = format!("{cell} {}", out.warn("(nothing answered through it)"));
            }
            cell
        }
    };
    let apps = match report.apps.is_empty() {
        true => out.warn("none"),
        false => report
            .apps
            .iter()
            .map(|app| app_cell(app, out))
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
                analytics_cell(report.analytics, &report.last_analytics, out),
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

/// One of the two apps in the `apps` row: the version, or that it is absent.
fn app_cell(app: &ShownApp, out: &Out) -> String {
    match app.installed {
        true => format!("{} {}", app.name, out.value(&app.version)),
        false => format!("{} {}", app.name, out.warn("not installed")),
    }
}

/// The `analytics` row: what DHIS2 records, and what chaps knows it is worth.
///
/// A timestamp on its own reads as "analytics is done", and on a seeded
/// deployment it is not that at all - it is the dump's. So the row says which
/// of the two it is, and never claims more than chaps checked: a run that
/// finished is a run that finished, which is not the same as tables with rows
/// in them. Nothing here can see a row count; `lastYears` is the reason that
/// distinction is worth keeping, and `docs/dhis2.md` carries it.
fn analytics_cell(evidence: dhis2::AnalyticsEvidence, last: &str, out: &Out) -> String {
    let when = out.value(last.trim());
    match evidence {
        dhis2::AnalyticsEvidence::Never => out.warn("never run"),
        dhis2::AnalyticsEvidence::Recorded => when,
        dhis2::AnalyticsEvidence::RanHere if last.trim().is_empty() => {
            out.ok("a run finished on this deployment")
        }
        dhis2::AnalyticsEvidence::RanHere => {
            format!("{when} {}", out.ok("(a run finished on this deployment)"))
        }
        dhis2::AnalyticsEvidence::Unconfirmed => {
            format!("{when} {}", out.warn("(unconfirmed on a seeded database)"))
        }
    }
}

/// The first line of an error, for a cell that has one line to say it in.
fn first_line(text: &str) -> String {
    text.lines().next().unwrap_or_default().trim().to_string()
}

#[cfg(test)]
mod tests;
