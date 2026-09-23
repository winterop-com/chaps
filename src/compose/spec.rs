//! The values the compose templates are filled with.
//!
//! The types are frozen here; the constructors are owned by agent B.

use crate::components::Components;
use crate::compose::{AMD64_PLATFORM, overrides, tag_env_var, volume_name};
use crate::project::EnabledModel;
use crate::registry::{Model, Version};

/// Values for the base `compose.yml`.
#[derive(Debug, Clone)]
pub struct BaseSpec {
    pub cli_version: String,
    /// chap-core's own `compose.ghcr.yml`, when the project records a tag it
    /// was downloaded from; `None` renders the copy compiled into the binary.
    pub upstream: Option<UpstreamCompose>,
}

/// chap-core's `compose.ghcr.yml` at one tag, as fetched.
///
/// The body is used verbatim - upstream is the source of truth for the base
/// stack, and the recorded SHA-256 only means something if nothing rewrites
/// it. `chaps` adds its own two header lines in front and nothing else.
#[derive(Debug, Clone)]
pub struct UpstreamCompose {
    pub tag: String,
    pub body: String,
}

/// Values for the generated `.env`.
#[derive(Debug, Clone)]
pub struct EnvSpec {
    pub postgres_user: String,
    pub postgres_password: String,
    pub postgres_db: String,
    /// `Some` when the chap image is pinned to something other than `latest`.
    pub chap_image_tag: Option<String>,
    /// Host port chap-core's API is published on. Written as an active line
    /// even at the default, so the one port the stack publishes is visible
    /// where an operator would look for it.
    pub api_port: u16,
    /// API token to write as an active `CHAP_API_TOKEN` line; `None` leaves
    /// the commented placeholder, which is chap-core's "no authentication".
    pub api_token: Option<String>,
    /// The same for `SERVICEKIT_REGISTRATION_KEY`. It travels with the API
    /// token: a protected API rejects an unauthenticated registration, so a
    /// deployment that has one needs the other.
    pub registration_key: Option<String>,
    /// `(<ID>_IMAGE_TAG, tag)` pairs written as commented-out pins.
    pub model_tag_pins: Vec<(String, String)>,
    pub cli_version: String,
}

/// Values for `compose.ocs.yml`.
#[derive(Debug, Clone)]
pub struct OcsSpec {
    /// Host port OCS is published on.
    pub host_port: u16,
    /// The tag the `${OCS_IMAGE_TAG:-...}` default carries.
    pub image_tag: String,
    /// Whether the `s3` component is on, which is what decides if the service
    /// gets the (forward-looking) `S3_*` variables.
    pub s3: bool,
    pub cli_version: String,
}

impl OcsSpec {
    /// The spec a project's components describe.
    pub fn from_components(components: &Components, cli_version: &str) -> OcsSpec {
        OcsSpec {
            host_port: components.ocs.port,
            image_tag: components.ocs.image_tag.clone(),
            s3: components.s3.enabled,
            cli_version: cli_version.to_string(),
        }
    }
}

/// Values for `compose.s3.yml`.
#[derive(Debug, Clone)]
pub struct S3Spec {
    /// Host port the object store is published on, or `None` for the default:
    /// reachable only inside the compose network, which is where OCS is.
    pub host_port: Option<u16>,
    pub image_tag: String,
    pub cli_version: String,
}

impl S3Spec {
    /// The spec a project's components describe.
    pub fn from_components(components: &Components, cli_version: &str) -> S3Spec {
        S3Spec {
            host_port: components.s3.port,
            image_tag: crate::components::S3_DEFAULT_TAG.to_string(),
            cli_version: cli_version.to_string(),
        }
    }
}

