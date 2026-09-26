//! Per-image data directory and user overrides.
//!
//! Most chapkit services write to `/work/data`, but the account they run as
//! differs per image, and getting it wrong either crash-loops the container on
//! the read-only root filesystem or takes away a permission the image's own
//! binaries need. The image config is the source of truth
//! ([`crate::compose::resolve`]); this table is the last resort, for a run
//! that can reach neither the registry nor a local copy of the image.
//! `--data-dir` and `--user` override both.

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

/// uid:gid the volume init container falls back to for a user it cannot
/// resolve, matching [`DEFAULT_USER`].
pub const FALLBACK_UID_GID: &str = "1000:1000";

/// Numeric ids of the account names the marketplace images create.
///
/// The overlay's init container chowns the data volume from busybox, which
/// knows none of these names, so `chown chapkit:chapkit` there fails with
/// "unknown user"; only numbers travel between images. Verified with
/// `id <name>` inside each published image.
///
/// Only the names the chapkit base images create are listed. `app`, which
/// GHRmodel's image declares, is deliberately left out: it is nobody's
/// convention - an image that says `USER app` has to be asked what uid that
/// is rather than assumed to match GHRmodel's 10001.
const KNOWN_IDS: &[(&str, u32)] = &[
    // chapkit-py.Dockerfile creates chapkit as uid/gid 1000; every image built
    // on chapkit-py (and the chapkit-r* family) inherits it. Checked against
    // the published sha-24d58c0 EWARS image: `uid=1000(chapkit)
    // gid=1000(chapkit)`.
    ("chapkit", 1000),
    // chapkit_simple_multistep_model/Dockerfile:10 adds `chap` with a plain
    // `useradd` on top of chapkit-py, where 1000 is already taken, so the
    // account lands on 1001 - not 1000. Checked against the published
    // sha-6c1d9ee image: `uid=1001(chap) gid=1001(chap)`.
    ("chap", 1001),
    ("root", 0),
];

/// Translate a compose `user:` value into the numeric `uid:gid` form.
///
/// Returns `None` for a name with no known id, so the caller can warn rather
/// than render a `chown` that would fail inside busybox. An already-numeric
/// value passes through unchanged, and a bare uid stays a bare uid (compose
/// and `chown` treat a missing group the same way: leave it alone).
pub fn numeric_user(user: &str) -> Option<String> {
    let mut parts = user.split(':');
    let uid = numeric_id(parts.next()?)?;
    let out = match parts.next() {
        Some(group) => format!("{uid}:{}", numeric_id(group)?),
        None => uid.to_string(),
    };
    // `a:b:c` is not a user spec.
    if parts.next().is_some() {
        return None;
    }
    Some(out)
}

/// One side of a `user:group` pair, as a number or a known account name.
fn numeric_id(part: &str) -> Option<u32> {
    if let Ok(n) = part.parse::<u32>() {
        return Some(n);
    }
    KNOWN_IDS
        .iter()
        .find(|(name, _)| *name == part)
        .map(|(_, id)| *id)
}

/// The full `uid:gid` an image's `User` means, for both the compose `user:`
/// line and the init container's `chown`.
///
/// [`numeric_user`] keeps a bare `chapkit` a bare uid, which is what compose
/// and `chown` have always read as "leave the group alone". A `User` read off
/// an image config is bare far more often than a hand-written `--user` is
/// (`USER chapkit` is the usual line), and leaving the group out there would
/// render a `user: 1000` next to a `chown 1000` that the group of the volume
/// no longer matches. Each account name the images create has a group of the
/// same name and the same id (verified with `id` inside each published image),
/// so a name resolves to the pair. A bare *number* still cannot: nothing here
/// knows which group uid 1500 is in, so it stays as it was given.
pub fn numeric_pair(user: &str) -> Option<String> {
    let user = user.trim();
    if user.contains(':') {
        return numeric_user(user);
    }
    if user.parse::<u32>().is_ok() {
        return Some(user.to_string());
    }
    let id = numeric_id(user)?;
    Some(format!("{id}:{id}"))
}

