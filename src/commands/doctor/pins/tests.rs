use super::*;

/// A model this deployment added itself, following `main` at `tag`.
fn manual_entry(tag: &str, follow: Option<&str>) -> crate::project::ManualModel {
    crate::project::ManualModel {
        service_id: "chapkit-ghr-model".into(),
        display_name: "chapkit_ghr_model".into(),
        repository: Some("https://github.com/chap-models/chapkit_ghr_model".into()),
        image: "ghcr.io/chap-models/chapkit_ghr_model".into(),
        tag: tag.to_string(),
        commit: None,
        follow: follow.map(str::to_string),
        data_dir: Some("/work/data".into()),
        user: Some("10001:10001".into()),
        runtime_amd64: true,
        added: "2026-09-24".into(),
    }
}

/// The `manual <id>` line: a warning only when the branch has moved on,
/// and never a failure.
#[test]
fn a_manual_model_is_checked_against_the_branch_it_follows() {
    let entry = manual_entry("sha-1eb8cf1", Some("main"));

    let ok = manual_check("chapkit_ghr_model", &entry, Some("sha-1eb8cf1"), false);
    assert_eq!(ok.status, Status::Ok);
    assert_eq!(ok.id, "manual-chapkit_ghr_model");
    assert_eq!(ok.name, "manual chapkit_ghr_model");
    assert!(ok.detail.contains("the newest published build on main"));
    assert_eq!(ok.fix, None);

    let behind = manual_check("chapkit_ghr_model", &entry, Some("sha-b1d6c31"), false);
    assert_eq!(behind.status, Status::Warn);
    assert!(behind.detail.contains("running sha-1eb8cf1"), "{behind:?}");
    assert!(
        behind.detail.contains("published sha-b1d6c31"),
        "{behind:?}"
    );
    assert_eq!(
        behind.fix.as_deref(),
        Some("run `chaps update` to move the pin")
    );

    // A repository that would not answer is a check that was not made.
    let unknown = manual_check("chapkit_ghr_model", &entry, None, false);
    assert_eq!(unknown.status, Status::Skip);
    assert!(unknown.detail.contains("could not ask"), "{unknown:?}");
    assert!(unknown.fix.is_some());

    // `--offline` asks nothing, and an entry with no branch has nothing
    // to be behind.
    let offline = manual_check("chapkit_ghr_model", &entry, None, true);
    assert_eq!(offline.status, Status::Skip);
    assert!(offline.detail.contains("--offline"), "{offline:?}");

    let pinned = manual_check(
        "chapkit_ghr_model",
        &manual_entry("sha-1eb8cf1", None),
        Some("sha-b1d6c31"),
        false,
    );
    assert_eq!(pinned.status, Status::Skip);
    assert!(
        pinned.detail.contains("pinned to sha-1eb8cf1"),
        "{pinned:?}"
    );
}

/// Where the marketplace is asked to move a pin.
const MARKETPLACE: &str = "https://github.com/dhis2-chap/model-marketplace";

/// The newest commit of the example branch, and the one before it.
const PIN_NEW: &str = "a6a05ef0000000000000000000000000000000aa";

const PIN_OLD: &str = "70c07a90000000000000000000000000000000bb";

/// A commit that is on no branch this listing carries: a tag of its own.
const PIN_OFF: &str = "5adf3a80000000000000000000000000000000cc";

/// A branch listing, newest first, with the newest commit that has a
/// published image named separately.
fn branch(log: &[(&str, &str)], newest: Option<&str>) -> crate::manual::BranchBuilds {
    crate::manual::BranchBuilds {
        log: log
            .iter()
            .map(|(sha, day)| crate::manual::github::Commit {
                sha: (*sha).to_string(),
                day: (*day).to_string(),
            })
            .collect(),
        newest: newest.map(|sha| (sha.to_string(), crate::manual::github::sha_tag(sha))),
    }
}

/// The main branch of the example model: two commits, both built.
fn main_log() -> [(&'static str, &'static str); 2] {
    [(PIN_NEW, "2026-09-23"), (PIN_OLD, "2026-09-08")]
}