/// Values for the scaffolded `ocs/climate-service.yaml`.
///
/// Everything but `example` comes from the `--ocs-*` flags; without them the
/// file is OCS's own Sierra Leone example, and `example` is what says so - in
/// the file, and to `chaps doctor`.
#[derive(Debug, Clone)]
pub struct OcsConfigSpec {
    /// STAC catalog id, derived from the name.
    pub id: String,
    /// Human-readable instance name.
    pub name: String,
    /// Name of the spatial extent: the country or region.
    pub extent_name: String,
    /// `xmin, ymin, xmax, ymax`, already formatted.
    pub bbox: String,
    /// ISO 3166-1 alpha-3 country code.
    pub country_code: String,
    /// Whether these are the example values rather than someone's own.
    pub example: bool,
}

impl Default for OcsConfigSpec {
    /// OCS's own `climate-service.yaml.example`, which is Sierra Leone.
    fn default() -> OcsConfigSpec {
        OcsConfigSpec {
            id: "sierra-leone-climate-service".to_string(),
            name: "Sierra Leone Climate Service".to_string(),
            extent_name: "Sierra Leone".to_string(),
            bbox: "-13.5, 6.9, -10.1, 10.0".to_string(),
            country_code: "SLE".to_string(),
            example: true,
        }
    }
}

/// Whether one field of a scaffold was given on the command line.
///
/// Kept as three independent options rather than all-or-nothing: someone who
/// only knows the country code still gets it into the file, and the fields
/// they did not give stay the example's, which the note still points at.
#[derive(Debug, Clone, Default)]
pub struct OcsConfigRequest {
    pub name: Option<String>,
    pub country_code: Option<String>,
    pub bbox: Option<String>,
}

impl OcsConfigRequest {
    /// Whether anything at all was asked for.
    pub fn is_empty(&self) -> bool {
        self.name.is_none() && self.country_code.is_none() && self.bbox.is_none()
    }

    /// The scaffold these flags describe, filling the gaps from the example.
    pub fn into_spec(self) -> OcsConfigSpec {
        let example = self.is_empty();
        let mut spec = OcsConfigSpec {
            example,
            ..OcsConfigSpec::default()
        };
        if let Some(name) = self
            .name
            .as_deref()
            .map(str::trim)
            .filter(|n| !n.is_empty())
        {
            spec.extent_name = name.to_string();
            spec.name = format!("{name} Climate Service");
            spec.id = slug(&spec.name);
        }
        if let Some(code) = self
            .country_code
            .as_deref()
            .map(str::trim)
            .filter(|c| !c.is_empty())
        {
            spec.country_code = code.to_ascii_uppercase();
        }
        if let Some(bbox) = self.bbox.as_deref() {
            spec.bbox = bbox
                .split(',')
                .map(str::trim)
                .collect::<Vec<_>>()
                .join(", ");
        }
        spec
    }
}

/// `Sierra Leone Climate Service` -> `sierra-leone-climate-service`.
fn slug(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for ch in text.chars() {
        if ch.is_ascii_alphanumeric() {
            out.push(ch.to_ascii_lowercase());
        } else if !out.ends_with('-') {
            out.push('-');
        }
    }
    out.trim_matches('-').to_string()
}

/// Values for one `compose.<service_id>.yml` overlay.
#[derive(Debug, Clone)]
pub struct OverlaySpec {
    pub id: String,
    pub service_id: String,
    pub display_name: String,
    pub version: String,
    pub repository: String,
    /// Tagless image reference.
    pub image: String,
    pub image_tag: String,
    /// See [`crate::compose::tag_env_var`].
    pub tag_env_var: String,
    /// Host port to publish 8000 on, or `None` for a service that is only
    /// `expose`d on the compose network - the default.
    pub host_port: Option<u16>,
    /// `Some("linux/amd64")` for R-INLA services.
    pub platform: Option<String>,
    pub data_dir: String,
    pub user: String,
    /// See [`crate::compose::volume_name`].
    pub volume_name: String,
    /// Whether to emit an active `SERVICEKIT_REGISTRATION_KEY` line.
    pub registration_key: bool,
    pub cli_version: String,
}

