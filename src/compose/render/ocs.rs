//! The `ocs` component, the `s3` store beside it, and the scaffolded OCS
//! instance config.

use super::labels::{ROLE_OCS, ROLE_S3, labels_block};
use super::{fill, normalize_newlines};
use crate::components::{
    OCS_BASE_URL_ENV_VAR, OCS_CONFIG_FILE, OCS_CONTAINER_PORT, OCS_DATA_SOURCE_ENV_VARS, OCS_DIR,
    OCS_IMAGE, OCS_PLUGINS_DIR, OCS_PLUGINS_TARGET, OCS_TAG_ENV_VAR, S3_ACCESS_KEY_ENV_VAR,
    S3_BUCKET, S3_CONTAINER_PORT, S3_IMAGE, S3_SECRET_KEY_ENV_VAR, S3_TAG_ENV_VAR,
};
use crate::compose::spec::{OcsConfigSpec, OcsSpec, S3Spec};
use std::sync::LazyLock;

/// The `ocs` component, as a compose file of its own.
pub(super) static OCS_TEMPLATE: LazyLock<String> =
    LazyLock::new(|| normalize_newlines(include_str!("../templates/compose.ocs.yml")));
/// The `s3` component, likewise.
pub(super) static S3_TEMPLATE: LazyLock<String> =
    LazyLock::new(|| normalize_newlines(include_str!("../templates/compose.s3.yml")));
/// The OCS instance config `init --with ocs` scaffolds.
static OCS_CONFIG_TEMPLATE: LazyLock<String> =
    LazyLock::new(|| normalize_newlines(include_str!("../templates/climate-service.yaml")));

/// Named volume holding OCS's data directory.
pub const OCS_VOLUME: &str = "ocs_data";
/// Named volume holding the object store's data directory.
pub const S3_VOLUME: &str = "s3_data";
/// Region the bucket request is signed for. RustFS ignores it, but SigV4 has
/// to name one.
const S3_REGION: &str = "us-east-1";

/// The marker line the scaffolded `ocs/climate-service.yaml` carries while it
/// still holds OCS's example values.
///
/// `chaps doctor` looks for exactly this, and the note tells the reader to
/// delete it once the file is theirs, so the check goes quiet by itself.
pub const OCS_EXAMPLE_MARKER: &str =
    "# These are the Laos example values, the country of the DHIS2 demo.";

