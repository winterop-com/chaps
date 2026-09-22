//! Per-image data directory and user overrides.
//!
//! Most chapkit services write to `/work/data` as `chapkit:chapkit`, but some
//! images differ; getting this wrong crash-loops the container on the
//! read-only root filesystem. `--data-dir` and `--user` override the table.
//!
//! Owned by agent B.

/// The non-default data dir and user of one image.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ImageOverride {
    pub data_dir: &'static str,
    pub user: &'static str,
}

/// Data directory used by most chapkit images.
///
/// The scaffolded images build from `WORKDIR /work` and leave chapkit's
/// default `DATABASE_URL` (`sqlite+aiosqlite:///data/chapkit.db`) alone, so
/// the SQLite file lands in `/work/data`.
pub const DEFAULT_DATA_DIR: &str = "/work/data";
/// User most chapkit images run as.
///
/// uid/gid 1000, created in `chapkit-py.Dockerfile` and inherited by
/// `chapkit-r`, `chapkit-r-tidyverse` and `chapkit-r-inla`.
pub const DEFAULT_USER: &str = "chapkit:chapkit";

/// Data dir and user per marketplace id, read off each model's own Dockerfile.
///
/// Every marketplace entry is listed, including the ones that match the
/// defaults, so that the table doubles as the record of what was checked.
/// Repositories are the clones under `chap-models/<id>`.
const KNOWN: &[(&str, ImageOverride)] = &[
    // chapkit_ewars_model/Dockerfile:14 `WORKDIR /app`,
    // :33 `RUN mkdir -p /app/data && chown -R chapkit:chapkit /app/data`,
    // :37 `USER chapkit`.
    (
        "chapkit_ewars_model",
        ImageOverride {
            data_dir: "/app/data",
            user: "chapkit:chapkit",
        },
    ),
    // chapkit_simple_multistep_model/Dockerfile:12 `WORKDIR /app`,
    // :10 `useradd ... chap`, :30 `RUN mkdir -p /app/data && chown chap:chap
    // /app/data`, :37 `USER chap`. The only image with its own user.
    (
        "chapkit_simple_multistep_model",
        ImageOverride {
            data_dir: "/app/data",
            user: "chap:chap",
        },
    ),
    // chapkit_rwanda_malaria_bym_model/Dockerfile:11 `WORKDIR /work`; main.py:81
    // keeps the relative default `sqlite+aiosqlite:///data/chapkit.db`, so
    // /work/data. The image ends on `USER root`; the inherited chapkit user
    // (uid 1000) is what the hardened overlay runs it as.
    (
        "chapkit_rwanda_malaria_bym_model",
        ImageOverride {
            data_dir: "/work/data",
            user: "chapkit:chapkit",
        },
    ),
    // auto_arima_chapkit/Dockerfile:13 `WORKDIR /work`; main.py:77 keeps the
    // relative default database URL. (The model repo's own compose.yml mounts
    // /workspace/data, which does not match its WORKDIR; /work/data is what
    // the service actually writes to.)
    (
        "auto_arima_chapkit",
        ImageOverride {
            data_dir: "/work/data",
            user: "chapkit:chapkit",
        },
    ),
    // chapkit_minimalist_example_py/Dockerfile:7 `WORKDIR /work`; main.py:101
    // keeps the relative default database URL.
    (
        "chapkit_minimalist_example_py",
        ImageOverride {
            data_dir: "/work/data",
            user: "chapkit:chapkit",
        },
    ),
    // chapkit_minimalist_example_r/Dockerfile:15 `WORKDIR /work`; same
    // default database URL as the Python template.
    (
        "chapkit_minimalist_example_r",
        ImageOverride {
            data_dir: "/work/data",
            user: "chapkit:chapkit",
        },
    ),
];

/// Look up the override for a marketplace id, if it has one.
///
/// Owned by agent B.
pub fn known_override(id: &str) -> Option<ImageOverride> {
    KNOWN
        .iter()
        .find(|(known, _)| *known == id)
        .map(|(_, o)| *o)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_vendored_model_is_in_the_table() {
        let registry = crate::registry::load_embedded().unwrap();
        for m in &registry.models {
            assert!(
                known_override(&m.id).is_some(),
                "{} has no entry; check its Dockerfile before enabling it",
                m.id
            );
        }
        assert_eq!(KNOWN.len(), registry.models.len());
    }

    #[test]
    fn the_table_matches_the_model_dockerfiles() {
        let expected = [
            ("chapkit_ewars_model", "/app/data", "chapkit:chapkit"),
            ("chapkit_simple_multistep_model", "/app/data", "chap:chap"),
            (
                "chapkit_rwanda_malaria_bym_model",
                "/work/data",
                "chapkit:chapkit",
            ),
            ("auto_arima_chapkit", "/work/data", "chapkit:chapkit"),
            (
                "chapkit_minimalist_example_py",
                "/work/data",
                "chapkit:chapkit",
            ),
            (
                "chapkit_minimalist_example_r",
                "/work/data",
                "chapkit:chapkit",
            ),
        ];
        for (id, data_dir, user) in expected {
            let found = known_override(id).unwrap_or_else(|| panic!("no entry for {id}"));
            assert_eq!(found.data_dir, data_dir, "{id} data dir");
            assert_eq!(found.user, user, "{id} user");
        }
    }

    #[test]
    fn an_unknown_id_falls_back_to_the_defaults() {
        assert_eq!(known_override("something_else"), None);
        assert_eq!(DEFAULT_DATA_DIR, "/work/data");
        assert_eq!(DEFAULT_USER, "chapkit:chapkit");
    }
}