impl OverlaySpec {
    /// Build a spec for a freshly enabled model.
    ///
    /// `data_dir` and `user` override
    /// [`crate::compose::known_override`] and the defaults when given.
    ///
    /// Owned by agent B.
    pub fn from_model(
        m: &Model,
        v: &Version,
        host_port: Option<u16>,
        data_dir: Option<&str>,
        user: Option<&str>,
        cli_version: &str,
    ) -> OverlaySpec {
        let known = overrides::known_override(&m.id);
        OverlaySpec {
            id: m.id.clone(),
            service_id: m.service_id.clone(),
            display_name: m.display_name.clone(),
            version: v.version.clone(),
            repository: m.source.repository.clone(),
            image: m.source.image.clone(),
            image_tag: v.image_tag.clone(),
            tag_env_var: tag_env_var(&m.id),
            host_port,
            // Every model image the marketplace publishes today is amd64-only
            // (checked with `docker manifest inspect`; only the simple multistep
            // model also ships arm64), and chap-core itself is amd64-only, so
            // the whole stack already runs as linux/amd64. Pinning every
            // overlay makes an arm64 host pull the right variant instead of
            // failing with "no matching manifest".
            platform: Some(AMD64_PLATFORM.to_string()),
            data_dir: data_dir
                .map(str::to_string)
                .or_else(|| known.map(|k| k.data_dir.to_string()))
                .unwrap_or_else(|| overrides::DEFAULT_DATA_DIR.to_string()),
            user: user
                .map(str::to_string)
                .or_else(|| known.map(|k| k.user.to_string()))
                .unwrap_or_else(|| overrides::DEFAULT_USER.to_string()),
            volume_name: volume_name(&m.id),
            // chap-core only enforces a registration key when it has one
            // configured, so the generated overlay leaves the line commented.
            registration_key: false,
            cli_version: cli_version.to_string(),
        }
    }

    /// Rebuild a spec from recorded state, as `chaps sync` does.
    ///
    /// Everything that affects the running service comes from the recorded
    /// entry; the marketplace only supplies the labels in the header.
    pub fn from_enabled(id: &str, e: &EnabledModel, m: &Model, cli_version: &str) -> OverlaySpec {
        OverlaySpec {
            display_name: m.display_name.clone(),
            repository: m.source.repository.clone(),
            ..OverlaySpec::from_enabled_without_registry(id, e, cli_version)
        }
    }

