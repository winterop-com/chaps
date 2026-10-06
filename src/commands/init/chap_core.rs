//! The chap-core tag `init` settles on, and where `compose.yml` comes from:
//! a release, the copy built into this binary, or a checkout.

use super::MOVING_DEFAULT_TAG;
use crate::chapcore;
use crate::commands::Ctx;
use crate::error::Result;
use crate::project::{VARDE_DIR, ComposeSource, cached_compose_file};
use std::path::Path;

/// What `.varde/project.yaml` records as the chap-core tag of a deployment
/// built from a checkout: there is no release behind it.
pub(super) const CHECKOUT_TAG: &str = "checkout";

/// The absolute path of a chap-core checkout, refused unless it has what a
/// build needs: the API's `Dockerfile`, the worker's `Dockerfile.worker` and
/// the `compose.ghcr.yml` that `compose.yml` is rendered from.
pub(super) fn checkout_source(path: &Path) -> Result<String> {
    let path = std::path::absolute(path)
        .map_err(|e| anyhow::anyhow!("resolving {}: {e}", path.display()))?;
    let missing: Vec<&str> = [
        "Dockerfile",
        "Dockerfile.worker",
        crate::project::CHECKOUT_COMPOSE,
    ]
    .into_iter()
    .filter(|name| !path.join(name).is_file())
    .collect();
    if !missing.is_empty() {
        return Err(anyhow::anyhow!(
            "{} is not a chap-core checkout: it has no {}; pass the directory you cloned \
             github.com/dhis2-chap/chap-core into",
            path.display(),
            missing.join(", ")
        ));
    }
    Ok(path.to_string_lossy().into_owned())
}

/// The chap-core tag `init` settled on, and where `compose.yml` comes from.
#[derive(Debug, Clone)]
pub(super) struct ChapCore {
    /// The tag written to `.env` and `.varde/project.yaml`.
    pub(super) tag: String,
    pub(super) source: ComposeSource,
    /// The downloaded `compose.ghcr.yml`, when one has to be written into
    /// `.varde/`; `None` when the copy is already there or is the embedded one.
    pub(super) cached: Option<String>,
}

/// Decide the chap-core tag and the compose file that goes with it.
///
/// `latest` is a moving tag: two `init` runs a month apart would deploy
/// different code from the same state file, and `varde update` would have
/// nothing to compare. So the default is resolved to the release it points at
/// right now, and the compose file that release publishes is downloaded with
/// it. Every step degrades to a warning: `--offline`, an unreachable GitHub, a
/// rate-limited API or a compose file that does not parse all end with the
/// literal tag and the copy compiled into this binary, which still deploys.
pub(super) fn resolve_chap_core(ctx: &Ctx, dir: &Path, requested: &str) -> ChapCore {
    let offline = ctx.registry.offline;
    let timeout = ctx.registry.timeout;
    let embedded = |tag: String| ChapCore {
        tag,
        source: ComposeSource::Embedded,
        cached: None,
    };

    let mut tag = requested.trim().to_string();
    if tag == MOVING_DEFAULT_TAG {
        match resolve_latest(offline, timeout) {
            Ok(release) => tag = release,
            Err(reason) => {
                crate::output::warn(&format!(
                    "{reason}; the chap-core tag stays `{MOVING_DEFAULT_TAG}`, so this deployment \
                     follows that moving tag - pass `--chap-tag vX.Y.Z` to pin a release"
                ));
                return embedded(tag);
            }
        }
    }

    // An unresolved `latest` names no fixed document, so there is no copy of
    // compose.ghcr.yml to pin either.
    if tag == MOVING_DEFAULT_TAG {
        return embedded(tag);
    }

    // A copy of this exact tag already in `.varde/` (an earlier `init`, or an
    // `init --force` over a working deployment) is reused rather than
    // re-downloaded, which is also what makes `--offline` reproducible here.
    let path = dir.join(VARDE_DIR).join(cached_compose_file(&tag));
    if let Ok(body) = std::fs::read_to_string(&path)
        && chapcore::validate_compose(&body).is_ok()
    {
        return ChapCore {
            source: fetched(&tag, &body),
            tag,
            cached: None,
        };
    }

    if offline {
        crate::output::warn(&format!(
            "--offline: compose.yml is rendered from the copy of chap-core's compose.ghcr.yml \
             built into this binary, not from the one {tag} publishes"
        ));
        return embedded(tag);
    }
    match chapcore::fetch_compose(&tag, timeout) {
        Ok(body) => ChapCore {
            source: fetched(&tag, &body),
            tag,
            cached: Some(body),
        },
        Err(err) => {
            crate::output::warn(&format!(
                "could not fetch chap-core's compose.ghcr.yml at {tag} ({err:#}); compose.yml is \
                 rendered from the copy built into this binary"
            ));
            embedded(tag)
        }
    }
}

/// The newest chap-core release, or why we are not asking.
pub(super) fn resolve_latest(
    offline: bool,
    timeout: std::time::Duration,
) -> std::result::Result<String, String> {
    if offline {
        return Err("--offline: the newest chap-core release cannot be looked up".to_string());
    }
    chapcore::latest_release(timeout)
        .map_err(|err| format!("could not resolve the newest chap-core release ({err:#})"))
}

/// A [`ComposeSource::Fetched`] for a body that is about to be (or already is)
/// cached under `.varde/`.
pub(super) fn fetched(tag: &str, body: &str) -> ComposeSource {
    ComposeSource::Fetched {
        url: chapcore::compose_url(tag),
        tag: tag.to_string(),
        sha256: chapcore::sha256_hex(body.as_bytes()),
    }
}