/// The `registry pin <id>` line: the marketplace's own pin against the
/// newest build the model's repository has published.
#[test]
fn a_marketplace_pin_is_checked_against_the_branch_the_model_builds_from() {
    let unasked = |sha: &str| -> std::result::Result<String, String> {
        Err(format!("could not ask about {sha}"))
    };
    let verdict = |pinned: &str, builds: &crate::manual::BranchBuilds, day: PinDayFn| {
        let facts = pin_facts_of("main", builds, pinned, day);
        registry_pin_verdict(&crate::manual::github::sha_tag(pinned), MARKETPLACE, &facts)
    };

    // Same: the registry pins the newest build there is.
    let built = branch(&main_log(), Some(PIN_NEW));
    let same = verdict(PIN_NEW, &built, &unasked);
    assert_eq!(same.0, Status::Ok);
    assert_eq!(same.1, "sha-a6a05ef is the newest build on main");
    assert_eq!(same.2, None);
    // The line it is printed as.
    let check = Check::from_verdict(
        pin_id("chapkit_ewars_model"),
        pin_name("chapkit_ewars_model"),
        same,
    );
    assert_eq!(check.id, "registry-pin-chapkit_ewars_model");
    assert_eq!(check.name, "registry pin chapkit_ewars_model");

    // Behind: the registry still pins the commit before it. This is the
    // case the check exists for.
    let behind = verdict(PIN_OLD, &built, &unasked);
    assert_eq!(behind.0, Status::Warn);
    assert_eq!(
        behind.1,
        "the registry pins sha-70c07a9 (2026-09-08) but main's newest build is \
             sha-a6a05ef (2026-09-23)"
    );
    assert_eq!(
        behind.2.as_deref(),
        Some(
            "ask the marketplace maintainers for a new pin: https://github.com/dhis2-chap/model-marketplace"
        )
    );

    // Ahead: the pinned commit is on the branch and newer than anything
    // the publish workflow has built, so there is nothing to move to.
    let ahead = verdict(PIN_NEW, &branch(&main_log(), Some(PIN_OLD)), &unasked);
    assert_eq!(ahead.0, Status::Ok);
    assert_eq!(
        ahead.1,
        "sha-a6a05ef (2026-09-23) is pinned ahead of main's newest build \
             sha-70c07a9 (2026-09-08)"
    );
    assert_eq!(ahead.2, None);

    // A pin the branch does not carry at all is placed by its day, which
    // is one more request. Newer than the newest build: ahead.
    let dated = |day: &'static str| move |_: &str| Ok(day.to_string());
    let off_ahead = dated("2026-09-24");
    let off_ahead = verdict(PIN_OFF, &built, &off_ahead);
    assert_eq!(off_ahead.0, Status::Ok);
    assert!(
        off_ahead.1.contains("sha-5adf3a8 (2026-09-24)"),
        "{off_ahead:?}"
    );
    assert!(off_ahead.1.contains("pinned ahead"), "{off_ahead:?}");

    // Older than it: behind, whatever branch it came from.
    let off_behind = dated("2026-08-01");
    let off_behind = verdict(PIN_OFF, &built, &off_behind);
    assert_eq!(off_behind.0, Status::Warn);
    assert!(
        off_behind
            .1
            .contains("the registry pins sha-5adf3a8 (2026-08-01)"),
        "{off_behind:?}"
    );
}

/// Nothing the lookup cannot answer is a failure: every one of them is a
/// line that was not checked, with the reason on it.
#[test]
fn a_marketplace_pin_nobody_could_place_is_skipped_with_the_reason() {
    let unasked = |sha: &str| -> std::result::Result<String, String> {
        Err(format!("could not ask about {sha}"))
    };
    let skipped = |facts: std::result::Result<PinFacts, String>| {
        registry_pin_verdict("sha-5adf3a8", MARKETPLACE, &facts)
    };

    // The one extra request a pin off the branch needs, refused.
    let off = skipped(pin_facts_of(
        "main",
        &branch(&main_log(), Some(PIN_NEW)),
        PIN_OFF,
        &unasked,
    ));
    assert_eq!(off.0, Status::Skip);
    assert!(off.1.contains("could not ask about"), "{off:?}");
    assert_eq!(off.2, None, "a skip has nothing to do about it");

    // A branch whose newest commits have no published build yet.
    let unbuilt = skipped(pin_facts_of(
        "main",
        &branch(&main_log(), None),
        PIN_OFF,
        &unasked,
    ));
    assert_eq!(unbuilt.0, Status::Skip);
    assert!(
        unbuilt.1.contains("has a published `sha-` build"),
        "{unbuilt:?}"
    );

    // A catalogue entry with no commit to compare.
    let bare = skipped(pin_facts_of(
        "main",
        &branch(&main_log(), Some(PIN_NEW)),
        "  ",
        &unasked,
    ));
    assert_eq!(bare.0, Status::Skip);
    assert!(bare.1.contains("names no commit"), "{bare:?}");

    // A listing with no dates cannot place a commit that is not on it.
    let undated = skipped(pin_facts_of(
        "main",
        &branch(&[(PIN_NEW, ""), (PIN_OLD, "")], Some(PIN_NEW)),
        PIN_OFF,
        &|_| Ok("2026-09-24".to_string()),
    ));
    assert_eq!(undated.0, Status::Skip);
    assert!(undated.1.contains("no commit dates"), "{undated:?}");

    // And the whole lookup ruled out by the flag the user gave.
    assert!(PIN_OFFLINE.contains("--offline"));
}

#[test]
fn a_manifest_list_is_searched_for_linux_amd64() {
    let list = r#"{"manifests":[
            {"platform":{"architecture":"amd64","os":"linux"}},
            {"platform":{"architecture":"arm64","os":"linux"}}
        ]}"#;
    assert_eq!(manifest_has_amd64(list), Some(true));

    let arm_only = r#"{"manifests":[{"platform":{"architecture":"arm64","os":"linux"}}]}"#;
    assert_eq!(manifest_has_amd64(arm_only), Some(false));

    // A single image manifest carries no list, so the platform is simply
    // not something this can answer.
    let single = r#"{"schemaVersion":2,"mediaType":"application/vnd.oci.image.manifest.v1+json"}"#;
    assert_eq!(manifest_has_amd64(single), None);
    assert_eq!(manifest_has_amd64("not json"), None);
}
