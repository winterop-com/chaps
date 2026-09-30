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

/// Who the credential belongs to, for a token that does not say.
pub const ME_PATH: &str = "/api/me?fields=username";

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

/// The route `auth` type that sends fixed headers with every proxied request,
/// which is how chap-core's bearer token rides along.
///
/// DHIS2 stores the headers and never hands them back: a listing shows
/// `"auth": {"type": "api-headers"}` and nothing else, so whether the token in
/// there is the current one is only ever known by asking through the route.
pub const ROUTE_AUTH_TYPE: &str = "api-headers";

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
const USER_AGENT: &str = concat!("chaps/", env!("CARGO_PKG_VERSION"));

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

/// The `.env` variable holding a DHIS2 personal access token. Also chaps'
/// alone, and preferred to the password pair when both are set.
pub const API_TOKEN_ENV_VAR: &str = "DHIS2_API_TOKEN";

/// The environment variable a password is read from when `.env` has none for
/// the user being asked as.
///
/// For an operator who will not have the password on disk: exporting it for
/// one shell is the way to run these commands without writing it down.
pub const PASSWORD_ENV_VAR: &str = "CHAPS_DHIS2_PASSWORD";

/// The user [`PASSWORD_ENV_VAR`] belongs to, when it is not the one `.env`
/// names.
pub const USERNAME_ENV_VAR: &str = "CHAPS_DHIS2_USERNAME";

/// The environment variable a personal access token is read from.
pub const TOKEN_ENV_VAR: &str = "CHAPS_DHIS2_TOKEN";

/// The user a DHIS2 with no `.env` line is asked as.
pub const DEFAULT_USERNAME: &str = "admin";

/// That user's password on a DHIS2 chaps deployed.
///
/// The same two words whichever way the database was created: the published
/// demo dumps ship this user, and a Flyway-bootstrapped empty database gets it
/// from `DefaultAdminUserPopulator`, which has it compiled in. It is a default,
/// not a secret - every DHIS2 tutorial in the world prints it - and an instance
/// whose password has been changed says so with [`ADMIN_PASSWORD_ENV_VAR`].
///
/// Never tried against an external DHIS2: chaps did not create that instance,
/// so it knows nothing about its passwords, and sending the tutorial one to a
/// production server is a failed login in its audit log and nothing else.
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

/// Where the credential came from, which is all a report may say about it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum CredentialSource {
    /// An active line in this deployment's `.env`: [`API_TOKEN_ENV_VAR`], or
    /// [`ADMIN_PASSWORD_ENV_VAR`] for the user `.env` names.
    EnvFile,
    /// [`TOKEN_ENV_VAR`] or [`PASSWORD_ENV_VAR`] in the environment this
    /// command ran in.
    Environment,
    /// Neither, so [`DEFAULT_PASSWORD`] - which is what a seeded dump and an
    /// empty database both give, on a DHIS2 chaps deployed and nowhere else.
    Default,
}

/// Which `Authorization` scheme a request goes out with.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum AuthKind {
    /// `Basic`, a username and its password.
    Basic,
    /// `ApiToken`, a DHIS2 personal access token.
    Token,
}

/// The phrase a report names a credential by: what it is and where it was
/// found, never what it is worth.
pub fn describe_credential(kind: AuthKind, source: CredentialSource) -> String {
    match (kind, source) {
        (AuthKind::Basic, CredentialSource::EnvFile) => "password from `.env`".to_string(),
        (AuthKind::Basic, CredentialSource::Environment) => {
            format!("password from {PASSWORD_ENV_VAR}")
        }
        (AuthKind::Basic, CredentialSource::Default) => "the DHIS2 default password".to_string(),
        (AuthKind::Token, CredentialSource::EnvFile) => "API token from `.env`".to_string(),
        (AuthKind::Token, _) => format!("API token from {TOKEN_ENV_VAR}"),
    }
}

/// The secret half of [`Credentials`].
#[derive(Clone)]
enum Secret {
    Basic { user: String, password: String },
    Token(String),
}

/// A DHIS2 user and password, or a personal access token, and where it was
/// found.
///
/// `Debug` is written by hand: a derived one would put the password into every
/// `-d` line and every panic message that carries this value.
#[derive(Clone)]
pub struct Credentials {
    secret: Secret,
    pub source: CredentialSource,
}

