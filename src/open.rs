//! What a browser is pointed at, and how it is pointed there.
//!
//! `varde components list` and the browser's REACH column both print where a
//! component is reached; this is the module that turns that address into the
//! page a person actually wants and hands it to the platform's opener.
//!
//! [`resolve`] is pure and decides nothing about docker: it answers from the
//! recorded state alone, so `varde open` and the browser's `o` cannot word the
//! same component differently. Whether a container is running is a separate
//! question, asked by [`crate::commands::open`] where docker is in reach, and
//! it never changes the URL - only what is said about it.

use crate::components::{Component, Components};

/// Path chap-core serves its interactive API documentation on.
///
/// The page a browser is useful for: the API itself answers JSON, and `/docs`
/// is the one address on that port a person reads. It sits under
/// `CHAP_ROOT_PATH` like every other route, which is why the base URL is
/// passed into [`resolve`] rather than composed here.
pub const API_DOCS_PATH: &str = "/docs";

/// What `varde open chap-core` says it is opening.
pub const CHAP_CORE_PAGE: &str = "chap-core's API documentation";
/// What `varde open ocs` says it is opening.
pub const OCS_PAGE: &str = "the OCS web interface";
/// What `varde open dhis2` says it is opening.
pub const DHIS2_PAGE: &str = "the DHIS2 user interface";

/// Why the object store is never opened, wherever that is said.
///
/// RustFS speaks the S3 API and serves no interface a person reads, so a browser
/// aimed at it gets an XML error document. Publishing a port does not change
/// that - it is there for an S3 client of your own, which is what `--port` is
/// documented as being for - so the line offers the two things that do answer
/// something about the store instead.
pub const NO_WEB_INTERFACE: &str = "the object store speaks the S3 API and serves no web interface, so there is nothing a \
     browser can open; run `varde components enable s3 --port N` to publish it for an S3 client \
     of your own, and `varde status` says whether it is running";

/// What one component offers a browser, worked out from the recorded state.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Openable {
    /// An address a browser can be pointed at.
    Url {
        url: String,
        /// The words for what that address lands on.
        what: &'static str,
        /// Whether it is the public origin the component records rather than a
        /// port on this machine.
        proxied: bool,
    },
    /// Enabled, publishing no host port: reached at this address inside the
    /// deployment and nowhere out here.
    Internal { inside: String },
    /// It has no web interface at all, whatever port it publishes.
    NoWeb,
    /// Not a component of this deployment.
    Off,
}

/// What opening `component` means on this deployment.
///
/// `api_base` is where chap-core's API is reached from this machine, without a
/// trailing slash: the origin plus whatever `CHAP_ROOT_PATH` prefixes it with.
/// It is passed in rather than composed here because the two callers know
/// different amounts about it - `varde open` reads `.env` for the port in
/// effect and the root path, the browser has only the recorded API port, which
/// is exactly what it already prints in its REACH column.
pub fn resolve(component: Component, components: &Components, api_base: &str) -> Openable {
    // The object store answers before the enabled check, because enabling it
    // would not make it openable: sending a reader to `components enable s3`
    // and refusing them again afterwards is worse than saying so once.
    if component == Component::S3 {
        return Openable::NoWeb;
    }
    // A DHIS2 recorded by `varde dhis2 use` is this deployment's DHIS2 as far
    // as anyone opening it is concerned, component or not.
    if component == Component::Dhis2
        && let Some(external) = &components.dhis2_external
    {
        return Openable::Url {
            url: external.url.trim_end_matches('/').to_string(),
            what: DHIS2_PAGE,
            proxied: true,
        };
    }
    if !components.is_enabled(component) {
        return Openable::Off;
    }
    match component {
        Component::ChapCore => Openable::Url {
            url: format!("{}{API_DOCS_PATH}", api_base.trim_end_matches('/')),
            what: CHAP_CORE_PAGE,
            proxied: false,
        },
        // An instance with no host port but a recorded public origin is not
        // unreachable, it is reached there - which is the same reasoning
        // `Components::ocs_reach` prints the base URL where the address would
        // be, and the address a browser wants.
        Component::Ocs => match (components.ocs_url(), &components.ocs.base_url) {
            (Some(url), _) => Openable::Url {
                url,
                what: OCS_PAGE,
                proxied: false,
            },
            (None, Some(base)) => Openable::Url {
                url: base.trim_end_matches('/').to_string(),
                what: OCS_PAGE,
                proxied: true,
            },
            (None, None) => internal(component),
        },
        // DHIS2 records no origin of its own: `server.base.url` lives in
        // `dhis2/dhis.conf`, which is the operator's file, so there is no
        // second copy of it here to open.
        Component::Dhis2 => match components.port_of(Component::Dhis2) {
            Some(port) => Openable::Url {
                url: format!("http://localhost:{port}"),
                what: DHIS2_PAGE,
                proxied: false,
            },
            None => internal(component),
        },
        // Answered above. Left here rather than folded into a catch-all so a
        // fifth component has to say what it offers a browser.
        Component::S3 => Openable::NoWeb,
    }
}

