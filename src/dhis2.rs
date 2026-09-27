//! DHIS2's Web API, and the three things that let the Modeling App reach CHAP.
//!
//! Nothing in a CHAP deployment talks to DHIS2 and nothing in DHIS2 talks to
//! chap-core. The only thing that talks to both is the Modeling App running in
//! a browser, and it reaches chap-core through **DHIS2's Route API**: a `Route`
//! row inside DHIS2 with `code: "chap"`, which DHIS2 then reverse-proxies under
//! `/api/routes/chap/run/`. So the work of connecting the two is three
//! requests to DHIS2 and none to chap-core:
//!
//! - the route, or the app redirects to `/get-started`;
//! - the analytics tables, or the app has nothing to send;
//! - the apps themselves, installed from the App Hub, or there is no CHAP user
//!   interface at all.
//!
//! This is a second HTTP client rather than a second caller of
//! [`crate::api`]. That one is chap-core's: it sends `Authorization: Bearer`,
//! and its transport failure is the sentence `chaps status` prints about
//! chap-core. DHIS2 takes HTTP Basic, answers a `WebMessage` rather than
//! FastAPI's `{"detail": ...}`, and a DHIS2 that is not answering is a
//! different sentence with a different command at the end of it. The response
//! shape is shared - [`crate::api::Answer`] - because a status, a body and a
//! content type are the same three things whoever sent them.
//!
//! The password is never printed, never traced and never returned by
//! [`Credentials`]'s `Debug`. `-v` narrates the request line and says that an
//! `Authorization: Basic` header went out, exactly as [`crate::api`] does for
//! the token.

use crate::api::Answer;
use crate::error::{ChapError, Result};
use serde::{Deserialize, Serialize};
use std::path::Path;
use std::time::{Duration, Instant};

/// The one route DHIS2 answers without credentials, and what the wait polls.
pub const PING_PATH: &str = crate::status::DHIS2_PING_PATH;

/// Where DHIS2 reports its own version, behind authentication.
pub const SYSTEM_INFO_PATH: &str = "/api/system/info";

/// The Route API collection.
pub const ROUTES_PATH: &str = "/api/routes";

/// The `code` the Modeling App looks the route up by. Not a name and not an
/// id: the app asks for `/api/routes/chap/run/...` and nothing else will do.
pub const ROUTE_CODE: &str = "chap";

/// The route's `name`, which is only what DHIS2's own maintenance app shows.
pub const ROUTE_NAME: &str = "Chap Modeling App";

/// The authority a DHIS2 user needs to be allowed through the route.
pub const ROUTE_AUTHORITY: &str = "F_CHAP_MODELING_APP";

/// What a route URL has to end in for DHIS2 to proxy paths under it.
///
/// Without it the route proxies exactly one path, so every request the app
/// makes past `/health` answers 404.
pub const ROUTE_SUFFIX: &str = "/**";

/// Seconds DHIS2 waits for chap-core to answer through the route.
///
/// chap-core answers its own endpoints in milliseconds; what takes time there
/// is a job, and a job is started and polled rather than waited on.
pub const ROUTE_TIMEOUT_SECONDS: u32 = 30;

/// Analytics generation, with tracked entities left out.
///
/// **No `lastYears`.** The parameter looks like an optimisation and is a trap:
/// measured on the climate demo, `&lastYears=8` produced zero rows in every
/// analytics table while reporting success in 16.9 seconds, where the same run
/// without it wrote 146,129 rows into `analytics_2024` in 15.7. A silent
/// success that populates nothing leaves the Modeling App with no data and
/// nothing on screen explaining why, so chaps never sends it.
pub const ANALYTICS_PATH: &str = "/api/resourceTables/analytics?skipTrackedEntities=true";

/// The job type analytics runs under, which is also its notifier's key.
pub const ANALYTICS_JOB_TYPE: &str = "ANALYTICS_TABLE";

/// Where a job's notifications are read from.
pub const TASKS_PATH: &str = "/api/system/tasks";

/// The installed apps of an instance.
pub const APPS_PATH: &str = "/api/apps";

/// Server-side installation from the App Hub: `POST /api/appHub/{versionId}`.
///
/// DHIS2 downloads the app itself, so there is nothing to fetch here and no
/// multipart upload to build - but it does mean the instance needs to reach
/// the App Hub, which is why this is the one step `--offline` refuses.
pub const APP_HUB_INSTALL_PATH: &str = "/api/appHub";

/// The variable the App Hub base URL is moved with, for the tests.
pub const APP_HUB_VAR: &str = "CHAPS_APP_HUB";

/// The public App Hub, unless [`APP_HUB_VAR`] points somewhere else.
pub const DEFAULT_APP_HUB: &str = "https://apps.dhis2.org/api/v1/apps";

/// `User-Agent` sent with every request, matching the rest of the CLI.
const USER_AGENT: &str = concat!("chaps-cli/", env!("CARGO_PKG_VERSION"));

/// How long one request may take, all of connect, send and receive.
pub const DEFAULT_TIMEOUT: Duration = Duration::from_secs(30);

/// How long `POST /api/appHub/{versionId}` may take.
///
/// Longer than the rest: DHIS2 downloads the app from the App Hub inside that
/// request, so it is a transfer rather than a query.
pub const INSTALL_TIMEOUT: Duration = Duration::from_secs(300);

/// How long to wait for DHIS2's API by default: twenty minutes.
///
/// DHIS2 migrates its whole schema before it serves a request, and a seeded
/// deployment restores the dump before that; under emulation the two together
/// are a quarter of an hour. The reference deployment waits exactly this long,
/// and a shorter default would turn a slow first start into a failure.
pub const DEFAULT_API_WAIT: u64 = 1200;

/// How long to wait for an analytics run by default: one hour.
///
/// Tens of seconds on demo data and much longer on real data, and a run that
/// is cut short is a set of half-populated tables.
pub const DEFAULT_ANALYTICS_TIMEOUT: u64 = 3600;

/// How often the waits ask again.
pub const POLL_INTERVAL: Duration = Duration::from_secs(5);

/// The `.env` variable naming the DHIS2 user chaps authenticates as.
///
/// Only chaps reads it: it is not passed to any container, so it never reaches
/// a compose file or the DHIS2 process.
pub const ADMIN_USERNAME_ENV_VAR: &str = "DHIS2_ADMIN_USERNAME";

/// The `.env` variable holding that user's password. Also chaps' alone.
pub const ADMIN_PASSWORD_ENV_VAR: &str = "DHIS2_ADMIN_PASSWORD";

/// The environment variable a password is read from when `.env` sets none.
///
/// For an operator who will not have the password on disk: exporting it for
/// one shell is the way to run these commands without writing it down.
pub const PASSWORD_ENV_VAR: &str = "CHAPS_DHIS2_PASSWORD";

/// The user a DHIS2 with no `.env` line is asked as.
pub const DEFAULT_USERNAME: &str = "admin";

/// That user's password on a DHIS2 chaps deployed.
///
/// The same two words whichever way the database was created: the published
/// demo dumps ship this user, and a Flyway-bootstrapped empty database gets it
/// from `DefaultAdminUserPopulator`, which has it compiled in. It is a default,
/// not a secret - every DHIS2 tutorial in the world prints it - and an instance
/// whose password has been changed says so with [`ADMIN_PASSWORD_ENV_VAR`].
pub const DEFAULT_PASSWORD: &str = "district";

/// The two apps a CHAP deployment wants, in the order they are installed.
pub const HUB_APPS: &[HubAppRef] = &[
    HubAppRef {
        id: "a29851f9-82a7-4ecd-8b2c-58e0f220bc75",
        name: "Modeling App",
        label: "the Modeling App",
        what: "the CHAP user interface inside DHIS2",
    },
    HubAppRef {
        id: "effb986c-a3c7-485e-a2f6-5e54ff9df7c3",
        name: "DHIS2 Climate App",
        label: "the Climate App",
        what: "imports climate data into DHIS2 through Google Earth Engine",
    },
];