impl std::fmt::Debug for Credentials {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let mut debug = f.debug_struct("Credentials");
        match &self.secret {
            Secret::Basic { user, .. } => debug.field("user", user).field("password", &"<hidden>"),
            Secret::Token(_) => debug.field("token", &"<hidden>"),
        };
        debug.field("source", &self.source).finish()
    }
}

impl Credentials {
    /// A username and password, however they were arrived at.
    pub fn new(user: &str, password: &str, source: CredentialSource) -> Credentials {
        Credentials {
            secret: Secret::Basic {
                user: user.to_string(),
                password: password.to_string(),
            },
            source,
        }
    }

    /// A personal access token.
    pub fn token(token: &str, source: CredentialSource) -> Credentials {
        Credentials {
            secret: Secret::Token(token.to_string()),
            source,
        }
    }

    /// Which scheme this is.
    pub fn kind(&self) -> AuthKind {
        match self.secret {
            Secret::Basic { .. } => AuthKind::Basic,
            Secret::Token(_) => AuthKind::Token,
        }
    }

    /// The username, for a password. A token carries none that chaps can
    /// read; DHIS2 says whose it is when asked ([`ME_PATH`]).
    pub fn user(&self) -> Option<&str> {
        match &self.secret {
            Secret::Basic { user, .. } => Some(user),
            Secret::Token(_) => None,
        }
    }

    /// The `Authorization` header value, which is the only thing that reads
    /// the secret.
    pub fn header(&self) -> String {
        match &self.secret {
            Secret::Basic { user, password } => {
                format!("Basic {}", base64(format!("{user}:{password}").as_bytes()))
            }
            Secret::Token(token) => format!("ApiToken {token}"),
        }
    }

    /// What a trace line says went out, in place of the header.
    fn traced(&self) -> String {
        match &self.secret {
            Secret::Basic { user, .. } => format!("Basic <{user} and its password>"),
            Secret::Token(_) => "ApiToken <the token>".to_string(),
        }
    }

    /// The phrase a report names this by.
    pub fn describe(&self) -> String {
        describe_credential(self.kind(), self.source)
    }
}

/// Everything [`credentials_of`] decides between, handed in so the rule can be
/// tested without an environment.
#[derive(Debug, Clone, Copy, Default)]
pub struct CredentialInputs<'a> {
    /// The deployment's `.env`, as text.
    pub env_body: &'a str,
    /// `--user NAME`.
    pub user: Option<&'a str>,
    /// [`USERNAME_ENV_VAR`].
    pub env_username: Option<&'a str>,
    /// [`PASSWORD_ENV_VAR`].
    pub env_password: Option<&'a str>,
    /// [`TOKEN_ENV_VAR`].
    pub env_token: Option<&'a str>,
    /// Whether chaps deployed this DHIS2, which is the only case in which
    /// [`DEFAULT_PASSWORD`] is known to be anybody's password.
    pub deployed: bool,
}

/// The credentials for this deployment's DHIS2, read from `.env` and the
/// environment this command runs in.
pub fn credentials_for(
    project_dir: &Path,
    user: Option<&str>,
    deployed: bool,
) -> Result<Credentials> {
    let body =
        std::fs::read_to_string(project_dir.join(crate::project::ENV_FILE)).unwrap_or_default();
    let var = |name: &str| std::env::var(name).ok();
    let (env_username, env_password, env_token) = (
        var(USERNAME_ENV_VAR),
        var(PASSWORD_ENV_VAR),
        var(TOKEN_ENV_VAR),
    );
    credentials_of(&CredentialInputs {
        env_body: &body,
        user,
        env_username: env_username.as_deref(),
        env_password: env_password.as_deref(),
        env_token: env_token.as_deref(),
        deployed,
    })
}

