//! `chaps dhis2 route`: the `chap` route, written and proved through.

use super::*;

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
pub(super) fn write_route(ctx: &Ctx, session: &Session) -> Result<RouteReport> {
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
pub(super) fn verify_route(client: &Dhis2, token_needed: bool) -> (bool, String, bool) {
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