/// One app chaps installs, as the App Hub identifies it.
///
/// The id is the App Hub's own, and the version to install is resolved from it
/// rather than pinned: the App Hub says which versions exist and what each one
/// needs of DHIS2, and a pin here would go stale in the binary.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct HubAppRef {
    pub id: &'static str,
    /// The App Hub's own name, which is what an installed app is matched on
    /// when there is no App Hub to ask - `chaps dhis2 show` reaches only DHIS2.
    pub name: &'static str,
    /// What a report calls it, in the words a sentence needs.
    pub label: &'static str,
    /// What it is for, for the line that explains why it is being installed.
    pub what: &'static str,
}

// ---------------------------------------------------------------------------
// Credentials
// ---------------------------------------------------------------------------

/// Where the password came from, which is all a report may say about it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum PasswordSource {
    /// An active `DHIS2_ADMIN_PASSWORD=` line in this deployment's `.env`.
    EnvFile,
    /// [`PASSWORD_ENV_VAR`] in the environment this command ran in.
    Environment,
    /// Neither, so [`DEFAULT_PASSWORD`] - which is what a seeded dump and an
    /// empty database both give.
    Default,
}

impl PasswordSource {
    /// The phrase a report names it with.
    pub fn describe(self) -> &'static str {
        match self {
            PasswordSource::EnvFile => "from `.env`",
            PasswordSource::Environment => "from CHAPS_DHIS2_PASSWORD",
            PasswordSource::Default => "the DHIS2 default",
        }
    }
}

/// A DHIS2 user and password, and where the password was found.
///
/// `Debug` is written by hand: a derived one would put the password into every
/// `-d` line and every panic message that carries this value.
#[derive(Clone)]
pub struct Credentials {
    pub user: String,
    password: String,
    pub source: PasswordSource,
}

impl std::fmt::Debug for Credentials {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Credentials")
            .field("user", &self.user)
            .field("password", &"<hidden>")
            .field("source", &self.source)
            .finish()
    }
}

impl Credentials {
    /// A pair, however they were arrived at.
    pub fn new(user: &str, password: &str, source: PasswordSource) -> Credentials {
        Credentials {
            user: user.to_string(),
            password: password.to_string(),
            source,
        }
    }

    /// The `Authorization` header value, which is the only thing that reads
    /// the password.
    pub fn header(&self) -> String {
        format!(
            "Basic {}",
            base64(format!("{}:{}", self.user, self.password).as_bytes())
        )
    }
}

/// The credentials for this deployment's DHIS2.
///
/// `.env` first, because that is the deployment's own answer and the file every
/// other secret of it lives in; then [`PASSWORD_ENV_VAR`], for an operator who
/// will not keep it on disk; then the DHIS2 default. The same order
/// [`crate::api::token_for`] uses for chap-core's token, so one CLI has one
/// rule.
///
/// `user` overrides the username for one run - a username is not a secret, so
/// it can be a flag; a password on a command line would be in the shell history
/// and in `ps`, so there is no flag for that.
pub fn credentials_for(project_dir: &Path, user: Option<&str>) -> Credentials {
    let body =
        std::fs::read_to_string(project_dir.join(crate::project::ENV_FILE)).unwrap_or_default();
    credentials_of(&body, user, std::env::var(PASSWORD_ENV_VAR).ok().as_deref())
}

/// [`credentials_for`]'s rule, with both sources handed in.
pub fn credentials_of(env_body: &str, user: Option<&str>, from_env: Option<&str>) -> Credentials {
    let user = user
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_string)
        .or_else(|| crate::dotenv::non_empty(env_body, ADMIN_USERNAME_ENV_VAR))
        .unwrap_or_else(|| DEFAULT_USERNAME.to_string());
    let (password, source) = match crate::dotenv::non_empty(env_body, ADMIN_PASSWORD_ENV_VAR) {
        Some(password) => (password, PasswordSource::EnvFile),
        None => match from_env.map(str::trim).filter(|value| !value.is_empty()) {
            Some(password) => (password.to_string(), PasswordSource::Environment),
            None => (DEFAULT_PASSWORD.to_string(), PasswordSource::Default),
        },
    };
    Credentials::new(&user, &password, source)
}

// ---------------------------------------------------------------------------
// The client
// ---------------------------------------------------------------------------

/// One DHIS2 instance, ready to be asked.
#[derive(Debug, Clone)]
pub struct Dhis2 {
    base: String,
    credentials: Credentials,
    timeout: Duration,
}

impl Dhis2 {
    /// A client for `base`, authenticating as `credentials`.
    pub fn new(base: &str, credentials: Credentials, timeout: Duration) -> Dhis2 {
        Dhis2 {
            base: base.trim_end_matches('/').to_string(),
            credentials,
            timeout,
        }
    }

    /// The same instance and user, with a different request timeout.
    pub fn with_timeout(&self, timeout: Duration) -> Dhis2 {
        Dhis2 {
            timeout,
            ..self.clone()
        }
    }

    /// The base URL, without its trailing slash.
    pub fn base(&self) -> &str {
        &self.base
    }

    /// The user requests are sent as.
    pub fn user(&self) -> &str {
        &self.credentials.user
    }

    /// Where the password came from, for the line that says so.
    pub fn password_source(&self) -> PasswordSource {
        self.credentials.source
    }

    /// The full URL `path` is asked at.
    pub fn url(&self, path: &str) -> String {
        if path.starts_with('/') {
            format!("{}{path}", self.base)
        } else {
            format!("{}/{path}", self.base)
        }
    }

    /// `method path` with the credentials and an optional JSON body.
    pub fn send(&self, method: &str, path: &str, body: Option<&str>) -> Result<Answer> {
        self.request(method, path, body, true)
    }

    /// `GET path` without credentials, for [`PING_PATH`].
    ///
    /// The one route DHIS2 answers to anybody, and therefore the only honest
    /// probe for "is the API layer up": a 401 from it would mean something in
    /// front of DHIS2 is asking as well.
    pub fn send_open(&self, path: &str) -> Result<Answer> {
        self.request("GET", path, None, false)
    }

    /// `GET path` as JSON, where anything but a 2xx JSON body is an error.
    pub fn get_json(&self, path: &str) -> Result<serde_json::Value> {
        let answer = self.send("GET", path, None)?;
        if !answer.is_success() {
            return Err(self.status_error(path, &answer));
        }
        answer.json().ok_or_else(|| {
            anyhow::anyhow!(
                "{} answered {} with {}, which is not JSON",
                self.url(path),
                answer.status_line(),
                answer.body_description()
            )
        })
    }

    fn request(
        &self,
        method: &str,
        path: &str,
        body: Option<&str>,
        authenticated: bool,
    ) -> Result<Answer> {
        let url = self.url(path);
        let agent = ureq::Agent::new_with_config(
            ureq::Agent::config_builder()
                .timeout_global(Some(self.timeout))
                .user_agent(USER_AGENT)
                // A status code is an answer: DHIS2 says what it did not like
                // in the body of the 409 or the 401, and dropping it would be
                // dropping the message.
                .http_status_as_error(false)
                .build(),
        );

        let mut request = ureq::http::Request::builder().method(method).uri(&url);
        crate::output::verbose(&format!("{method} {url}"));
        if authenticated {
            request = request.header("Authorization", self.credentials.header());
            // Never the value, and never the password anywhere near a trace
            // line: they end up in bug reports.
            crate::output::verbose(&format!(
                "  Authorization: Basic <{} and its password>",
                self.credentials.user
            ));
        }
        // DHIS2 answers HTML to a request that does not ask for JSON.
        request = request.header("Accept", crate::api::JSON);
        if body.is_some() {
            request = request.header("Content-Type", crate::api::JSON);
        }
        if let Some(body) = body {
            crate::output::debug(&format!("  {}", crate::output::trace_body(body)));
        }

        let request = request
            .body(body.unwrap_or_default().as_bytes().to_vec())
            .map_err(|e| ChapError::Usage(format!("{method} {url} is not a valid request: {e}")))?;

        let started = Instant::now();
        let mut response = agent.run(request).map_err(|e| {
            crate::output::verbose(&format!(
                "{method} {url} -> failed in {}ms",
                started.elapsed().as_millis()
            ));
            ChapError::Dhis2Unreachable {
                url: self.base.clone(),
                reason: e.to_string(),
            }
        })?;
        let status = response.status();
        crate::output::verbose(&format!(
            "{method} {url} -> {} in {}ms",
            status.as_u16(),
            started.elapsed().as_millis()
        ));
        let content_type = response
            .headers()
            .get("content-type")
            .and_then(|v| v.to_str().ok())
            .unwrap_or_default()
            .to_string();
        let body = response
            .body_mut()
            .read_to_vec()
            .map_err(|e| anyhow::anyhow!("reading the response body from {url}: {e}"))?;
        crate::output::debug(&format!(
            "  {}",
            crate::output::trace_body(&String::from_utf8_lossy(&body))
        ));
        Ok(Answer {
            status: status.as_u16(),
            reason: status.canonical_reason().unwrap_or_default().to_string(),
            content_type,
            body,
        })
    }