/// [`credentials_for`]'s rule.
///
/// **A password belongs to the user it was set with.** Each source is a pair,
/// and the first pair that has a value wins:
///
/// 1. `.env`: [`API_TOKEN_ENV_VAR`], then [`ADMIN_PASSWORD_ENV_VAR`] for the
///    user [`ADMIN_USERNAME_ENV_VAR`] names (`admin` when it names none);
/// 2. the environment: [`TOKEN_ENV_VAR`], then [`PASSWORD_ENV_VAR`] for
///    [`USERNAME_ENV_VAR`], or for the `.env` user when that is unset;
/// 3. `admin` / [`DEFAULT_PASSWORD`], on a DHIS2 chaps deployed only.
///
/// `.env` first, because that is the deployment's own answer and the file every
/// other secret of it lives in - the same order [`crate::api::token_for`] uses
/// for chap-core's token. Within a source the token comes first: it is the
/// credential that can be scoped and revoked, and an operator who wrote one
/// down meant it to be used.
///
/// `user` (`--user NAME`) asks for one user by name, so it is always a
/// password, and only a password that belongs to `NAME` is sent:
/// [`ADMIN_PASSWORD_ENV_VAR`] when `.env` names that user,
/// [`PASSWORD_ENV_VAR`] unless [`USERNAME_ENV_VAR`] gives it to someone else,
/// and the default for `admin` on a deployed DHIS2. Anything else is an error
/// before a request is made, rather than another user's password sent in
/// `NAME`'s name to fail in DHIS2's audit log.
pub fn credentials_of(inputs: &CredentialInputs) -> Result<Credentials> {
    let clean = |value: Option<&str>| {
        value
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(str::to_string)
    };
    let body = inputs.env_body;
    let file_user = crate::dotenv::non_empty(body, ADMIN_USERNAME_ENV_VAR);
    let file_password = crate::dotenv::non_empty(body, ADMIN_PASSWORD_ENV_VAR);
    let file_token = crate::dotenv::non_empty(body, API_TOKEN_ENV_VAR);
    let env_user = clean(inputs.env_username);
    let env_password = clean(inputs.env_password);
    let env_token = clean(inputs.env_token);
    // Whose each password is.
    let file_owner = file_user
        .clone()
        .unwrap_or_else(|| DEFAULT_USERNAME.to_string());
    let default_allowed = inputs.deployed && file_owner == DEFAULT_USERNAME;

    if let Some(name) = clean(inputs.user) {
        if let Some(password) = file_password.filter(|_| file_owner == name) {
            return Ok(Credentials::new(
                &name,
                &password,
                CredentialSource::EnvFile,
            ));
        }
        if let Some(password) =
            env_password.filter(|_| env_user.as_ref().is_none_or(|u| *u == name))
        {
            return Ok(Credentials::new(
                &name,
                &password,
                CredentialSource::Environment,
            ));
        }
        if inputs.deployed && name == DEFAULT_USERNAME {
            return Ok(Credentials::new(
                &name,
                DEFAULT_PASSWORD,
                CredentialSource::Default,
            ));
        }
        return Err(anyhow::anyhow!(
            "chaps has no password for DHIS2 user `{name}`: `.env` and `{USERNAME_ENV_VAR}` \
             give theirs to other users; export `{PASSWORD_ENV_VAR}` for this run"
        ));
    }

    if let Some(token) = file_token {
        return Ok(Credentials::token(&token, CredentialSource::EnvFile));
    }
    if let Some(password) = file_password {
        return Ok(Credentials::new(
            &file_owner,
            &password,
            CredentialSource::EnvFile,
        ));
    }
    if let Some(token) = env_token {
        return Ok(Credentials::token(&token, CredentialSource::Environment));
    }
    if let Some(password) = env_password {
        let user = env_user.unwrap_or(file_owner);
        return Ok(Credentials::new(
            &user,
            &password,
            CredentialSource::Environment,
        ));
    }
    if default_allowed {
        return Ok(Credentials::new(
            DEFAULT_USERNAME,
            DEFAULT_PASSWORD,
            CredentialSource::Default,
        ));
    }
    Err(anyhow::anyhow!(match inputs.deployed {
        true => format!(
            "chaps has no password for DHIS2 user `{file_owner}`: `.env` names the user and sets \
             no `{ADMIN_PASSWORD_ENV_VAR}`; set it there, or export `{PASSWORD_ENV_VAR}`"
        ),
        false => format!(
            "chaps has no credentials for this DHIS2, and did not deploy it, so there is no \
             default to try; {}",
            CREDENTIALS_WAY_OUT
        ),
    }))
}

/// The two ways to give chaps a DHIS2's credentials, in `.env` where the rest
/// of the deployment's secrets are.
pub const CREDENTIALS_WAY_OUT: &str = "set `DHIS2_ADMIN_USERNAME` and `DHIS2_ADMIN_PASSWORD`, or \
     `DHIS2_API_TOKEN`, in `.env`";

// ---------------------------------------------------------------------------
// The client
// ---------------------------------------------------------------------------

/// One DHIS2 instance, ready to be asked.
#[derive(Debug, Clone)]
pub struct Dhis2 {
    base: String,
    credentials: Credentials,
    timeout: Duration,
    /// Whether this is the deployment's own `dhis2` container, whose logs
    /// `chaps logs dhis2` shows, rather than an external instance.
    deployed: bool,
}