/// The [`Openable::Internal`] answer for a component that publishes no host
/// port, naming the address it is reached at on the compose default network.
fn internal(component: Component) -> Openable {
    Openable::Internal {
        inside: internal_url(component),
    }
}

/// Where a component is reached inside the deployment: its compose service name
/// and the port it listens on.
///
/// The address that is true whether or not a host port is published, and the
/// one a deployment's own reverse proxy is pointed at. chap-core answers here
/// too, for completeness, even though its port is always published and so it is
/// never the component this is printed for.
pub fn internal_url(component: Component) -> String {
    match component {
        Component::ChapCore => format!("http://{}:8000", crate::compose::API_SERVICE),
        Component::Ocs => format!(
            "http://{}:{}",
            crate::compose::OCS_SERVICE,
            crate::components::OCS_CONTAINER_PORT
        ),
        Component::S3 => format!(
            "http://{}:{}",
            crate::compose::S3_SERVICE,
            crate::components::S3_CONTAINER_PORT
        ),
        Component::Dhis2 => format!(
            "http://{}:{}",
            crate::compose::DHIS2_SERVICE,
            crate::compose::render::DHIS2_CONTAINER_PORT
        ),
    }
}

/// Why a component that is off cannot be opened, and the way out.
pub fn off_reason(component: Component) -> String {
    format!(
        "{} is not a component of this deployment, so there is nothing to open; \
         run `varde components enable {}` to add it",
        component.name(),
        component.name()
    )
}

/// Why a component with no host port cannot be opened from this machine, and
/// the way out.
pub fn internal_reason(component: Component, inside: &str) -> String {
    format!(
        "{} publishes no host port, so there is nothing to open from this machine: it is reached \
         at {inside} inside the deployment; run `varde components enable {} --port N` to publish \
         one",
        component.name(),
        component.name()
    )
}

/// Hand a URL to whatever this platform opens URLs with, and say what happened.
///
/// Best effort on purpose: there is no dependency to do it properly, the caller
/// must not block on the child, and a machine with no opener at all - a server
/// over ssh, which is where varde mostly runs - has to hear the URL instead of
/// nothing. That is not a failure, which is why this returns a line rather than
/// an error.
pub fn launch(url: &str) -> String {
    let (command, _) = opener();
    if spawn_opener(url) {
        format!("opening {url}")
    } else {
        format!("no `{command}` on this machine: {url}")
    }
}

/// Whether the platform's opener took the URL, without saying it in words.
///
/// What [`launch`] is made of, and what `varde open` uses directly: it reports
/// the two outcomes in its own sentences rather than in the browser's one-liner,
/// so it needs the answer as a flag.
pub fn spawn_opener(url: &str) -> bool {
    let (command, args) = opener();
    std::process::Command::new(command)
        .args(args)
        .arg(url)
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn()
        .is_ok()
}

/// The command this platform opens URLs with, and the arguments before the URL
/// itself.
pub fn opener() -> (&'static str, &'static [&'static str]) {
    if cfg!(target_os = "macos") {
        ("open", &[])
    } else if cfg!(target_os = "windows") {
        // The empty argument is `start`'s window title, which it would
        // otherwise read the URL as.
        ("cmd", &["/C", "start", ""])
    } else {
        ("xdg-open", &[])
    }
}

#[cfg(test)]
mod tests;