/// Whether this `user:` value is root, which is the one value that needs no
/// `user:` line: the image's own account already is root, and overriding it
/// could only take a permission away.
///
/// An empty value counts, because an image config with no `User` is an image
/// that runs as root.
pub fn is_root(user: &str) -> bool {
    let user = user.trim();
    if user.is_empty() {
        return true;
    }
    matches!(numeric_pair(user).as_deref(), Some("0") | Some("0:0"))
}

/// The pair a `chown` hands the data volume to.
///
/// [`numeric_pair`] leaves a bare number bare, because nothing here knows
/// which group uid 1500 is in and `chown 1500` is compose's and chown's own
/// "leave the group alone". Root is the one case where the group is not a
/// guess, so it is always written as the pair: a volume carried over from a
/// deployment that ran this model as `1000:1000` has group 1000 too, and a
/// bare `chown 0` would leave it there.
///
/// An account name nothing could turn into numbers falls back to the chapkit
/// ids, which is what `chaps sync` warns about.
pub fn chown_pair(user: &str) -> String {
    if is_root(user) {
        return "0:0".to_string();
    }
    numeric_pair(user).unwrap_or_else(|| FALLBACK_UID_GID.to_string())
}

/// Data dir and user per marketplace id, as each image's own config declares
/// them.
///
/// Every marketplace entry is listed, including the ones that match the
/// defaults, so that the table doubles as the record of what was checked. The
/// values are `config.User` and `config.WorkingDir`+`/data` of the image the
/// `stable` channel pins, read anonymously off ghcr - the same two fields
/// [`crate::compose::resolve`] reads at enable time, so a run that falls back
/// to this table lands where a run that could reach the registry would have.
/// Three of the seven images end their Dockerfile on `USER root`; EWARS, the
/// simple multistep model, the Rwanda BYM model and GHRmodel drop to an
/// account of their own.
const KNOWN: &[(&str, ImageOverride)] = &[
    // User=chapkit, WorkingDir=/app at sha-24d58c0.
    // chapkit_ewars_model/Dockerfile:14 `WORKDIR /app`,
    // :33 `RUN mkdir -p /app/data && chown -R chapkit:chapkit /app/data`,
    // :37 `USER chapkit`.
    (
        "chapkit_ewars_model",
        ImageOverride {
            data_dir: "/app/data",
            user: "chapkit",
        },
    ),
    // User=chap, WorkingDir=/app at sha-6c1d9ee.
    // chapkit_simple_multistep_model/Dockerfile:12 `WORKDIR /app`,
    // :10 `useradd ... chap`, :30 `RUN mkdir -p /app/data && chown chap:chap
    // /app/data`, :37 `USER chap`. The one image that creates an account of
    // its own on top of the chapkit base.
    (
        "chapkit_simple_multistep_model",
        ImageOverride {
            data_dir: "/app/data",
            user: "chap",
        },
    ),
    // User=chapkit, WorkingDir=/work at sha-a7b2892.
    // chapkit_rwanda_malaria_bym_model/Dockerfile:16 `WORKDIR /work`; main.py:81
    // keeps the relative default `sqlite+aiosqlite:///data/chapkit.db`, so
    // /work/data. :34 `RUN mkdir -p /work/data && chown -R chapkit:chapkit
    // /work/data`, :42 `USER chapkit`. Up to 0.1.1 (sha-28d9fc3) the image ended
    // on `USER root` with its INLA binaries mode 744 root-owned; 0.1.2 rebuilds
    // on a chapkit-r-inla base that leaves them mode 755, so the chapkit user
    // can execute them.
    (
        "chapkit_rwanda_malaria_bym_model",
        ImageOverride {
            data_dir: "/work/data",
            user: "chapkit",
        },
    ),
    // User=root, WorkingDir=/work at sha-5adf3a8.
    // auto_arima_chapkit/Dockerfile:13 `WORKDIR /work`; main.py:77 keeps the
    // relative default database URL. (The model repo's own compose.yml mounts
    // /workspace/data, which does not match its WORKDIR; /work/data is what
    // the service actually writes to.)
    (
        "auto_arima_chapkit",
        ImageOverride {
            data_dir: "/work/data",
            user: "root",
        },
    ),
    // User=app, WorkingDir=/work at sha-a9532c7. The chapkit-r-inla base
    // leaves chapkit's relative default database URL alone, so /work/data,
    // which the model file's own notes repeat. `app` is the image's own
    // account, uid/gid 10001 (`id app` inside sha-a9532c7), and is the one
    // user in this table that [`numeric_pair`] cannot turn into numbers: see
    // KNOWN_IDS for why it stays out of there.
    (
        "chapkit_ghr_model",
        ImageOverride {
            data_dir: "/work/data",
            user: "app",
        },
    ),
    // User=root, WorkingDir=/work at sha-73557ef.
    // chapkit_minimalist_example_py/Dockerfile:7 `WORKDIR /work`; main.py:101
    // keeps the relative default database URL.
    (
        "chapkit_minimalist_example_py",
        ImageOverride {
            data_dir: "/work/data",
            user: "root",
        },
    ),
    // User=root, WorkingDir=/work at sha-9fbdd09.
    // chapkit_minimalist_example_r/Dockerfile:15 `WORKDIR /work`; same
    // default database URL as the Python template.
    (
        "chapkit_minimalist_example_r",
        ImageOverride {
            data_dir: "/work/data",
            user: "root",
        },
    ),
];

