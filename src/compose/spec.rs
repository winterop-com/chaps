//! The values the compose templates are filled with.
//!
//! The types are frozen here; the constructors are owned by agent B.

use crate::project::EnabledModel;
use crate::registry::{Model, Version};

/// Values for the base `compose.yml` header.
#[derive(Debug, Clone)]
pub struct BaseSpec {
    pub cli_version: String,
}

/// Values for the generated `.env`.
#[derive(Debug, Clone)]
pub struct EnvSpec {
    pub postgres_user: String,
    pub postgres_password: String,
    pub postgres_db: String,
    /// `Some` when the chap image is pinned to something other than `latest`.
    pub chap_image_tag: Option<String>,
    /// `(<ID>_IMAGE_TAG, tag)` pairs written as commented-out pins.
    pub model_tag_pins: Vec<(String, String)>,
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
    pub host_port: u16,
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
        _m: &Model,
        _v: &Version,
        _host_port: u16,
        _data_dir: Option<&str>,
        _user: Option<&str>,
        _cli_version: &str,
    ) -> OverlaySpec {
        unimplemented_spec()
    }

    /// Rebuild a spec from recorded state, e.g. when regenerating overlays.
    ///
    /// Owned by agent B.
    pub fn from_enabled(
        _id: &str,
        _e: &EnabledModel,
        _m: &Model,
        _cli_version: &str,
    ) -> OverlaySpec {
        unimplemented_spec()
    }
}

/// Placeholder returned by the not-yet-implemented constructors.
///
/// It is deliberately inert rather than a panic, so a caller that reaches it
/// during development renders an obviously wrong overlay instead of aborting.
fn unimplemented_spec() -> OverlaySpec {
    OverlaySpec {
        id: String::new(),
        service_id: String::new(),
        display_name: String::new(),
        version: String::new(),
        repository: String::new(),
        image: String::new(),
        image_tag: String::new(),
        tag_env_var: String::new(),
        host_port: 0,
        platform: None,
        data_dir: String::new(),
        user: String::new(),
        volume_name: String::new(),
        registration_key: false,
        cli_version: String::new(),
    }
}
