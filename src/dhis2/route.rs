//! The `chap` route: where it has to point, whether the one there does, and
//! what a refusal to write it means.

use super::{
    Dhis2, ROUTE_AUTH_TYPE, ROUTE_AUTHORITY, ROUTE_CODE, ROUTE_NAME, ROUTE_SUFFIX,
    ROUTE_TIMEOUT_SECONDS, ROUTES_PATH, said,
};
use crate::api::Answer;
use serde::Deserialize;

/// One row of DHIS2's Route API.
#[derive(Debug, Clone, Default, PartialEq, Eq, Deserialize)]
pub struct Route {
    #[serde(default)]
    pub id: String,
    #[serde(default)]
    pub name: String,
    #[serde(default)]
    pub code: String,
    #[serde(default)]
    pub url: String,
    #[serde(default)]
    pub disabled: bool,
    #[serde(default)]
    pub authorities: Vec<String>,
    #[serde(default)]
    pub auth: Option<RouteAuth>,
}

/// A route's `auth` as DHIS2 lists it: the type, never the secret.
#[derive(Debug, Clone, Default, PartialEq, Eq, Deserialize)]
pub struct RouteAuth {
    #[serde(default, rename = "type")]
    pub kind: String,
}

/// The fields a listing asks for, so the answer is small and stable.
pub const ROUTE_FIELDS: &str = "id,name,code,url,disabled,authorities,auth";

/// `GET /api/routes` with the fields and no paging.
pub fn routes_query() -> String {
    format!("{ROUTES_PATH}?fields={ROUTE_FIELDS}&paging=false")
}

/// Where the route has to point for the app to reach this deployment's
/// chap-core.
///
/// A hostname DHIS2's own container resolves, which is the compose service
/// alias and never `localhost`: the app's requests are made by DHIS2, from
/// inside the deployment. `root` is `CHAP_ROOT_PATH` when the deployment sets
/// one, because that moves every chap-core route with it.
pub fn route_target(root: &str) -> String {
    format!(
        "{}{}{ROUTE_SUFFIX}",
        crate::open::internal_url(crate::components::Component::ChapCore),
        root.trim_end_matches('/')
    )
}

/// The authority DHIS2 checks before it creates a route, which a superuser has
/// through `ALL` and an ordinary admin role may not: the Sierra Leone demo
/// database's `admin` does not have it.
pub const ROUTE_ADD_AUTHORITY: &str = "F_ROUTE_PUBLIC_ADD";

impl Dhis2 {
    /// The user these requests are made as, in backticks, for a sentence.
    pub fn who(&self) -> String {
        match self.credentials.user() {
            Some(user) => format!("`{user}`"),
            None => "the token's user".to_string(),
        }
    }

    /// What a 403 on writing the `chap` route says: DHIS2's own reason, the
    /// authority the write needs, and the two ways to get it.
    pub fn route_refusal(&self, answer: &Answer) -> String {
        let reason = match said(answer) {
            reason if reason.is_empty() => String::new(),
            reason => format!(", \"{reason}\""),
        };
        format!(
            "DHIS2 refused to write the `{ROUTE_CODE}` route as {} (HTTP 403{reason}): that needs \
             the {ROUTE_ADD_AUTHORITY} authority; add it to one of that user's roles in DHIS2 \
             (Users, User role), or name a superuser with `--user NAME`",
            self.who()
        )
    }
}

/// Where the route has to point for an external DHIS2: chap-core's own URL as
/// that DHIS2 reaches it, recorded by `chaps dhis2 use --chap-url`.
pub fn external_route_target(chap_url: &str) -> String {
    format!("{}{ROUTE_SUFFIX}", chap_url.trim_end_matches('/'))
}

