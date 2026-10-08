//! `varde dhis2 route`: the `chap` route, written and proved through.

use super::*;

/// `varde dhis2 route` — create the `chap` route, or repoint the one that is
/// there.
pub fn route(ctx: &Ctx, args: &Dhis2RouteArgs) -> Result<()> {
    let session = open_session(ctx, &args.common)?;
    let route = write_route(ctx, &session)?;
    let report = Dhis2Report {
        instance: session.instance(),
        next: "run `varde dhis2 apps` next, or `varde dhis2 show` to see what is still missing"
            .to_string(),
        route: Some(route),
        apps: None,
        analytics: None,
        skipped: Vec::new(),
        record: None,
        configured: None,
    };
    ctx.out
        .report(&report, |lines| report_summary(&report, lines))
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

    // A chap-core that does not answer is a warning in the report, not an
    // error: the route is correct whatever chap-core did.
    let proof = verify_route(&session.dhis2, token.is_some());
    let way_out = match proof.verified || proof.token_refused {
        true => String::new(),
        false => unanswered_way_out(session, &proof, &target),
    };
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
        verified: proof.verified,
        answered: proof.answered,
        token_refused: proof.token_refused,
        way_out,
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

/// What a request through the route that got no answer at all says.
///
/// DHIS2 answered the route write a moment ago, so a proxied request that
/// gets no answer is about chap-core behind the route, not about DHIS2. The
/// client's own sentence for an unreachable DHIS2 names `varde status` for
/// the DHIS2 container, which sends the reader to the wrong service.
pub(super) fn proxied_failure(err: &anyhow::Error) -> String {
    match err.downcast_ref::<crate::error::ChapError>() {
        Some(crate::error::ChapError::Dhis2Unreachable { reason, .. }) => {
            let lower = reason.to_lowercase();
            match lower.contains("timeout") || lower.contains("timed out") {
                true => "the request through DHIS2 timed out".to_string(),
                false => format!("the request through DHIS2 failed: {reason}"),
            }
        }
        _ => err.to_string(),
    }
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
pub(super) fn verify_route(client: &Dhis2, token_needed: bool) -> RouteProof {
    let health = verify_health(client);
    if !health.verified || !token_needed {
        return health;
    }
    let path = dhis2::route_run_path(crate::status::SERVICES_PATH);
    match client.send("GET", &path, None) {
        Ok(answer) if answer.is_success() => health,
        Ok(answer) => RouteProof {
            answered: format!(
                "{} on /health, but {} on {}",
                health.answered,
                answer.status_line(),
                crate::status::SERVICES_PATH
            ),
            token_refused: matches!(answer.status, 401 | 403),
            ..RouteProof::failed(String::new())
        },
        Err(err) => RouteProof::failed(proxied_failure(&err)),
    }
}

/// What the request proxied through the route found.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct RouteProof {
    /// Whether chap-core answered through the route.
    pub verified: bool,
    /// What chap-core said, or why nothing did.
    pub answered: String,
    /// Whether chap-core refused the API token the route carries.
    pub token_refused: bool,
    /// Whether DHIS2 itself answered 503 with an empty body. That is what
    /// DHIS2 does for a route whose origin `route.remote_servers_allowed`
    /// does not allow, at the time of the proxied request.
    pub empty_503: bool,
}

impl RouteProof {
    fn failed(answered: String) -> RouteProof {
        RouteProof {
            verified: false,
            answered,
            token_refused: false,
            empty_503: false,
        }
    }
}

/// The way out for a route that nothing answered through, after the answer.
///
/// varde asks chap-core directly first, at the address this machine uses.
/// If chap-core answers there, the problem is between DHIS2 and chap-core,
/// and the clause names that: the allowlist for an empty 503, and otherwise
/// the address DHIS2 uses for chap-core. If chap-core does not answer here
/// either, `varde status` is the next step, as before.
pub(super) fn unanswered_way_out(session: &Session, proof: &RouteProof, target: &str) -> String {
    let project = &session.project;
    let api = crate::api::Api::new(
        &project.api_base(),
        crate::api::token_for(Some(&project.dir)),
        Duration::from_secs(5),
    );
    let chap_answers = api
        .send("GET", crate::status::HEALTH_PATH, None)
        .is_ok_and(|answer| answer.is_success());
    way_out_for(
        proof,
        chap_answers.then(|| project.api_url()),
        session
            .external()
            .map(|external| external.chap_url.as_str()),
        target,
    )
}

/// [`unanswered_way_out`] without the request: `answers_at` is where
/// chap-core answered on this machine, `external_chap_url` the `--chap-url`
/// of an external DHIS2.
pub(super) fn way_out_for(
    proof: &RouteProof,
    answers_at: Option<String>,
    external_chap_url: Option<&str>,
    target: &str,
) -> String {
    let Some(at) = answers_at else {
        return "run `varde status` to see whether chap-core is up".to_string();
    };
    let origin = dhis2::route_origin(target);
    match (proof.empty_503, external_chap_url) {
        (true, Some(_)) => format!(
            "chap-core answers at {at}, so the DHIS2 allowlist may refuse {origin}; add it to \
             `route.remote_servers_allowed` in the `dhis.conf` of the DHIS2 server"
        ),
        (true, None) => format!(
            "chap-core answers at {at}, so the DHIS2 allowlist may refuse {origin}; add it to \
             `route.remote_servers_allowed` in `dhis2/dhis.conf` and run `varde restart dhis2`"
        ),
        (false, Some(chap_url)) => format!(
            "chap-core answers at {at}, so DHIS2 may not reach it at {chap_url}; give the \
             address DHIS2 reaches chap-core at with `varde dhis2 use --chap-url URL`"
        ),
        (false, None) => format!(
            "chap-core answers at {at}, so DHIS2 may not reach it at {origin}; \
             `varde logs dhis2` may say why"
        ),
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
fn verify_health(client: &Dhis2) -> RouteProof {
    let path = dhis2::route_run_path(crate::status::HEALTH_PATH);
    let answer = match client.send("GET", &path, None) {
        Ok(answer) => answer,
        Err(err) => return RouteProof::failed(proxied_failure(&err)),
    };
    if !answer.is_success() {
        // The status line alone where DHIS2's message is the reason phrase
        // again: "HTTP 502 Bad Gateway: Bad Gateway" says it twice.
        let said = dhis2::said(&answer);
        let answered = match said.is_empty() || answer.status_line().ends_with(&said) {
            true => answer.status_line(),
            false => format!("{}: {said}", answer.status_line()),
        };
        return RouteProof {
            empty_503: answer.status == 503 && answer.text().trim().is_empty(),
            ..RouteProof::failed(answered)
        };
    }
    match answer.json().and_then(|body| {
        body.get("message")
            .or_else(|| body.get("status"))
            .and_then(serde_json::Value::as_str)
            .map(str::to_string)
    }) {
        Some(message) => RouteProof {
            verified: true,
            ..RouteProof::failed(message)
        },
        // A 200 that is not chap-core's health document is somebody else on
        // that hostname, which is exactly the thing this route gets wrong.
        None => RouteProof::failed(format!(
            "a {} that is not chap-core's health document",
            answer.body_description()
        )),
    }
}