/// Render `compose.ocs.yml`: the `ocs` component.
///
/// Three blocks are conditional. The host port is published only when the
/// component records one, so an instance behind a reverse proxy is `expose`d on
/// the compose network and nowhere else. The plugin mount appears only when the
/// project has an `ocs/plugins/` to mount, since compose fails to start a
/// service whose bind source does not exist. And the `S3_*` block only appears
/// when the `s3` component is on: OCS does not read those variables yet - it is
/// about to - so they are written as forward-looking settings with a comment
/// that says as much, rather than being left out until the day the contract
/// lands.
///
/// The five dataset credential variables are unconditional, all of them
/// `${VAR:-}`. OCS reads each one as `os.getenv(...) or <the file>`, so an empty
/// value is one it does not have: passing them always costs a deployment that
/// sets none of them nothing, and means the operator only ever has to put a
/// credential in `.env` rather than in `.env` and then in `.chaps/` as well.
pub fn render_ocs(spec: &OcsSpec) -> String {
    let s3_lines = if spec.s3 {
        format!(
            "      # Forward-looking: OCS does not read these yet.\n\
             \x20     S3_ENDPOINT: http://s3:{S3_CONTAINER_PORT}\n\
             \x20     {S3_ACCESS_KEY_ENV_VAR}: ${{{S3_ACCESS_KEY_ENV_VAR}:-}}\n\
             \x20     {S3_SECRET_KEY_ENV_VAR}: ${{{S3_SECRET_KEY_ENV_VAR}:-}}\n\
             \x20     S3_BUCKET: {S3_BUCKET}\n"
        )
    } else {
        String::new()
    };
    let mut data_source_lines = String::from(
        "      # Dataset credentials, empty unless .env sets them; see docs/components.md.\n",
    );
    for var in OCS_DATA_SOURCE_ENV_VARS {
        data_source_lines.push_str(&format!("      {var}: ${{{var}:-}}\n"));
    }
    let mut port_lines = format!("    expose:\n      - \"{OCS_CONTAINER_PORT}\"\n");
    if let Some(port) = spec.host_port {
        port_lines.push_str(&format!(
            "    ports:\n      - \"{port}:{OCS_CONTAINER_PORT}\"\n"
        ));
    }
    let plugins_line = if spec.plugins {
        format!("      - ./{OCS_DIR}/{OCS_PLUGINS_DIR}:{OCS_PLUGINS_TARGET}:ro\n")
    } else {
        String::new()
    };
    fill(
        &OCS_TEMPLATE,
        &[
            (
                "LABELS",
                &labels_block(ROLE_OCS, None, spec.group.as_deref()),
            ),
            ("IMAGE", OCS_IMAGE),
            ("TAG_VAR", OCS_TAG_ENV_VAR),
            ("IMAGE_TAG", &spec.image_tag),
            ("PORT_LINES", &port_lines),
            ("CONTAINER_PORT", &OCS_CONTAINER_PORT.to_string()),
            ("BASE_URL_VAR", OCS_BASE_URL_ENV_VAR),
            ("BASE_URL", spec.base_url.as_deref().unwrap_or_default()),
            ("DATA_SOURCE_LINES", &data_source_lines),
            ("S3_LINES", &s3_lines),
            ("CONFIG_PATH", &format!("{OCS_DIR}/{OCS_CONFIG_FILE}")),
            ("PLUGINS_LINE", &plugins_line),
            ("VOLUME", OCS_VOLUME),
        ],
    )
}

/// Render `compose.s3.yml`: the `s3` component.
pub fn render_s3(spec: &S3Spec) -> String {
    let port_lines = match spec.host_port {
        None => format!("    expose:\n      - \"{S3_CONTAINER_PORT}\"\n"),
        Some(port) => format!(
            "    expose:\n      - \"{S3_CONTAINER_PORT}\"\n\
             \x20   ports:\n      - \"{port}:{S3_CONTAINER_PORT}\"\n"
        ),
    };
    fill(
        &S3_TEMPLATE,
        &[
            (
                "LABELS",
                &labels_block(ROLE_S3, None, spec.group.as_deref()),
            ),
            ("IMAGE", S3_IMAGE),
            ("TAG_VAR", S3_TAG_ENV_VAR),
            ("IMAGE_TAG", &spec.image_tag),
            ("PORT_LINES", &port_lines),
            ("CONTAINER_PORT", &S3_CONTAINER_PORT.to_string()),
            ("ACCESS_KEY_VAR", S3_ACCESS_KEY_ENV_VAR),
            ("SECRET_KEY_VAR", S3_SECRET_KEY_ENV_VAR),
            ("BUCKET", S3_BUCKET),
            ("REGION", S3_REGION),
            ("VOLUME", S3_VOLUME),
        ],
    )
}

/// Render the scaffolded `ocs/climate-service.yaml`.
///
/// Written once, when the component is enabled, and never again: it is the
/// operator's file from that moment on.
pub fn render_ocs_config(spec: &OcsConfigSpec) -> String {
    // The token starts its own line and carries its own newlines, so a file
    // written from real values leaves no blank comment block behind.
    let note = if spec.example {
        format!("{OCS_EXAMPLE_MARKER} Edit them, then delete this note.\n")
    } else {
        String::new()
    };
    fill(
        &OCS_CONFIG_TEMPLATE,
        &[
            ("EXAMPLE_NOTE", &note),
            ("ID", &spec.id),
            ("NAME", &spec.name),
            ("EXTENT_NAME", &spec.extent_name),
            ("BBOX", &spec.bbox),
            ("COUNTRY_CODE", &spec.country_code),
        ],
    )
}