/// The route as DHIS2 is asked to store it.
///
/// Sent on the create and on the repoint alike: `PUT /api/routes/{id}` replaces
/// the row, so one payload is also what repairs a route that was disabled or
/// had lost the authority.
///
/// `token` is chap-core's API token when it has one. It goes in the route's
/// `auth` as an `Authorization: Bearer` header rather than in `headers`,
/// because DHIS2 lists `headers` back to anyone who can read the route and
/// keeps `auth` to itself. Without it every call the Modeling App makes past
/// `/health` answers 401.
pub fn route_payload(target: &str, token: Option<&str>) -> String {
    let mut payload = serde_json::json!({
        "name": ROUTE_NAME,
        "code": ROUTE_CODE,
        "url": target,
        "authorities": [ROUTE_AUTHORITY],
        "headers": {"Content-Type": crate::api::JSON},
        "responseTimeoutSeconds": ROUTE_TIMEOUT_SECONDS,
    });
    if let Some(token) = token {
        payload["auth"] = serde_json::json!({
            "type": ROUTE_AUTH_TYPE,
            "headers": {"Authorization": format!("Bearer {token}")},
        });
    }
    payload.to_string()
}

/// What has to be done about the route that is there, or is not.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RouteAction {
    /// No route with this code: create one.
    Create,
    /// One is there and is wrong. Each reason is a clause a report prints.
    Rewrite(Vec<String>),
    /// One is there and already points at this deployment.
    Keep,
}

/// The route with [`ROUTE_CODE`], out of a `GET /api/routes` answer.
///
/// Read out of the listing rather than fetched at `/api/routes/chap`, because
/// the path segment there is an id and the code is not one. Codes are unique in
/// DHIS2, so there is at most one.
pub fn route_in(listing: &serde_json::Value) -> Option<Route> {
    listing
        .get("routes")
        .and_then(serde_json::Value::as_array)
        .or_else(|| listing.as_array())?
        .iter()
        .filter_map(|row| serde_json::from_value::<Route>(row.clone()).ok())
        .find(|route| route.code == ROUTE_CODE)
}

/// Whether the route that is there is the one this deployment needs.
///
/// The URL is the point, and not the whole of it: a route aimed at the right
/// chap-core but disabled proxies nothing, and one without
/// [`ROUTE_AUTHORITY`] refuses the app's own user. Each of the three is named
/// separately so the report says what was actually wrong. When chap-core
/// wants a token (`token_needed`), a route with no header auth is a fourth: it
/// reaches `/health` and nothing else.
///
/// **The URL is what makes this a repoint rather than a create.** The climate
/// demo dumps ship a `chap` route of their own, aimed at an external CHAP
/// server, with the right code, the right authority and not disabled - so an
/// implementation that created the route only when one was absent would leave
/// the deployment sending its data to a stranger's chap-core, and would look
/// like it had worked.
pub fn route_action(existing: Option<&Route>, target: &str, token_needed: bool) -> RouteAction {
    let Some(route) = existing else {
        return RouteAction::Create;
    };
    let mut reasons = Vec::new();
    if route.url != target {
        reasons.push(format!("it pointed at {}", route.url));
    }
    if route.disabled {
        reasons.push("it was disabled".to_string());
    }
    if !route
        .authorities
        .iter()
        .any(|authority| authority == ROUTE_AUTHORITY)
    {
        reasons.push(format!("it was missing the {ROUTE_AUTHORITY} authority"));
    }
    if token_needed
        && route
            .auth
            .as_ref()
            .is_none_or(|auth| auth.kind != ROUTE_AUTH_TYPE)
    {
        reasons.push("it carried no chap-core API token".to_string());
    }
    if reasons.is_empty() {
        RouteAction::Keep
    } else {
        RouteAction::Rewrite(reasons)
    }
}

/// The path a request is proxied through the route on.
///
/// The code, not the id: this is the address the Modeling App itself uses, so
/// asking here proves the path the app will take rather than one beside it.
pub fn route_run_path(path: &str) -> String {
    format!(
        "{ROUTES_PATH}/{ROUTE_CODE}/run{}",
        if path.starts_with('/') {
            path.to_string()
        } else {
            format!("/{path}")
        }
    )
}