    /// The error for a non-2xx answer to a request that had to work, with the
    /// way out for the two statuses that have one.
    pub fn status_error(&self, path: &str, answer: &Answer) -> anyhow::Error {
        if let Some(text) = self.refusal(answer) {
            return anyhow::anyhow!(text);
        }
        let said = said(answer);
        let where_ = self.url(path);
        if said.is_empty() {
            anyhow::anyhow!("{where_} answered {}", answer.status_line())
        } else {
            anyhow::anyhow!("{where_} answered {}: {said}", answer.status_line())
        }
    }

    /// The sentence a 401 or a 403 deserves, naming the file to change.
    ///
    /// The two are different problems: a 401 is the wrong password, a 403 is
    /// the right password on a user DHIS2 will not let do this. Neither ever
    /// prints the password.
    pub fn refusal(&self, answer: &Answer) -> Option<String> {
        match answer.status {
            401 => Some(format!(
                "DHIS2 at {} did not accept the credentials for `{}` ({}); set \
                 `{ADMIN_USERNAME_ENV_VAR}` and `{ADMIN_PASSWORD_ENV_VAR}` in `.env`, or export \
                 `{PASSWORD_ENV_VAR}`",
                self.base,
                self.credentials.user,
                self.credentials.source.describe()
            )),
            403 => Some(format!(
                "DHIS2 refused the request as `{}` (HTTP 403): that user is authenticated but not \
                 allowed to do this, and a DHIS2 superuser is; name another with `--user NAME`",
                self.credentials.user
            )),
            _ => None,
        }
    }

    /// Wait until [`PING_PATH`] answers, then report what DHIS2 says it is.
    ///
    /// Two questions rather than one, because the container being up says
    /// neither: Tomcat serves pages while every `/api/*` request 404s, which is
    /// what [`PING_PATH`] settles, and a password that is wrong only shows on a
    /// request that carries one, which is what [`SYSTEM_INFO_PATH`] settles.
    ///
    /// `waiting` is called once, with the deadline, the first time the API has
    /// not answered: a command that is about to sit here for twenty minutes has
    /// to say so, and one that finds DHIS2 ready says nothing at all.
    pub fn wait_for_api(
        &self,
        wait: Duration,
        interval: Duration,
        waiting: &mut impl FnMut(Duration),
    ) -> Result<Ready> {
        let started = Instant::now();
        let until = started + wait;
        let mut said = false;
        loop {
            match self.send_open(PING_PATH) {
                Ok(answer) if answer.is_success() => break,
                Ok(answer) => crate::output::verbose(&format!(
                    "{} answered {}",
                    self.url(PING_PATH),
                    answer.status_line()
                )),
                Err(err) => crate::output::verbose(&format!("{err}")),
            }
            if !said {
                said = true;
                waiting(wait);
            }
            if Instant::now() + interval >= until {
                return Err(anyhow::anyhow!(
                    "DHIS2 at {} did not answer {PING_PATH} within {}; `chaps logs dhis2` is \
                     where the migration shows, and `--wait SECONDS` waits longer",
                    self.base,
                    crate::output::human_age(wait)
                ));
            }
            std::thread::sleep(interval);
        }
        let answer = self.send("GET", SYSTEM_INFO_PATH, None)?;
        if !answer.is_success() {
            return Err(self.status_error(SYSTEM_INFO_PATH, &answer));
        }
        let info = answer.json().unwrap_or(serde_json::Value::Null);
        Ok(Ready {
            version: text_at(&info, "version"),
            last_analytics: text_at(&info, "lastAnalyticsTableSuccess"),
            waited: started.elapsed(),
        })
    }
}

/// What an instance said about itself once it was ready.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Ready {
    /// DHIS2's own version string, `2.42.6` and the like. Empty when the
    /// answer carried none.
    pub version: String,
    /// When analytics last succeeded, as DHIS2 records it. Empty for an
    /// instance that has never run it.
    pub last_analytics: String,
    /// How long the wait took, which is the fact a slow first start needs.
    pub waited: Duration,
}

/// A string field of a JSON object, or `""`.
fn text_at(value: &serde_json::Value, key: &str) -> String {
    value
        .get(key)
        .and_then(serde_json::Value::as_str)
        .unwrap_or_default()
        .to_string()
}

/// What DHIS2 said went wrong: its `WebMessage`, else the body.
///
/// Every DHIS2 error carries `message`, and the interesting ones carry
/// `devMessage` or a `response.message` under it as well. Anything that is not
/// JSON is handed back trimmed and cut, because a Tomcat error page in an error
/// line helps nobody.
pub fn said(answer: &Answer) -> String {
    if let Some(value) = answer.json() {
        for key in ["message", "devMessage", "description"] {
            if let Some(text) = value.get(key).and_then(serde_json::Value::as_str)
                && !text.trim().is_empty()
            {
                return text.trim().to_string();
            }
        }
    }
    let text = answer.text();
    let text = text.trim();
    // A body that carries nothing is nothing to say: "answered HTTP 404 Not
    // Found: {}" is the status line plus noise.
    if matches!(text, "{}" | "[]" | "null") {
        return String::new();
    }
    if text.chars().count() > 200 {
        let cut: String = text.chars().take(200).collect();
        format!("{cut}...")
    } else {
        text.to_string()
    }
}

// ---------------------------------------------------------------------------
// The route
// ---------------------------------------------------------------------------

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
}

/// The fields a listing asks for, so the answer is small and stable.
pub const ROUTE_FIELDS: &str = "id,name,code,url,disabled,authorities";

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