/// Look up the override for a marketplace id, if it has one.
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

    /// The table is the last resort, so it has to land where reading the
    /// image config would have. Each pair below was read off the published
    /// image the `stable` channel pins, anonymously from ghcr: a pull token,
    /// the OCI index, the amd64 manifest inside it and the config blob it
    /// names - `config.User` and `config.WorkingDir` verbatim. `docker image
    /// inspect` of the same tags agrees, except for the simple multistep
    /// model, whose local copy on the development machine carries an empty
    /// config; ghcr is what is quoted here.
    #[test]
    fn the_table_matches_what_the_marketplace_images_declare() {
        // (id, WorkingDir, User) as ghcr serves them today.
        let declared = [
            ("chapkit_ewars_model", "/app", "chapkit"),
            ("chapkit_simple_multistep_model", "/app", "chap"),
            ("chapkit_rwanda_malaria_bym_model", "/work", "chapkit"),
            ("auto_arima_chapkit", "/work", "root"),
            ("chapkit_ghr_model", "/work", "app"),
            ("chapkit_minimalist_example_py", "/work", "root"),
            ("chapkit_minimalist_example_r", "/work", "root"),
        ];
        for (id, working_dir, user) in declared {
            let found = known_override(id).unwrap_or_else(|| panic!("no entry for {id}"));
            assert_eq!(
                found.data_dir,
                format!("{working_dir}/data"),
                "{id} writes under its WorkingDir"
            );
            assert_eq!(found.user, user, "{id} user");
        }
    }

    /// The three images that end on `USER root` get no `user:` line and no
    /// init container; the four that drop to an account of their own do.
    #[test]
    fn the_table_says_which_images_run_as_root() {
        let root: Vec<&str> = KNOWN
            .iter()
            .filter(|(_, o)| is_root(o.user))
            .map(|(id, _)| *id)
            .collect();
        assert_eq!(
            root,
            [
                "auto_arima_chapkit",
                "chapkit_minimalist_example_py",
                "chapkit_minimalist_example_r",
            ]
        );
    }

    #[test]
    fn an_unknown_id_falls_back_to_the_defaults() {
        assert_eq!(known_override("something_else"), None);
        assert_eq!(DEFAULT_DATA_DIR, "/work/data");
        assert_eq!(DEFAULT_USER, "chapkit:chapkit");
    }

    #[test]
    fn numeric_user_resolves_the_names_the_images_create() {
        // busybox has no chapkit account, so the init container needs numbers.
        assert_eq!(
            numeric_user("chapkit:chapkit").as_deref(),
            Some("1000:1000")
        );
        // `chap` is 1001, not 1000: useradd skipped the uid chapkit holds.
        assert_eq!(numeric_user("chap:chap").as_deref(), Some("1001:1001"));
        assert_eq!(numeric_user("root:root").as_deref(), Some("0:0"));
        assert_eq!(numeric_user("chapkit").as_deref(), Some("1000"));
        assert_eq!(numeric_user("chapkit:chap").as_deref(), Some("1000:1001"));
    }

    #[test]
    fn numeric_user_passes_numbers_through() {
        assert_eq!(numeric_user("1000:1000").as_deref(), Some("1000:1000"));
        assert_eq!(numeric_user("1001:2002").as_deref(), Some("1001:2002"));
        assert_eq!(numeric_user("0:0").as_deref(), Some("0:0"));
        assert_eq!(numeric_user("1000").as_deref(), Some("1000"));
        // A mixed pair resolves either side.
        assert_eq!(numeric_user("1000:chapkit").as_deref(), Some("1000:1000"));
    }

    #[test]
    fn numeric_user_gives_up_on_a_name_it_does_not_know() {
        for user in ["nobody", "chapkit:nobody", "nobody:1000", "", "a:b:c", "-1"] {
            assert_eq!(numeric_user(user), None, "{user:?}");
        }
        assert_eq!(FALLBACK_UID_GID, "1000:1000");
    }

    /// The init container chowns from busybox, so a table user that cannot be
    /// turned into numbers is a `chown` that fails. GHRmodel is the one
    /// deliberate exception: its `app` is nobody's convention, so it stays out
    /// of `KNOWN_IDS` and is asked of the image instead. Enabling it reaches
    /// the registry or the local daemon and records `10001:10001`; only a run
    /// that can reach neither lands on this row, and `chaps sync` warns there.
    #[test]
    fn every_user_in_the_table_is_resolvable() {
        const PROBED: &[&str] = &["chapkit_ghr_model"];
        for (id, o) in KNOWN {
            if PROBED.contains(id) {
                assert_eq!(
                    numeric_pair(o.user),
                    None,
                    "{id} would no longer be probed for its uid"
                );
                continue;
            }
            assert!(
                numeric_pair(o.user).is_some(),
                "{id} runs as {} which the init container cannot chown to",
                o.user
            );
        }
    }

    #[test]
    fn numeric_pair_gives_a_bare_account_name_its_group() {
        // What an image config says - `USER chapkit` - has to render the same
        // pair as a hand-written `chapkit:chapkit`.
        assert_eq!(numeric_pair("chapkit").as_deref(), Some("1000:1000"));
        assert_eq!(
            numeric_pair("chapkit:chapkit").as_deref(),
            Some("1000:1000")
        );
        assert_eq!(numeric_pair("chap").as_deref(), Some("1001:1001"));
        assert_eq!(numeric_pair("root").as_deref(), Some("0:0"));
        // A chown always gets a pair for root, whatever spelling it arrived
        // in, because root's group is not a guess.
        for spelling in ["root", "root:root", "0", "0:0", ""] {
            assert_eq!(chown_pair(spelling), "0:0", "{spelling:?}");
        }
        assert_eq!(chown_pair("chapkit"), "1000:1000");
        assert_eq!(chown_pair("1500"), "1500", "nothing knows uid 1500's group");
        assert_eq!(chown_pair("1500:1600"), "1500:1600");
        assert_eq!(chown_pair("nobody-in-particular"), FALLBACK_UID_GID);
        // A bare number names no group this CLI can look up, so it is left
        // exactly as it was given.
        assert_eq!(numeric_pair("1500").as_deref(), Some("1500"));
        assert_eq!(numeric_pair("1500:1600").as_deref(), Some("1500:1600"));
        assert_eq!(numeric_pair("nobody"), None);
        assert_eq!(numeric_pair("chapkit:nobody"), None);
    }

    #[test]
    fn root_is_recognised_however_it_is_written() {
        for user in ["root", "root:root", "0", "0:0", "0:root", " root ", ""] {
            assert!(is_root(user), "{user:?}");
        }
        for user in ["chapkit", "chapkit:chapkit", "1000:1000", "nobody"] {
            assert!(!is_root(user), "{user:?}");
        }
    }
}
