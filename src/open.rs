//! What a browser is pointed at, and how it is pointed there.
//!
//! `chaps components list` and the browser's REACH column both print where a
//! component is reached; this is the module that turns that address into the
//! page a person actually wants and hands it to the platform's opener.
//!
//! [`resolve`] is pure and decides nothing about docker: it answers from the
//! recorded state alone, so `chaps open` and the browser's `o` cannot word the
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

/// What `chaps open chap-core` says it is opening.
pub const CHAP_CORE_PAGE: &str = "chap-core's API documentation";
/// What `chaps open ocs` says it is opening.
pub const OCS_PAGE: &str = "the OCS web interface";
/// What `chaps open dhis2` says it is opening.
pub const DHIS2_PAGE: &str = "the DHIS2 user interface";

/// Why the object store is never opened, wherever that is said.
///
/// RustFS speaks the S3 API and serves no interface a person reads, so a browser
/// aimed at it gets an XML error document. Publishing a port does not change
/// that - it is there for an S3 client of your own, which is what `--port` is
/// documented as being for - so the line offers the two things that do answer
/// something about the store instead.
pub const NO_WEB_INTERFACE: &str = "the object store speaks the S3 API and serves no web interface, so there is nothing a \
     browser can open; run `chaps components enable s3 --port N` to publish it for an S3 client \
     of your own, and `chaps status` says whether it is running";

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
/// different amounts about it - `chaps open` reads `.env` for the port in
/// effect and the root path, the browser has only the recorded API port, which
/// is exactly what it already prints in its REACH column.
pub fn resolve(component: Component, components: &Components, api_base: &str) -> Openable {
    // The object store answers before the enabled check, because enabling it
    // would not make it openable: sending a reader to `components enable s3`
    // and refusing them again afterwards is worse than saying so once.
    if component == Component::S3 {
        return Openable::NoWeb;
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
         run `chaps components enable {}` to add it",
        component.name(),
        component.name()
    )
}

/// Why a component with no host port cannot be opened from this machine, and
/// the way out.
pub fn internal_reason(component: Component, inside: &str) -> String {
    format!(
        "{} publishes no host port, so there is nothing to open from this machine: it is reached \
         at {inside} inside the deployment; run `chaps components enable {} --port N` to publish \
         one",
        component.name(),
        component.name()
    )
}

/// Hand a URL to whatever this platform opens URLs with, and say what happened.
///
/// Best effort on purpose: there is no dependency to do it properly, the caller
/// must not block on the child, and a machine with no opener at all - a server
/// over ssh, which is where chaps mostly runs - has to hear the URL instead of
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
/// What [`launch`] is made of, and what `chaps open` uses directly: it reports
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
mod tests {
    use super::*;
    use crate::components::{Dhis2Component, OcsComponent, S3Component};

    fn url_of(openable: &Openable) -> Option<&str> {
        match openable {
            Openable::Url { url, .. } => Some(url.as_str()),
            _ => None,
        }
    }

    /// chap-core's useful page is `/docs`, not the origin: the API answers JSON
    /// everywhere else.
    #[test]
    fn chap_core_opens_its_api_documentation() {
        let components = Components::default();
        assert_eq!(
            resolve(Component::ChapCore, &components, "http://localhost:18000"),
            Openable::Url {
                url: "http://localhost:18000/docs".to_string(),
                what: CHAP_CORE_PAGE,
                proxied: false,
            }
        );
    }

    /// `CHAP_ROOT_PATH` moves the whole API, `/docs` with it, and a trailing
    /// slash on the base must not double up.
    #[test]
    fn the_root_path_prefixes_the_documentation() {
        let components = Components::default();
        assert_eq!(
            url_of(&resolve(
                Component::ChapCore,
                &components,
                "http://localhost:8000/master"
            )),
            Some("http://localhost:8000/master/docs")
        );
        assert_eq!(
            url_of(&resolve(
                Component::ChapCore,
                &components,
                "http://localhost:8000/"
            )),
            Some("http://localhost:8000/docs")
        );
    }