/// The route as DHIS2 is asked to store it.
///
/// Sent on the create and on the repoint alike: `PUT /api/routes/{id}` replaces
/// the row, so one payload is also what repairs a route that was disabled or
/// had lost the authority.
pub fn route_payload(target: &str) -> String {
    serde_json::json!({
        "name": ROUTE_NAME,
        "code": ROUTE_CODE,
        "url": target,
        "authorities": [ROUTE_AUTHORITY],
        "headers": {"Content-Type": crate::api::JSON},
        "responseTimeoutSeconds": ROUTE_TIMEOUT_SECONDS,
    })
    .to_string()
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
/// separately so the report says what was actually wrong.
///
/// **The URL is what makes this a repoint rather than a create.** The climate
/// demo dumps ship a `chap` route of their own, aimed at an external CHAP
/// server, with the right code, the right authority and not disabled - so an
/// implementation that created the route only when one was absent would leave
/// the deployment sending its data to a stranger's chap-core, and would look
/// like it had worked.
pub fn route_action(existing: Option<&Route>, target: &str) -> RouteAction {
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
/// permits `http://chap:8000`, so this only happens on an instance whose file
/// was narrowed or replaced - which is exactly the failure that otherwise looks
/// like nothing at all.
pub fn allowlist_hint(target: &str) -> String {
    format!(
        "DHIS2 refused the route: version 42 and later only allow the targets \
         `route.remote_servers_allowed` lists, and {target} has to be one of them; check that \
         line in `dhis2/dhis.conf` and run `chaps restart dhis2`"
    )
}

/// Whether a refusal reads like the allowlist rather than anything else.
pub fn is_allowlist_refusal(answer: &Answer) -> bool {
    let text = said(answer).to_ascii_lowercase();
    text.contains("remote server") || text.contains("not allowed") || text.contains("allowlist")
}

// ---------------------------------------------------------------------------
// Analytics
// ---------------------------------------------------------------------------

/// Where one job's notifications are read.
pub fn job_path(job: &str) -> String {
    format!(
        "{TASKS_PATH}/{ANALYTICS_JOB_TYPE}/{}",
        crate::api::encode(job)
    )
}

/// Where every analytics job's notifications are read, newest run included.
pub fn jobs_path() -> String {
    format!("{TASKS_PATH}/{ANALYTICS_JOB_TYPE}")
}

/// How far a job has got, as its notifications say.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Progress {
    /// Still going. The message is the latest step it announced.
    Running(String),
    /// Finished, and nothing said it failed.
    Done(String),
    /// A notification came in at `ERROR` level.
    Failed(String),
}

impl Progress {
    /// The step or the reason, whichever this is.
    pub fn message(&self) -> &str {
        match self {
            Progress::Running(text) | Progress::Done(text) | Progress::Failed(text) => text,
        }
    }
}

/// What a notification array says about the job it belongs to.
///
/// `None` for an empty array, which is what a job that has been queued behind
/// another one looks like: DHIS2 runs one analytics job at a time, and the
/// second one's notifier stays empty until the first finishes.
///
/// The entries are ordered by their own `time` rather than by their position,
/// because the two ends of the array have swapped between DHIS2 versions and
/// the timestamps are ISO-8601, which sorts as text. An `ERROR` outranks a
/// `completed`, since a failed run announces both.
pub fn progress_of(notifications: &serde_json::Value) -> Option<Progress> {
    let entries = notifications.as_array()?;
    if entries.is_empty() {
        return None;
    }
    let message_of = |entry: &serde_json::Value| text_at(entry, "message");
    let latest = entries
        .iter()
        .max_by_key(|entry| text_at(entry, "time"))
        .map(message_of)
        .unwrap_or_default();
    if let Some(failure) = entries
        .iter()
        .filter(|entry| text_at(entry, "level").eq_ignore_ascii_case("ERROR"))
        .max_by_key(|entry| text_at(entry, "time"))
    {
        return Some(Progress::Failed(message_of(failure)));
    }
    if entries
        .iter()
        .any(|entry| entry.get("completed").and_then(serde_json::Value::as_bool) == Some(true))
    {
        return Some(Progress::Done(latest));
    }
    Some(Progress::Running(latest))
}

/// The analytics job that is already running, out of `GET /api/system/tasks/
/// ANALYTICS_TABLE`.
///
/// Worth asking before starting one: DHIS2 runs a single analytics job at a
/// time, so a second `POST` queues behind the first and its notifier stays
/// empty - a wait on it would report nothing for as long as the first one takes.
pub fn running_job(tasks: &serde_json::Value) -> Option<String> {
    let mut running: Vec<(String, String)> = tasks
        .as_object()?
        .iter()
        .filter(|(_, notifications)| {
            matches!(progress_of(notifications), Some(Progress::Running(_)))
        })
        .map(|(job, notifications)| (latest_time(notifications), job.clone()))
        .collect();
    running.sort();
    running.pop().map(|(_, job)| job)
}

/// The newest `time` in a notification array, for ordering two of them.
fn latest_time(notifications: &serde_json::Value) -> String {
    notifications
        .as_array()
        .into_iter()
        .flatten()
        .map(|entry| text_at(entry, "time"))
        .max()
        .unwrap_or_default()
}

/// The job id a `POST /api/resourceTables/analytics` answered with.
///
/// Three spellings, because the shape of that `WebMessage` has moved: the id
/// under `response`, the notifier endpoint it is the last segment of, and a
/// bare `id` at the top. A run whose id cannot be read is not a failure - the
/// caller falls back to whatever job is running, which is this one.
pub fn job_id_of(response: &serde_json::Value) -> Option<String> {
    let inner = response.get("response");
    let id = inner
        .and_then(|inner| inner.get("id"))
        .and_then(serde_json::Value::as_str)
        .map(str::to_string)
        .or_else(|| {
            inner
                .and_then(|inner| inner.get("relativeNotifierEndpoint"))
                .and_then(serde_json::Value::as_str)
                .and_then(|endpoint| endpoint.rsplit('/').next())
                .map(str::to_string)
        })
        .or_else(|| {
            response
                .get("id")
                .and_then(serde_json::Value::as_str)
                .map(str::to_string)
        })?;
    Some(id).filter(|id| !id.trim().is_empty())
}

impl Dhis2 {
    /// Poll one analytics job until it is no longer running.
    ///
    /// `step` is called with each message the job announces, so a wait that
    /// takes an hour is not a silent one. The deadline is an error rather than
    /// a verdict: DHIS2 is still doing the work, and stopping it is nobody's
    /// business but the operator's, so the message names the job.
    pub fn wait_for_job(
        &self,
        job: &str,
        timeout: Duration,
        interval: Duration,
        step: &mut impl FnMut(&str),
    ) -> Result<Progress> {
        let until = Instant::now() + timeout;
        let path = job_path(job);
        let mut last = String::new();
        loop {
            let progress = progress_of(&self.get_json(&path)?);
            if let Some(progress) = &progress
                && progress.message() != last
            {
                last = progress.message().to_string();
                step(&last);
            }
            match progress {
                Some(Progress::Running(_)) | None => {}
                Some(done) => return Ok(done),
            }
            if Instant::now() + interval >= until {
                return Err(anyhow::anyhow!(
                    "analytics job {job} was still running after {}; DHIS2 is still generating the \
                     tables, so `chaps dhis2 analytics` watches the same job again, and \
                     `--timeout SECONDS` waits longer",
                    crate::output::human_age(timeout)
                ));
            }
            std::thread::sleep(interval);
        }
    }
}

// ---------------------------------------------------------------------------
// The apps
// ---------------------------------------------------------------------------

/// One app as the App Hub describes it.
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct HubApp {
    #[serde(default)]
    pub name: String,
    #[serde(default)]
    pub versions: Vec<HubVersion>,
}

/// One published version of it.
#[derive(Debug, Clone, Default, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct HubVersion {
    /// The id `POST /api/appHub/{id}` installs.
    #[serde(default)]
    pub id: String,
    #[serde(default)]
    pub version: String,
    #[serde(default)]
    pub min_dhis_version: Option<String>,
    #[serde(default)]
    pub max_dhis_version: Option<String>,
}

/// One app as the instance lists it under `GET /api/apps`.
#[derive(Debug, Clone, Default, PartialEq, Eq, Deserialize)]
pub struct InstalledApp {
    #[serde(default)]
    pub name: String,
    #[serde(default)]
    pub key: String,
    #[serde(default)]
    pub version: String,
}

/// Base of the App Hub this run reads, for the tests to move.
pub fn app_hub_base() -> String {
    crate::github::base(APP_HUB_VAR, DEFAULT_APP_HUB)
}