impl Dhis2 {
    /// A client for `base`, authenticating as `credentials`.
    pub fn new(base: &str, credentials: Credentials, timeout: Duration) -> Dhis2 {
        Dhis2 {
            base: base.trim_end_matches('/').to_string(),
            credentials,
            timeout,
            deployed: true,
        }
    }

    /// The same client, for a DHIS2 this deployment did not start.
    pub fn external(self) -> Dhis2 {
        Dhis2 {
            deployed: false,
            ..self
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

    /// The credential requests carry, for the line that says what it is.
    pub fn credentials(&self) -> &Credentials {
        &self.credentials
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
            // Never the value, and never the password or the token anywhere
            // near a trace line: they end up in bug reports.
            crate::output::verbose(&format!("  Authorization: {}", self.credentials.traced()));
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
                next: match self.deployed {
                    true => "run `chaps status` to see whether its container is up",
                    false => {
                        "check the URL with `chaps dhis2 use`, and that this machine can reach it"
                    }
                },
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
    /// The two are different problems: a 401 is the wrong password or a token
    /// DHIS2 no longer honours, a 403 is the right credential on a user DHIS2
    /// will not let do this. Neither ever prints the secret.
    pub fn refusal(&self, answer: &Answer) -> Option<String> {
        let who = self.who();
        match (answer.status, self.credentials.kind()) {
            (401, AuthKind::Basic) => Some(format!(
                "DHIS2 at {} did not accept the password for {who} ({}); set \
                 `{ADMIN_USERNAME_ENV_VAR}` and `{ADMIN_PASSWORD_ENV_VAR}` in `.env`, or export \
                 `{PASSWORD_ENV_VAR}`",
                self.base,
                self.credentials.describe()
            )),
            (401, AuthKind::Token) => Some(format!(
                "DHIS2 at {} did not accept the API token ({}): it is expired, revoked, or not \
                 allowed from this address; set a current one as `{API_TOKEN_ENV_VAR}` in \
                 `.env`, or export `{TOKEN_ENV_VAR}`",
                self.base,
                self.credentials.describe()
            )),
            (403, _) => Some(format!(
                "DHIS2 refused the request as {who} (HTTP 403{}): that user is authenticated but \
                 not allowed to do this, and a DHIS2 superuser is; name another with `--user NAME`",
                match said(answer) {
                    reason if reason.is_empty() => String::new(),
                    reason => format!(", \"{reason}\""),
                }
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
                    "DHIS2 at {} did not answer {PING_PATH} within {}; {}, and `--wait SECONDS` \
                     waits longer",
                    self.base,
                    crate::output::human_age(wait),
                    match self.deployed {
                        true => "`chaps logs dhis2` is where the migration shows",
                        false => "check that the URL recorded by `chaps dhis2 use` is right",
                    }
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
            user: self.whoami(),
            waited: started.elapsed(),
        })
    }

    /// Who the requests are sent as: the username for a password, and for a
    /// token whatever [`ME_PATH`] says, since the token itself does not tell.
    ///
    /// A token whose user cannot be read is not a failure - the request that
    /// proved the token works has already been answered - so it is named for
    /// what it is instead.
    fn whoami(&self) -> String {
        if let Some(user) = self.credentials.user() {
            return user.to_string();
        }
        match self.get_json(ME_PATH) {
            Ok(me) if !text_at(&me, "username").is_empty() => text_at(&me, "username"),
            Ok(_) => "the token's user".to_string(),
            Err(why) => {
                crate::output::verbose(&format!(
                    "{ME_PATH} could not be read ({why}), so the token's user is not named"
                ));
                "the token's user".to_string()
            }
        }
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
    /// Who the requests are sent as. See [`Dhis2::whoami`].
    pub user: String,
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
    match deployed {
        true => format!(
            "DHIS2 refused the route: version 42 and later only allow the targets \
             `route.remote_servers_allowed` lists, and {target} has to be one of them; check \
             that line in `dhis2/dhis.conf` and run `chaps restart dhis2`, which recreates it \
             to read the file again"
        ),
        false => format!(
            "DHIS2 refused the route: version 42 and later only allow the targets \
             `route.remote_servers_allowed` lists, and {target} has to be one of them; add it to \
             that line in the `dhis.conf` of the DHIS2 server and restart DHIS2 there"
        ),
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

/// Whether an analytics run has finished on this DHIS2 since it started.
///
/// `GET /api/system/tasks/ANALYTICS_TABLE` is DHIS2's notifier, and the
/// notifier lives in the running process rather than in the database. That is
/// exactly what makes it useful here: a deployment seeded from a dump starts
/// with it empty however much analytics the instance the dump was taken from
/// ever ran, so a completed run in it is a run that happened *here*.
///
/// It is evidence one way only. A restart empties it, so a `false` means chaps
/// has not seen a run rather than that there was none, and every sentence built
/// on it has to say so.
pub fn finished_here(tasks: &serde_json::Value) -> bool {
    tasks
        .as_object()
        .into_iter()
        .flatten()
        .any(|(_, notifications)| matches!(progress_of(notifications), Some(Progress::Done(_))))
}

/// What chaps actually knows about a deployment's analytics tables.
///
/// **`lastAnalyticsTableSuccess` is not a fact about this deployment.** It is a
/// row of DHIS2's own settings, so a seeded deployment inherits it from the
/// dump: measured on the Laos climate demo, a freshly seeded instance reported
/// a last success of `2026-06-16T07:51:00.093` while `analytics_2024` did not
/// exist at all - the table was absent, not empty. That is the failure this
/// command exists to catch, and the timestamp on its own hides it.
///
/// chaps talks HTTP and cannot look at the tables, so it reports what it
/// checked and no more. Two questions settle which of these four it is: what
/// DHIS2 records, and whether a run has finished here, which [`finished_here`]
/// answers.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum AnalyticsEvidence {
    /// DHIS2 records no successful run at all, neither here nor in a dump.
    Never,
    /// A run finished on this DHIS2 since it started. chaps saw it happen.
    RanHere,
    /// DHIS2 records a success and no dump could have brought it: this
    /// deployment's database was migrated from empty, so the record was made
    /// against it.
    Recorded,
    /// DHIS2 records a success, the database was restored from a seed dump
    /// that carries that timestamp, and no run has finished here since DHIS2
    /// started. The timestamp is not evidence either way.
    Unconfirmed,
}

/// Which of the four this deployment is in.
///
/// `ran_here` outranks everything, because it is the only first-hand answer.
/// `seeded` is read from `.chaps/components.yaml` rather than from DHIS2: chaps
/// knows whether it restored a dump into this database, and DHIS2 does not know
/// where its rows came from.
pub fn analytics_evidence(last_success: &str, ran_here: bool, seeded: bool) -> AnalyticsEvidence {
    match (ran_here, last_success.trim().is_empty(), seeded) {
        (true, _, _) => AnalyticsEvidence::RanHere,
        (false, true, _) => AnalyticsEvidence::Never,
        (false, false, true) => AnalyticsEvidence::Unconfirmed,
        (false, false, false) => AnalyticsEvidence::Recorded,
    }
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
    let at_least = bound(version.min_dhis_version.as_deref())
        .map(|min| compare_parts(instance, &min) != std::cmp::Ordering::Less)
        .unwrap_or(true);
    let at_most = bound(version.max_dhis_version.as_deref())
        .map(|max| compare_parts(instance, &max) != std::cmp::Ordering::Greater)
        .unwrap_or(true);
    at_least && at_most
}

/// The version a declared bound names, or `None` where it names none.
///
/// **A blank bound is no bound, on both sides.** The App Hub sends a bound it
/// has not set as an empty string rather than as an absent field: every one of
/// the 32 published versions of the Modeling App carries
/// `"maxDhisVersion": ""`, and `#[serde(default)]` turns that into `Some("")`
/// and not into `None`. Read as a version, `""` has no parts at all, which
/// compares below every instance - so `2.42.6` sat *above* a maximum of
/// nothing, every version was filtered out, and `chaps dhis2 apps` reported
/// that the App Hub publishes no version this DHIS2 can run while a plain
/// `POST /api/appHub/{versionId}` installed it fine.
///
/// Missing, empty, whitespace and unreadable are therefore one answer. A bound
/// chaps cannot compare against is a bound it cannot honour, and refusing on it
/// would refuse everything - the same reason [`pick_version`] takes the newest
/// version when the instance itself did not say what it is.
fn bound(value: Option<&str>) -> Option<Vec<u64>> {
    let parts = dhis_version_parts(value.unwrap_or_default());
    (!parts.is_empty()).then_some(parts)
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
mod tests;