    /// [`OverlaySpec::from_enabled`] for a model the registry no longer lists:
    /// the header names the id in place of the display name and the image in
    /// place of the repository.
    pub fn from_enabled_without_registry(
        id: &str,
        e: &EnabledModel,
        cli_version: &str,
    ) -> OverlaySpec {
        OverlaySpec {
            id: id.to_string(),
            service_id: e.service_id.clone(),
            display_name: id.to_string(),
            version: e.version.clone(),
            repository: e.image.clone(),
            image: e.image.clone(),
            image_tag: e.image_tag.clone(),
            tag_env_var: tag_env_var(id),
            host_port: e.host_port,
            platform: e.platform.clone(),
            data_dir: e.data_dir.clone(),
            user: e.user.clone(),
            volume_name: volume_name(id),
            registration_key: false,
            cli_version: cli_version.to_string(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::registry::{Channel, Registry, VersionSelector, load_embedded};

    fn registry() -> Registry {
        load_embedded().unwrap()
    }

    fn spec_for(r: &Registry, id: &str, data_dir: Option<&str>, user: Option<&str>) -> OverlaySpec {
        let m = r.get(id).unwrap();
        let v = m
            .resolve(&VersionSelector::Channel(Channel::Stable))
            .unwrap();
        OverlaySpec::from_model(m, v, Some(5001), data_dir, user, "0.1.0")
    }

    #[test]
    fn from_model_copies_the_marketplace_entry() {
        let r = registry();
        let spec = spec_for(&r, "chapkit_ewars_model", None, None);
        assert_eq!(spec.id, "chapkit_ewars_model");
        assert_eq!(spec.service_id, "chapkit-ewars-model");
        assert_eq!(spec.display_name, "CHAP-EWARS");
        assert_eq!(spec.version, "1.0.0");
        assert_eq!(spec.image, "ghcr.io/chap-models/chapkit_ewars_model");
        assert_eq!(spec.image_tag, "sha-fa880a1");
        assert_eq!(spec.tag_env_var, "CHAPKIT_EWARS_MODEL_IMAGE_TAG");
        assert_eq!(spec.volume_name, "ck_chapkit_ewars_model_data");
        assert_eq!(spec.host_port, Some(5001));
        assert!(!spec.registration_key);
        assert_eq!(spec.cli_version, "0.1.0");
    }

    #[test]
    fn from_model_pins_every_overlay_to_amd64() {
        let r = registry();
        for id in ["chapkit_ewars_model", "chapkit_simple_multistep_model"] {
            assert_eq!(
                spec_for(&r, id, None, None).platform.as_deref(),
                Some("linux/amd64"),
                "{id}"
            );
        }
    }

    #[test]
    fn data_dir_and_user_prefer_the_explicit_value_then_the_table() {
        let r = registry();
        // The table: ewars writes to /app/data.
        let table = spec_for(&r, "chapkit_ewars_model", None, None);
        assert_eq!(table.data_dir, "/app/data");
        assert_eq!(table.user, "chapkit:chapkit");

        // The flags win over the table.
        let explicit = spec_for(
            &r,
            "chapkit_ewars_model",
            Some("/srv/data"),
            Some("1000:1000"),
        );
        assert_eq!(explicit.data_dir, "/srv/data");
        assert_eq!(explicit.user, "1000:1000");

        // Nothing known: the chapkit defaults.
        let fallback = spec_for(&r, "auto_arima_chapkit", None, None);
        assert_eq!(fallback.data_dir, overrides::DEFAULT_DATA_DIR);
        assert_eq!(fallback.user, overrides::DEFAULT_USER);
    }

    #[test]
    fn from_enabled_replays_the_recorded_state() {
        let r = registry();
        let m = r.get("chapkit_ewars_model").unwrap();
        let enabled = EnabledModel {
            service_id: "chapkit-ewars-model".into(),
            image: "ghcr.io/chap-models/chapkit_ewars_model".into(),
            image_tag: "sha-0000000".into(),
            version: "0.9.0".into(),
            channel: None,
            host_port: Some(5007),
            data_dir: "/srv/data".into(),
            user: "1000:1000".into(),
            platform: None,
            compose_file: "compose.chapkit-ewars-model.yml".into(),
        };
        let spec = OverlaySpec::from_enabled("chapkit_ewars_model", &enabled, m, "0.1.0");
        assert_eq!(spec.host_port, Some(5007));

        // A model that publishes nothing replays as such.
        let internal = EnabledModel {
            host_port: None,
            ..enabled.clone()
        };
        let spec = OverlaySpec::from_enabled("chapkit_ewars_model", &internal, m, "0.1.0");
        assert_eq!(spec.host_port, None);
        let spec = OverlaySpec::from_enabled_without_registry("gone", &internal, "0.1.0");
        assert_eq!(spec.host_port, None);
        let spec = OverlaySpec::from_enabled("chapkit_ewars_model", &enabled, m, "0.1.0");
        // Recorded values win; the marketplace only supplies the labels.
        assert_eq!(spec.image_tag, "sha-0000000");
        assert_eq!(spec.version, "0.9.0");
        assert_eq!(spec.host_port, Some(5007));
        assert_eq!(spec.data_dir, "/srv/data");
        assert_eq!(spec.user, "1000:1000");
        assert_eq!(spec.platform, None);
        assert_eq!(spec.display_name, "CHAP-EWARS");
        assert_eq!(spec.repository, m.source.repository);
        assert_eq!(spec.tag_env_var, "CHAPKIT_EWARS_MODEL_IMAGE_TAG");
    }
}
