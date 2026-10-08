//! The footer notes the reducer shows, kept apart so the tests can name them.

/// Footer note shown after enabling a template.
pub const TEMPLATE_WARNING: &str = "templates are not for real forecasts";

/// Footer note shown when a template row is toggled while templates are hidden.
pub const TEMPLATE_HIDDEN_HINT: &str = "press t to show templates first";

/// Footer note shown when `p` is pressed on a row that is not enabled.
pub const PUBLISH_NEEDS_ENABLED_HINT: &str = "enable the model first (space), then press p";

/// Footer note shown when `u` is pressed with nothing to discard.
pub const NOTHING_TO_DISCARD_HINT: &str = "there is nothing to discard";

/// Footer note shown after `u` threw the pending changes away.
pub const DISCARDED_HINT: &str = "the pending changes are gone; nothing was written";

/// Footer note shown when `p` is pressed on a component that is not enabled.
pub const PORT_NEEDS_COMPONENT_HINT: &str = "enable the component first (space), then press p";

/// Footer note shown when `o` is pressed on a component this session has only
/// just enabled.
///
/// The address is a plan until the compose file exists and something is running
/// behind it, so the browser says what is missing rather than opening a port
/// nothing is on yet.
pub const OPEN_NEEDS_SAVING: &str = "press s to save the change first, then `varde up` starts it";

/// Footer note shown when `o` is pressed on a component this deployment does
/// not have.
pub const OPEN_NEEDS_COMPONENT: &str = "enable the component first (space), then save with s";

/// Footer note shown when `p` is pressed on `chap-core`.
///
/// chap-core's host port is the API port, which lives in `project.yaml` rather
/// than in the component block, so this page is not where it is edited.
pub const CORE_PORT_IS_API_PORT: &str =
    "chap-core's host port is the API port; set CHAP_API_PORT in `.env`";

/// Why the component port prompt refuses `auto`.
pub const COMPONENT_HAS_NO_AUTO_PORT: &str =
    "`auto` picks from the model port range; type a number, or none";