    /// A published component opens at its root: that is where both web
    /// interfaces live.
    #[test]
    fn a_published_component_opens_at_its_host_port() {
        let components = Components {
            ocs: OcsComponent {
                enabled: true,
                port: Some(9010),
                ..OcsComponent::default()
            },
            dhis2: Dhis2Component {
                enabled: true,
                port: Some(18080),
                ..Dhis2Component::default()
            },
            ..Components::default()
        };
        let base = "http://localhost:8000";
        assert_eq!(
            url_of(&resolve(Component::Ocs, &components, base)),
            Some("http://localhost:9010")
        );
        assert_eq!(
            url_of(&resolve(Component::Dhis2, &components, base)),
            Some("http://localhost:18080")
        );
    }

    /// No host port and no recorded origin: reached inside the deployment, so
    /// nothing is opened and the internal address is what there is to say.
    #[test]
    fn a_component_with_no_host_port_is_reached_inside_the_deployment() {
        let components = Components {
            ocs: OcsComponent {
                enabled: true,
                port: None,
                ..OcsComponent::default()
            },
            dhis2: Dhis2Component {
                enabled: true,
                port: None,
                ..Dhis2Component::default()
            },
            ..Components::default()
        };
        let base = "http://localhost:8000";
        assert_eq!(
            resolve(Component::Ocs, &components, base),
            Openable::Internal {
                inside: "http://ocs:9000".to_string()
            }
        );
        assert_eq!(
            resolve(Component::Dhis2, &components, base),
            Openable::Internal {
                inside: "http://dhis2:8080".to_string()
            }
        );
        let reason = internal_reason(Component::Ocs, "http://ocs:9000");
        assert!(reason.contains("http://ocs:9000"), "{reason}");
        assert!(
            reason.contains("`chaps components enable ocs --port N`"),
            "{reason}"
        );
    }

    /// An unpublished instance behind a proxy is reached at the origin it
    /// records, which is an address a browser can be pointed at.
    #[test]
    fn an_unpublished_ocs_opens_the_origin_it_records() {
        let components = Components {
            ocs: OcsComponent {
                enabled: true,
                port: None,
                base_url: Some("https://climate.example.org/".to_string()),
                ..OcsComponent::default()
            },
            ..Components::default()
        };
        assert_eq!(
            resolve(Component::Ocs, &components, "http://localhost:8000"),
            Openable::Url {
                url: "https://climate.example.org".to_string(),
                what: OCS_PAGE,
                proxied: true,
            }
        );
    }

    /// A component this deployment does not have is refused, and the refusal
    /// names the command that adds it.
    #[test]
    fn a_component_that_is_off_is_refused() {
        let components = Components::default();
        let base = "http://localhost:8000";
        assert_eq!(resolve(Component::Ocs, &components, base), Openable::Off);
        assert_eq!(resolve(Component::Dhis2, &components, base), Openable::Off);

        let mut off = components.clone();
        off.chap_core.enabled = false;
        assert_eq!(resolve(Component::ChapCore, &off, base), Openable::Off);

        let reason = off_reason(Component::Dhis2);
        assert!(
            reason.contains("`chaps components enable dhis2`"),
            "{reason}"
        );
    }

    /// The object store has no web interface, so it says so whether it is on,
    /// off, or publishing a port: enabling it would not make it openable.
    #[test]
    fn the_object_store_is_never_opened() {
        let mut components = Components::default();
        let base = "http://localhost:8000";
        assert_eq!(resolve(Component::S3, &components, base), Openable::NoWeb);
        components.s3 = S3Component {
            enabled: true,
            port: Some(9002),
        };
        assert_eq!(resolve(Component::S3, &components, base), Openable::NoWeb);
        assert!(NO_WEB_INTERFACE.contains("S3 API"));
    }

    /// Every component has an internal address, and none of them is empty or
    /// points at the host.
    #[test]
    fn every_component_names_where_it_is_reached_inside() {
        for component in Component::ALL {
            let url = internal_url(*component);
            assert!(url.starts_with("http://"), "{url}");
            assert!(!url.contains("localhost"), "{url}");
        }
    }

    /// Spawning the opener is not tested - it would open a browser on the
    /// machine running the suite - but the command it would spawn is.
    #[test]
    fn every_platform_names_an_opener() {
        let (command, args) = opener();
        assert!(!command.is_empty());
        assert!(args.iter().all(|arg| !arg.contains("://")));
    }
}