/// `GET {app_hub}/{id}`: what the App Hub publishes for one app.
///
/// Its own request rather than one through DHIS2: the version id has to be
/// resolved before the instance can be told to install it, and this is the only
/// place that number exists. It is also why the step needs the network twice -
/// once from here for the id, once from DHIS2 for the app itself.
pub fn fetch_hub_app(base: &str, id: &str, timeout: Duration) -> Result<HubApp> {
    let url = format!("{}/{id}", base.trim_end_matches('/'));
    let agent = ureq::Agent::new_with_config(
        ureq::Agent::config_builder()
            .timeout_global(Some(timeout))
            .user_agent(USER_AGENT)
            .http_status_as_error(false)
            .build(),
    );
    crate::output::verbose(&format!("GET {url}"));
    let mut response = agent
        .get(&url)
        .header("Accept", crate::api::JSON)
        .call()
        .map_err(|e| {
            anyhow::anyhow!(
                "the DHIS2 App Hub at {url} could not be reached: {e}; install both apps from \
                 DHIS2's own App Management page instead, or fix this machine's reach to \
                 apps.dhis2.org"
            )
        })?;
    let status = response.status().as_u16();
    let body = response
        .body_mut()
        .read_to_string()
        .map_err(|e| anyhow::anyhow!("reading the answer from {url}: {e}"))?;
    crate::output::debug(&format!("  {}", crate::output::trace_body(&body)));
    if !(200..300).contains(&status) {
        return Err(ChapError::Http {
            url: url.clone(),
            status,
        }
        .into());
    }
    serde_json::from_str(&body)
        .map_err(|e| anyhow::anyhow!("the App Hub's answer for {id} is not an app listing: {e}"))
}

/// The newest version of an app this DHIS2 can run.
///
/// `dhis_version` is the instance's own, and a version whose `minDhisVersion`
/// is newer than it is skipped: DHIS2 installs it happily and the app then
/// fails in the browser, which is a worse outcome than installing the newest
/// one that fits. An empty `dhis_version` - an instance that did not say -
/// takes the newest published version, because refusing on a missing field
/// would refuse everything.
pub fn pick_version<'a>(versions: &'a [HubVersion], dhis_version: &str) -> Option<&'a HubVersion> {
    let instance = dhis_version_parts(dhis_version);
    versions
        .iter()
        .filter(|version| !version.id.trim().is_empty())
        .filter(|version| instance.is_empty() || fits(version, &instance))
        .max_by(|a, b| compare_versions(&a.version, &b.version))
}

/// Whether one published version runs on an instance of these parts.
fn fits(version: &HubVersion, instance: &[u64]) -> bool {
    let at_least = version
        .min_dhis_version
        .as_deref()
        .map(|min| compare_parts(instance, &dhis_version_parts(min)) != std::cmp::Ordering::Less)
        .unwrap_or(true);
    let at_most = version
        .max_dhis_version
        .as_deref()
        .map(|max| compare_parts(instance, &dhis_version_parts(max)) != std::cmp::Ordering::Greater)
        .unwrap_or(true);
    at_least && at_most
}

/// The app of that name the instance already has, if any.
///
/// Matched on the App Hub's name against the instance's `name` and `key`, with
/// the punctuation and the case taken out, because an installed app's key is its
/// name in another spelling (`DHIS2 Climate App` / `dhis2-climate-app`). An
/// exact match first, then one that contains the other, which covers the app
/// that is listed under a longer or shorter name than the App Hub publishes.
///
/// A wrong answer here costs a reinstall and nothing else:
/// `POST /api/appHub/{versionId}` is idempotent, so a miss installs an app that
/// was already there rather than skipping one that was not.
pub fn installed_app(apps: &serde_json::Value, name: &str) -> Option<InstalledApp> {
    let wanted = squash(name);
    let rows: Vec<InstalledApp> = apps
        .as_array()?
        .iter()
        .filter_map(|row| serde_json::from_value::<InstalledApp>(row.clone()).ok())
        .collect();
    rows.iter()
        .find(|app| squash(&app.name) == wanted || squash(&app.key) == wanted)
        .or_else(|| {
            rows.iter()
                .find(|app| near(&squash(&app.name), &wanted) || near(&squash(&app.key), &wanted))
        })
        .cloned()
}

/// A name with everything but its letters and digits removed, lowercased.
fn squash(name: &str) -> String {
    name.chars()
        .filter(|c| c.is_alphanumeric())
        .flat_map(char::to_lowercase)
        .collect()
}

/// Whether two squashed names are the same app under different spellings.
///
/// Containment, with a floor on the length so that two short names cannot match
/// each other by accident.
fn near(a: &str, b: &str) -> bool {
    a.len() >= 6 && b.len() >= 6 && (a.contains(b) || b.contains(a))
}

/// Where `POST /api/appHub/{versionId}` installs from.
pub fn install_path(version_id: &str) -> String {
    format!(
        "{APP_HUB_INSTALL_PATH}/{}",
        crate::api::encode(version_id.trim())
    )
}

/// Why installing an app cannot work with `--offline`.
pub const OFFLINE_APPS: &str = "installing an app needs the DHIS2 App Hub, twice: chaps resolves \
     the version there and DHIS2 downloads the app itself; run the command without `--offline`, \
     or install both apps from DHIS2's own App Management page";

// ---------------------------------------------------------------------------
// Versions, and the one encoding
// ---------------------------------------------------------------------------

/// The numeric parts of a version, `7.1.0` as `[7, 1, 0]`.
///
/// Leading digits of each dot-separated part, so a pre-release suffix is
/// dropped rather than read as a fourth number: `1.16.2-rc1` is `[1, 16, 2]`.
/// It stops at the first part that does not begin with a digit, which is what
/// makes an empty string - an instance that did not say its version - an empty
/// list rather than a version 0 that nothing is compatible with.
pub fn version_parts(version: &str) -> Vec<u64> {
    version
        .trim()
        .trim_start_matches('v')
        .split('.')
        .map(|part| {
            part.chars()
                .take_while(|c| c.is_ascii_digit())
                .collect::<String>()
        })
        .take_while(|digits| !digits.is_empty())
        .map(|digits| digits.parse().unwrap_or(0))
        .collect()
}

/// [`version_parts`] with DHIS2's historical `2.` prefix dropped.
///
/// The same release is written `2.42` and `42` depending on who is writing it -
/// the App Hub says `minDhisVersion: "2.40"`, other places say `40` - and
/// comparing the two spellings as they stand would make every app look
/// incompatible. Dropping a leading `2` in front of something else normalises
/// both to the minor line.
pub fn dhis_version_parts(version: &str) -> Vec<u64> {
    let parts = version_parts(version);
    match parts.split_first() {
        Some((2, rest)) if !rest.is_empty() => rest.to_vec(),
        _ => parts,
    }
}

/// Compare two version strings part by part, a missing part counting as 0.
pub fn compare_versions(a: &str, b: &str) -> std::cmp::Ordering {
    compare_parts(&version_parts(a), &version_parts(b))
}

/// [`compare_versions`] on parts already read.
fn compare_parts(a: &[u64], b: &[u64]) -> std::cmp::Ordering {
    let width = a.len().max(b.len());
    for at in 0..width {
        let ordering = a.get(at).unwrap_or(&0).cmp(b.get(at).unwrap_or(&0));
        if ordering != std::cmp::Ordering::Equal {
            return ordering;
        }
    }
    std::cmp::Ordering::Equal
}