/// Why a route write DHIS2 refused is usually about `dhis.conf`.
///
/// DHIS2 42 and later default `route.remote_servers_allowed` to `https://*` and
/// then refuse an `http://` target. The scaffolded `dhis2/dhis.conf` already
/// permits every http and https target, so this only happens on an instance
/// whose file was narrowed or replaced - which is exactly the failure that
/// otherwise looks like nothing at all.
///
/// A plain `chaps restart dhis2` applies the edit: `dhis.conf` is a bind mount
/// compose does not compare, so `restart` recreates a service whose mounted
/// config is newer than its container itself (see
/// [`crate::commands::docker::edited_configs`]).
///
/// An external DHIS2 has its own `dhis.conf`, which is its operator's and not
/// in this directory, so that one is only named.
pub fn allowlist_hint(target: &str, deployed: bool) -> String {
    let origin = route_origin(target);
    match deployed {
        true => format!(
            "DHIS2 refused the route: version 42 and later only allow the origins \
             `route.remote_servers_allowed` lists, and {origin} has to be one of them; check \
             that line in `dhis2/dhis.conf` and run `chaps restart dhis2`, which recreates it \
             to read the file again"
        ),
        false => format!(
            "DHIS2 refused the route: version 42 and later only allow the origins \
             `route.remote_servers_allowed` lists, and {origin} has to be one of them; add it, \
             with no path (DHIS2 will not start with one), to that line in the `dhis.conf` of \
             the DHIS2 server and restart DHIS2 there"
        ),
    }
}

/// The origin of a route target, which is all `route.remote_servers_allowed`
/// takes: DHIS2 refuses to start when an entry carries a path, and the target
/// always does (`/**`).
///
/// `http://localhost:8700/**` -> `http://localhost:8700`.
pub(super) fn route_origin(target: &str) -> &str {
    let after_scheme = target.find("://").map_or(0, |at| at + 3);
    match target[after_scheme..].find('/') {
        Some(slash) => &target[..after_scheme + slash],
        None => target,
    }
}

/// The half of DHIS2's refusal that is the allowlist and nothing else.
///
/// Lowercased, because [`is_allowlist_refusal`] compares that way.
const ROUTE_NOT_PERMITTED: &str = "route url is not permitted";

/// Whether a refusal is the allowlist rather than anything else.
///
/// **The message is the signal, and `errorCode` is not.** Measured against a
/// live DHIS2 2.42.6, a route write the allowlist refuses answers
///
/// ```json
/// {"httpStatus":"Conflict","httpStatusCode":409,"status":"ERROR",
///  "message":"Route URL is not permitted","errorCode":"E1004"}
/// ```
///
/// and `E1004` reads as the authoritative half until it is looked up. DHIS2
/// defines it as `API query cannot be performed` and hands it to every
/// `ConflictException(String)` there is - 66 of them in the 2.44 source, four of
/// those inside `validateRoute` itself. A malformed URL, a scheme that is
/// neither HTTP nor HTTPS, a placeholder in the origin and a response timeout
/// outside 1..=60 all answer 409 with `E1004`, so keying on the code would send
/// four other failures to `dhis.conf` to hunt for a line that is not their
/// problem. It is a general-purpose code, not this one's.
///
/// The message is the specific half, and it is a literal in DHIS2's own source
/// rather than a translated string: 2.42 says exactly `Route URL is not
/// permitted`, later versions append `. Ask your DHIS2 server administrator to
/// allow it`, and what is matched here is the part both carry - unchanged since
/// the commit that added the allowlist.
///
/// Nothing corroborates it, deliberately. Requiring `E1004` as well, or a 409,
/// could only ever stop the good message appearing on an instance that words its
/// answer slightly differently - and the bug being fixed here is precisely that
/// the good message never appeared. The phrase is narrow enough alone: the three
/// this used to guess at - `remote server`, `not allowed`, `allowlist` - matched
/// no DHIS2 that has ever shipped, and `not allowed` would have matched plenty
/// that is not this.
pub fn is_allowlist_refusal(answer: &Answer) -> bool {
    said(answer)
        .to_ascii_lowercase()
        .contains(ROUTE_NOT_PERMITTED)
}
