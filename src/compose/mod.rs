//! Generating the compose files that make up a deployment.

pub mod apply;
pub mod overrides;
pub mod ports;
pub mod render;
pub mod spec;
pub mod sync;

pub use apply::{ApplyReport, EnableRequest, Selection, apply};
pub use render::{render_base, render_env};
pub use sync::{SyncReport, sync};

/// Platform pinned for services built on the R-INLA runtime.
pub const AMD64_PLATFORM: &str = "linux/amd64";

/// Overlay file name for a model service: `compose.<service_id>.yml`.
pub fn overlay_filename(service_id: &str) -> String {
    format!("compose.{service_id}.yml")
}

/// Environment variable that overrides a model's image tag:
/// the uppercased marketplace id with non-alphanumerics folded to `_`,
/// suffixed with `_IMAGE_TAG`.
pub fn tag_env_var(id: &str) -> String {
    let mut out = String::with_capacity(id.len() + 10);
    for c in id.chars() {
        if c.is_ascii_alphanumeric() {
            out.push(c.to_ascii_uppercase());
        } else {
            out.push('_');
        }
    }
    out.push_str("_IMAGE_TAG");
    out
}

/// Named volume holding a model's data directory: `ck_<id>_data`.
pub fn volume_name(id: &str) -> String {
    format!("ck_{id}_data")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn overlay_filename_uses_the_service_id() {
        assert_eq!(
            overlay_filename("chapkit-ewars-model"),
            "compose.chapkit-ewars-model.yml"
        );
        assert_eq!(
            overlay_filename("auto-arima-chapkit"),
            "compose.auto-arima-chapkit.yml"
        );
    }

    #[test]
    fn tag_env_var_uppercases_and_folds_separators() {
        assert_eq!(
            tag_env_var("chapkit_ewars_model"),
            "CHAPKIT_EWARS_MODEL_IMAGE_TAG"
        );
        assert_eq!(
            tag_env_var("chapkit-ewars-model"),
            "CHAPKIT_EWARS_MODEL_IMAGE_TAG"
        );
        assert_eq!(
            tag_env_var("auto_arima_chapkit"),
            "AUTO_ARIMA_CHAPKIT_IMAGE_TAG"
        );
        assert_eq!(tag_env_var("a.b c"), "A_B_C_IMAGE_TAG");
        // The chap-core deployment test matches ^\$\{[A-Z_]+_IMAGE_TAG:-...\}$.
        assert!(
            tag_env_var("chapkit_minimalist_example_py")
                .chars()
                .all(|c| c.is_ascii_uppercase() || c == '_')
        );
    }

    #[test]
    fn volume_name_is_prefixed_and_suffixed() {
        assert_eq!(
            volume_name("chapkit_ewars_model"),
            "ck_chapkit_ewars_model_data"
        );
        assert_eq!(
            volume_name("auto_arima_chapkit"),
            "ck_auto_arima_chapkit_data"
        );
    }
}