/// Standard base64, for the one header that needs it.
///
/// Written out rather than pulled in, the way [`crate::api::encode`] is: HTTP
/// Basic is the only thing in this CLI that encodes anything, and the binary
/// stays dependency-light.
pub fn base64(bytes: &[u8]) -> String {
    const ALPHABET: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::with_capacity(bytes.len().div_ceil(3) * 4);
    for chunk in bytes.chunks(3) {
        let bits = ((chunk[0] as u32) << 16)
            | ((*chunk.get(1).unwrap_or(&0) as u32) << 8)
            | *chunk.get(2).unwrap_or(&0) as u32;
        out.push(ALPHABET[(bits >> 18 & 63) as usize] as char);
        out.push(ALPHABET[(bits >> 12 & 63) as usize] as char);
        out.push(match chunk.len() {
            1 => '=',
            _ => ALPHABET[(bits >> 6 & 63) as usize] as char,
        });
        out.push(match chunk.len() {
            3 => ALPHABET[(bits & 63) as usize] as char,
            _ => '=',
        });
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn answer(status: u16, body: &str) -> Answer {
        Answer {
            status,
            reason: "Conflict".to_string(),
            content_type: crate::api::JSON.to_string(),
            body: body.as_bytes().to_vec(),
        }
    }

    fn route(url: &str) -> Route {
        Route {
            id: "abc123".to_string(),
            name: ROUTE_NAME.to_string(),
            code: ROUTE_CODE.to_string(),
            url: url.to_string(),
            disabled: false,
            authorities: vec![ROUTE_AUTHORITY.to_string()],
        }
    }

    #[test]
    fn base64_is_the_standard_encoding_with_padding() {
        assert_eq!(base64(b""), "");
        assert_eq!(base64(b"f"), "Zg==");
        assert_eq!(base64(b"fo"), "Zm8=");
        assert_eq!(base64(b"foo"), "Zm9v");
        assert_eq!(base64(b"foob"), "Zm9vYg==");
        assert_eq!(base64(b"fooba"), "Zm9vYmE=");
        assert_eq!(base64(b"foobar"), "Zm9vYmFy");
        // The header every one of these commands sends.
        assert_eq!(base64(b"admin:district"), "YWRtaW46ZGlzdHJpY3Q=");
    }

    #[test]
    fn the_authorization_header_is_basic_and_the_password_is_nowhere_else() {
        let credentials = Credentials::new("admin", "district", PasswordSource::Default);
        assert_eq!(credentials.header(), "Basic YWRtaW46ZGlzdHJpY3Q=");
        // Debug is written by hand precisely so this holds.
        let shown = format!("{credentials:?}");
        assert!(!shown.contains("district"), "{shown}");
        assert!(shown.contains("admin"), "{shown}");
        assert!(shown.contains("<hidden>"), "{shown}");
    }

    #[test]
    fn credentials_come_from_env_then_the_environment_then_the_default() {
        // Nothing anywhere: the DHIS2 default, which a seeded dump and an
        // empty database both give.
        let plain = credentials_of("", None, None);
        assert_eq!(plain.user, "admin");
        assert_eq!(plain.source, PasswordSource::Default);
        assert_eq!(plain.header(), "Basic YWRtaW46ZGlzdHJpY3Q=");

        // The environment, for an operator who will not have it on disk.
        let exported = credentials_of("", None, Some("sekret"));
        assert_eq!(exported.source, PasswordSource::Environment);
        assert_eq!(exported.user, "admin");

        // `.env` is the deployment's own answer and wins, the way
        // `api::token_for` reads the chap-core token.
        let body = "DHIS2_ADMIN_USERNAME=ops\nDHIS2_ADMIN_PASSWORD=from-the-file\n";
        let from_file = credentials_of(body, None, Some("sekret"));
        assert_eq!(from_file.user, "ops");
        assert_eq!(from_file.source, PasswordSource::EnvFile);

        // A commented placeholder sets nothing, and neither does an empty one.
        let commented = credentials_of("# DHIS2_ADMIN_PASSWORD=district\n", None, None);
        assert_eq!(commented.source, PasswordSource::Default);
        assert_eq!(
            credentials_of("DHIS2_ADMIN_PASSWORD=\n", None, Some(" ")).source,
            PasswordSource::Default
        );

        // `--user` names a user for one run and nothing else.
        let flagged = credentials_of(body, Some("someone"), None);
        assert_eq!(flagged.user, "someone");
        assert_eq!(flagged.source, PasswordSource::EnvFile);
        assert_eq!(credentials_of(body, Some("  "), None).user, "ops");
    }

    #[test]
    fn every_password_source_says_where_it_came_from() {
        for source in [
            PasswordSource::EnvFile,
            PasswordSource::Environment,
            PasswordSource::Default,
        ] {
            assert!(!source.describe().is_empty());
        }
        assert_eq!(PasswordSource::EnvFile.describe(), "from `.env`");
    }

    /// The target is the compose alias and the `/**` suffix, never localhost:
    /// the requests are made by DHIS2, from inside the deployment.
    #[test]
    fn the_route_target_is_the_service_alias_and_a_wildcard() {
        assert_eq!(route_target(""), "http://chap:8000/**");
        assert!(!route_target("").contains("localhost"));
        // A deployment behind a proxy moves every chap-core route with
        // CHAP_ROOT_PATH, this one included.
        assert_eq!(route_target("/master"), "http://chap:8000/master/**");
        assert_eq!(route_target("/master/"), "http://chap:8000/master/**");
    }

    #[test]
    fn the_route_payload_is_the_one_that_works() {
        let payload: serde_json::Value =
            serde_json::from_str(&route_payload("http://chap:8000/**"))
                .expect("the payload is JSON");
        assert_eq!(payload["code"], "chap");
        assert_eq!(payload["name"], ROUTE_NAME);
        assert_eq!(payload["url"], "http://chap:8000/**");
        assert_eq!(payload["authorities"][0], ROUTE_AUTHORITY);
        assert_eq!(payload["headers"]["Content-Type"], "application/json");
        assert_eq!(payload["responseTimeoutSeconds"], 30);
    }

    /// The whole point of the step: the demo dumps ship this route pointed at
    /// somebody else's CHAP, so a missing route and a wrong one are both work.
    #[test]
    fn a_route_pointing_elsewhere_is_repointed_and_not_skipped() {
        let target = route_target("");
        let external = route("http://158.39.75.126/stable/**");
        let RouteAction::Rewrite(reasons) = route_action(Some(&external), &target) else {
            panic!("a route aimed elsewhere has to be repointed");
        };
        assert_eq!(
            reasons,
            vec!["it pointed at http://158.39.75.126/stable/**"]
        );
    }

    #[test]
    fn a_route_that_already_matches_is_left_alone() {
        let target = route_target("");
        assert_eq!(
            route_action(Some(&route(&target)), &target),
            RouteAction::Keep
        );
        assert_eq!(route_action(None, &target), RouteAction::Create);
    }

    /// The URL is the point and not the whole of it: a route aimed correctly
    /// but switched off proxies nothing.
    #[test]
    fn a_disabled_or_unauthorised_route_is_repaired_with_its_reason() {
        let target = route_target("");
        let mut off = route(&target);
        off.disabled = true;
        let RouteAction::Rewrite(reasons) = route_action(Some(&off), &target) else {
            panic!("a disabled route has to be rewritten");
        };
        assert_eq!(reasons, vec!["it was disabled"]);

        let mut bare = route(&target);
        bare.authorities.clear();
        let RouteAction::Rewrite(reasons) = route_action(Some(&bare), &target) else {
            panic!("a route without the authority has to be rewritten");
        };
        assert_eq!(
            reasons,
            vec!["it was missing the F_CHAP_MODELING_APP authority"]
        );

        // All three at once, in the order they are checked.
        let mut every = route("http://elsewhere/**");
        every.disabled = true;
        every.authorities.clear();
        let RouteAction::Rewrite(reasons) = route_action(Some(&every), &target) else {
            panic!("three reasons is still a rewrite");
        };
        assert_eq!(reasons.len(), 3);
    }

    #[test]
    fn the_route_is_found_by_code_in_either_listing_shape() {
        let listing = serde_json::json!({"routes": [
            {"id": "one", "code": "other", "url": "http://x/**"},
            {"id": "two", "code": "chap", "url": "http://chap:8000/**",
             "authorities": ["F_CHAP_MODELING_APP"]},
        ]});
        let found = route_in(&listing).expect("the chap route");
        assert_eq!(found.id, "two");
        assert_eq!(found.url, "http://chap:8000/**");
        assert!(!found.disabled, "absent means not disabled");

        // A bare array answers too, and a listing without the code answers
        // nothing rather than the first row.
        let bare = serde_json::json!([{"id": "two", "code": "chap", "url": "u"}]);
        assert_eq!(route_in(&bare).expect("the chap route").id, "two");
        assert_eq!(route_in(&serde_json::json!({"routes": []})), None);
        assert_eq!(route_in(&serde_json::json!("nope")), None);
    }

    #[test]
    fn the_run_path_is_the_one_the_app_itself_uses() {
        assert_eq!(route_run_path("/health"), "/api/routes/chap/run/health");
        assert_eq!(
            route_run_path("system/info"),
            "/api/routes/chap/run/system/info"
        );
    }

    #[test]
    fn the_allowlist_refusal_names_the_file_and_the_way_out() {
        let text = allowlist_hint("http://chap:8000/**");
        assert!(text.contains("`dhis2/dhis.conf`"), "{text}");
        assert!(text.contains("route.remote_servers_allowed"), "{text}");
        assert!(text.contains("`chaps restart dhis2`"), "{text}");

        assert!(is_allowlist_refusal(&answer(
            409,
            r#"{"message":"Remote server not allowed"}"#
        )));
        assert!(!is_allowlist_refusal(&answer(
            409,
            r#"{"message":"Route with code chap already exists"}"#
        )));
    }

    #[test]
    fn a_dhis2_refusal_names_the_user_and_never_the_password() {
        let dhis2 = Dhis2::new(
            "http://localhost:18080",
            Credentials::new("admin", "district", PasswordSource::Default),
            DEFAULT_TIMEOUT,
        );
        let text = dhis2.refusal(&answer(401, "{}")).expect("a 401 sentence");
        assert!(
            text.contains("did not accept the credentials for `admin`"),
            "{text}"
        );
        assert!(text.contains("DHIS2_ADMIN_PASSWORD"), "{text}");
        assert!(text.contains("CHAPS_DHIS2_PASSWORD"), "{text}");
        assert!(text.contains("the DHIS2 default"), "{text}");
        assert!(!text.contains("district"), "{text}");

        let text = dhis2.refusal(&answer(403, "{}")).expect("a 403 sentence");
        assert!(text.contains("--user NAME"), "{text}");
        assert!(!text.contains("district"), "{text}");

        assert_eq!(dhis2.refusal(&answer(404, "{}")), None);
        // And the generic error keeps DHIS2's own message.
        let err = dhis2.status_error(ROUTES_PATH, &answer(409, r#"{"message":"nope"}"#));
        assert!(err.to_string().contains("nope"), "{err}");
        assert!(err.to_string().contains("HTTP 409"), "{err}");
    }

    #[test]
    fn what_dhis2_said_is_its_web_message() {
        assert_eq!(
            said(&answer(409, r#"{"message":"Already exists"}"#)),
            "Already exists"
        );
        assert_eq!(said(&answer(500, r#"{"devMessage":"NPE"}"#)), "NPE");
        assert_eq!(said(&answer(404, "  Not Found  ")), "Not Found");
        // An empty document says nothing, so the status line stands alone.
        assert_eq!(said(&answer(404, "{}")), "");
        assert_eq!(said(&answer(404, " [] ")), "");
        let long = answer(500, &"x".repeat(400));
        assert!(said(&long).ends_with("..."));
        assert_eq!(said(&long).chars().count(), 203);
    }

    /// The analytics endpoint must never carry `lastYears`: measured, it wrote
    /// zero rows into every table and called it a success.
    #[test]
    fn the_analytics_request_skips_tracked_entities_and_nothing_else() {
        assert_eq!(
            ANALYTICS_PATH,
            "/api/resourceTables/analytics?skipTrackedEntities=true"
        );
        assert!(!ANALYTICS_PATH.contains("lastYears"));
    }

    #[test]
    fn a_jobs_progress_is_read_off_its_notifications() {
        let running = serde_json::json!([
            {"time": "2026-09-25T10:00:00.000", "level": "INFO", "message": "started",
             "completed": false},
            {"time": "2026-09-25T10:00:30.000", "level": "INFO", "message": "analytics_2024",
             "completed": false},
        ]);
        assert_eq!(
            progress_of(&running),
            Some(Progress::Running("analytics_2024".to_string()))
        );

        // The latest is by `time`, not by position: the two ends of the array
        // have swapped between DHIS2 versions.
        let reversed = serde_json::json!([
            {"time": "2026-09-25T10:00:30.000", "message": "analytics_2024"},
            {"time": "2026-09-25T10:00:00.000", "message": "started"},
        ]);
        assert_eq!(
            progress_of(&reversed),
            Some(Progress::Running("analytics_2024".to_string()))
        );

        let done = serde_json::json!([
            {"time": "2026-09-25T10:00:00.000", "message": "started", "completed": false},
            {"time": "2026-09-25T10:01:00.000", "message": "Analytics tables updated",
             "completed": true},
        ]);
        assert_eq!(
            progress_of(&done),
            Some(Progress::Done("Analytics tables updated".to_string()))
        );

        // A failure announces both, and the error is what matters.
        let failed = serde_json::json!([
            {"time": "2026-09-25T10:00:00.000", "message": "started"},
            {"time": "2026-09-25T10:00:10.000", "level": "ERROR", "message": "out of memory"},
            {"time": "2026-09-25T10:00:11.000", "message": "done", "completed": true},
        ]);
        assert_eq!(
            progress_of(&failed),
            Some(Progress::Failed("out of memory".to_string()))
        );

        // An empty notifier is a job queued behind another one, which is not
        // the same thing as a job that has finished.
        assert_eq!(progress_of(&serde_json::json!([])), None);
        assert_eq!(progress_of(&serde_json::json!({})), None);
    }

    /// DHIS2 runs one analytics job at a time, so the one already going is
    /// what to watch: a second POST queues and its notifier stays empty.
    #[test]
    fn the_running_job_is_the_one_to_watch() {
        let tasks = serde_json::json!({
            "old": [{"time": "2026-09-25T09:00:00.000", "message": "done", "completed": true}],
            "now": [{"time": "2026-09-25T10:00:00.000", "message": "working"}],
        });
        assert_eq!(running_job(&tasks).as_deref(), Some("now"));

        // Two running at once cannot happen, but the newer one is the answer.
        let two = serde_json::json!({
            "a": [{"time": "2026-09-25T09:00:00.000", "message": "working"}],
            "b": [{"time": "2026-09-25T10:00:00.000", "message": "working"}],
        });
        assert_eq!(running_job(&two).as_deref(), Some("b"));

        // Nothing running, and nothing at all.
        let finished = serde_json::json!({
            "old": [{"time": "2026-09-25T09:00:00.000", "message": "done", "completed": true}],
        });
        assert_eq!(running_job(&finished), None);
        assert_eq!(running_job(&serde_json::json!({})), None);
    }

    #[test]
    fn the_job_id_is_read_in_every_spelling_that_web_message_has_had() {
        assert_eq!(
            job_id_of(&serde_json::json!({"response": {"id": "abc"}})).as_deref(),
            Some("abc")
        );
        assert_eq!(
            job_id_of(&serde_json::json!({"response": {
                "relativeNotifierEndpoint": "/api/system/tasks/ANALYTICS_TABLE/xyz"
            }}))
            .as_deref(),
            Some("xyz")
        );
        assert_eq!(
            job_id_of(&serde_json::json!({"id": "top"})).as_deref(),
            Some("top")
        );
        assert_eq!(job_id_of(&serde_json::json!({"status": "OK"})), None);
        assert_eq!(
            job_id_of(&serde_json::json!({"response": {"id": " "}})),
            None
        );
    }

    #[test]
    fn a_version_is_compared_part_by_part() {
        assert_eq!(version_parts("7.1.0"), vec![7, 1, 0]);
        assert_eq!(version_parts("v1.16.2"), vec![1, 16, 2]);
        assert_eq!(version_parts("1.16.2-rc1"), vec![1, 16, 2]);
        // An instance that did not say has no version, which is not version 0.
        assert_eq!(version_parts(""), Vec::<u64>::new());
        assert_eq!(version_parts("unknown"), Vec::<u64>::new());
        assert_eq!(compare_versions("7.1.0", "7.2.0"), std::cmp::Ordering::Less);
        assert_eq!(compare_versions("7.2", "7.2.0"), std::cmp::Ordering::Equal);
        assert_eq!(
            compare_versions("10.0.0", "9.9.9"),
            std::cmp::Ordering::Greater
        );
    }

    /// The same DHIS2 release is written `2.42` and `42`, and comparing the
    /// two spellings as they stand would make every app look incompatible.
    #[test]
    fn a_dhis2_version_is_the_same_whichever_way_it_is_spelled() {
        assert_eq!(dhis_version_parts("2.42.6"), vec![42, 6]);
        assert_eq!(dhis_version_parts("2.40"), vec![40]);
        assert_eq!(dhis_version_parts("42"), vec![42]);
        assert_eq!(dhis_version_parts("42.1"), vec![42, 1]);
        // Nothing to strip: a bare `2` stays a 2.
        assert_eq!(dhis_version_parts("2"), vec![2]);
    }

    #[test]
    fn the_newest_version_the_instance_can_run_is_the_one_picked() {
        let versions = vec![
            HubVersion {
                id: "old".into(),
                version: "6.0.0".into(),
                min_dhis_version: Some("2.38".into()),
                max_dhis_version: None,
            },
            HubVersion {
                id: "new".into(),
                version: "7.1.0".into(),
                min_dhis_version: Some("2.40".into()),
                max_dhis_version: None,
            },
            HubVersion {
                id: "future".into(),
                version: "8.0.0".into(),
                min_dhis_version: Some("2.43".into()),
                max_dhis_version: None,
            },
        ];
        assert_eq!(pick_version(&versions, "2.42.6").unwrap().id, "new");
        assert_eq!(pick_version(&versions, "2.38.1").unwrap().id, "old");
        assert_eq!(pick_version(&versions, "2.43").unwrap().id, "future");
        // An instance that did not say its version takes the newest rather
        // than nothing at all.
        assert_eq!(pick_version(&versions, "").unwrap().id, "future");
        // A version with no installable id is not a candidate.
        assert_eq!(pick_version(&[HubVersion::default()], "2.42"), None);
        assert_eq!(pick_version(&[], "2.42"), None);

        // An upper bound is honoured where the App Hub sets one.
        let capped = vec![HubVersion {
            id: "capped".into(),
            version: "1.0.0".into(),
            min_dhis_version: None,
            max_dhis_version: Some("2.41".into()),
        }];
        assert_eq!(pick_version(&capped, "2.42"), None);
        assert_eq!(pick_version(&capped, "2.41").unwrap().id, "capped");
    }

    #[test]
    fn an_installed_app_is_matched_on_its_name_or_its_key() {
        let apps = serde_json::json!([
            {"name": "Modeling App", "key": "modeling-app", "version": "7.0.0"},
            {"name": "DHIS2 Climate App", "key": "dhis2-climate-app", "version": "1.16.2"},
        ]);
        let found = installed_app(&apps, "Modeling App").expect("the modeling app");
        assert_eq!(found.version, "7.0.0");
        // The key is the name in another spelling, and either one answers.
        assert_eq!(
            installed_app(&apps, "dhis2 climate app").unwrap().version,
            "1.16.2"
        );
        assert_eq!(
            installed_app(&apps, "DHIS2-Climate-App").unwrap().key,
            "dhis2-climate-app"
        );
        assert_eq!(installed_app(&apps, "Something Else"), None);
        assert_eq!(installed_app(&serde_json::json!([]), "Modeling App"), None);
        assert_eq!(installed_app(&serde_json::json!({}), "Modeling App"), None);

        // A longer or shorter listing name is the same app: the App Hub's
        // `DHIS2 Climate App` is installed as `Climate`, and the other way
        // round.
        assert_eq!(
            installed_app(
                &serde_json::json!([{"name": "Climate App", "version": "1.16.2"}]),
                "DHIS2 Climate App"
            )
            .unwrap()
            .version,
            "1.16.2"
        );
        // And two short names cannot match each other by accident.
        assert_eq!(
            installed_app(&serde_json::json!([{"name": "Maps"}]), "Data"),
            None
        );
    }

    #[test]
    fn every_app_carries_the_name_a_listing_is_matched_on() {
        for app in HUB_APPS {
            assert!(!app.name.is_empty());
            let listing = serde_json::json!([{"name": app.name, "version": "1.0.0"}]);
            assert!(
                installed_app(&listing, app.name).is_some(),
                "{} has to match its own name",
                app.name
            );
        }
    }

    #[test]
    fn the_install_path_is_the_app_hub_one() {
        assert_eq!(install_path("abc-123"), "/api/appHub/abc-123");
        // An id with a slash in it would otherwise be a path of its own.
        assert_eq!(install_path("a/b"), "/api/appHub/a%2Fb");
    }

    #[test]
    fn the_two_apps_are_the_modeling_app_and_the_climate_app() {
        assert_eq!(HUB_APPS.len(), 2);
        assert_eq!(HUB_APPS[0].id, "a29851f9-82a7-4ecd-8b2c-58e0f220bc75");
        assert_eq!(HUB_APPS[1].id, "effb986c-a3c7-485e-a2f6-5e54ff9df7c3");
        for app in HUB_APPS {
            assert!(!app.label.is_empty() && !app.what.is_empty());
        }
    }

    #[test]
    fn the_app_hub_base_is_public_unless_the_variable_moves_it() {
        assert_eq!(
            crate::github::base_of(None, DEFAULT_APP_HUB),
            "https://apps.dhis2.org/api/v1/apps"
        );
        assert_eq!(
            crate::github::base_of(Some("http://127.0.0.1:18099/apps/".into()), DEFAULT_APP_HUB),
            "http://127.0.0.1:18099/apps"
        );
    }

    #[test]
    fn the_url_is_the_base_and_the_path_with_one_slash_between_them() {
        let dhis2 = Dhis2::new(
            "http://localhost:18080/",
            Credentials::new("admin", "district", PasswordSource::Default),
            DEFAULT_TIMEOUT,
        );
        assert_eq!(dhis2.base(), "http://localhost:18080");
        assert_eq!(dhis2.url("/api/me"), "http://localhost:18080/api/me");
        assert_eq!(dhis2.url("api/me"), "http://localhost:18080/api/me");
        assert_eq!(dhis2.user(), "admin");
        assert_eq!(dhis2.password_source(), PasswordSource::Default);
        assert_eq!(
            dhis2.with_timeout(INSTALL_TIMEOUT).url("/api/me"),
            "http://localhost:18080/api/me"
        );
    }

    /// Port 9 is the discard service, so this covers the transport-error path
    /// without a network - and the sentence is DHIS2's, not chap-core's.
    #[test]
    fn an_unreachable_dhis2_says_so_in_its_own_words() {
        let dhis2 = Dhis2::new(
            "http://127.0.0.1:9",
            Credentials::new("admin", "district", PasswordSource::Default),
            Duration::from_secs(2),
        );
        let err = dhis2
            .send("GET", SYSTEM_INFO_PATH, None)
            .expect_err("nothing is listening on port 9");
        assert!(
            matches!(
                err.downcast_ref::<ChapError>(),
                Some(ChapError::Dhis2Unreachable { .. })
            ),
            "{err}"
        );
        let text = err.to_string();
        assert!(text.contains("DHIS2 at http://127.0.0.1:9"), "{text}");
        assert!(text.contains("`chaps status`"), "{text}");
        assert!(!text.contains("chap-core"), "{text}");
    }

    /// The wait gives up with the two things the reader needs: where the
    /// migration shows, and the flag that waits longer.
    #[test]
    fn a_wait_that_runs_out_names_the_logs_and_the_flag() {
        let dhis2 = Dhis2::new(
            "http://127.0.0.1:9",
            Credentials::new("admin", "district", PasswordSource::Default),
            Duration::from_millis(200),
        );
        let mut said = 0;
        let err = dhis2
            .wait_for_api(
                Duration::from_millis(1),
                Duration::from_millis(1),
                &mut |_| said += 1,
            )
            .expect_err("nothing is listening on port 9");
        let text = err.to_string();
        assert!(text.contains("`chaps logs dhis2`"), "{text}");
        assert!(text.contains("--wait SECONDS"), "{text}");
        assert_eq!(said, 1, "the wait announces itself once");
    }

    #[test]
    fn the_offline_refusal_names_both_halves_of_the_network_it_needs() {
        assert!(OFFLINE_APPS.contains("App Hub"));
        assert!(OFFLINE_APPS.contains("`--offline`"));
        assert!(OFFLINE_APPS.contains("App Management"));
    }
}
