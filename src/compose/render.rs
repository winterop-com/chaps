//! Filling the compose text templates.
//!
//! The templates are plain text with `@KEY@` tokens rather than a template
//! engine, because compose's own `${VAR:-default}` syntax collides with the
//! usual `{{ }}` delimiters and a serde round trip would drop the comments.
//!
//! Owned by agent B.

use crate::compose::spec::{BaseSpec, EnvSpec, OverlaySpec};

/// Render the base `compose.yml`.
pub fn render_base(_spec: &BaseSpec) -> String {
    String::new()
}

/// Render the generated `.env`.
pub fn render_env(_spec: &EnvSpec) -> String {
    String::new()
}

/// Render one `compose.<service_id>.yml` overlay.
pub fn render_overlay(_spec: &OverlaySpec) -> String {
    String::new()
}

/// Render `compose.marketplace.yml` from the ordered overlay file names.
///
/// With no overlays this must emit `services: {}` rather than an empty
/// `include:` list, which compose rejects.
pub fn render_umbrella(_files: &[String]) -> String {
    String::new()
}

/// Replace every `@KEY@` token in `template` with its value.
pub(crate) fn fill(template: &str, vars: &[(&str, &str)]) -> String {
    let mut out = template.to_string();
    for (key, value) in vars {
        out = out.replace(&format!("@{key}@"), value);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fill_replaces_every_token() {
        let out = fill("a=@A@ b=@B@ a=@A@", &[("A", "1"), ("B", "2")]);
        assert_eq!(out, "a=1 b=2 a=1");
    }

    #[test]
    fn fill_leaves_compose_variables_alone() {
        let out = fill("${TAG:-sha-1234567} @TAG@", &[("TAG", "x")]);
        assert_eq!(out, "${TAG:-sha-1234567} x");
    }
}
