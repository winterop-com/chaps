use super::*;

/// The repository payload, trimmed to the fields that exist in the real
/// one plus a few that are ignored.
const REPO: &str = r#"{
      "id": 1042,
      "name": "chapkit_ghr_model",
      "full_name": "chap-models/chapkit_ghr_model",
      "private": false,
      "default_branch": "main",
      "visibility": "public"
    }"#;

const COMMITS: &str = r#"[
      {"sha": "b1d6c31f4b2f0d8a0f4c6e2a5e9d3c7b8a1f0e2d", "commit": {"message": "docs"}},
      {"sha": "60b16a2c1e9f4d3a2b8c7d6e5f4a3b2c1d0e9f8a", "commit": {"message": "fix"}}
    ]"#;

#[test]
fn the_repository_payload_yields_its_default_branch() {
    assert_eq!(parse_default_branch(REPO).unwrap(), "main");
    assert_eq!(
        parse_default_branch(r#"{"default_branch":" master "}"#).unwrap(),
        "master"
    );
}

#[test]
fn a_repository_without_a_branch_is_an_error() {
    // The rate-limit body parses as JSON and names no branch.
    let err = parse_default_branch(r#"{"message":"API rate limit exceeded"}"#)
        .expect_err("no default_branch");
    assert!(err.to_string().contains("default_branch"), "{err}");
    assert!(parse_default_branch("<html>nope</html>").is_err());
}

#[test]
fn the_commit_listing_yields_its_shas_newest_first() {
    let log = parse_commit_log(COMMITS).unwrap();
    assert_eq!(log.len(), 2);
    assert!(log[0].sha.starts_with("b1d6c31"));
    assert_eq!(sha_tag(&log[0].sha), "sha-b1d6c31");
    assert_eq!(sha_tag(&log[1].sha), "sha-60b16a2");
}

#[test]
fn an_empty_or_unreadable_commit_listing_is_an_error() {
    assert!(parse_commit_log("[]").is_err());
    assert!(parse_commit_log(r#"{"message":"Not Found"}"#).is_err());
}

/// The listing the pin checks read: every commit with the day it was
/// made, and an entry with neither date left with none.
#[test]
fn the_commit_listing_carries_the_day_of_every_commit() {
    const DATED: &str = r#"[
          {"sha": "a6a05ef1111111111111111111111111111111ff",
           "commit": {"committer": {"date": "2026-09-23T08:15:00Z"},
                      "author": {"date": "2026-09-20T08:15:00Z"}}},
          {"sha": "70c07a92222222222222222222222222222222ff",
           "commit": {"author": {"date": "2026-09-08T11:00:00Z"}}},
          {"sha": "00000003333333333333333333333333333333ff", "commit": {"message": "x"}}
        ]"#;
    let log = parse_commit_log(DATED).unwrap();
    assert_eq!(log.len(), 3);
    // The committer date wins over the author's, which is the day the
    // build that carries the commit was made.
    assert_eq!(log[0].day, "2026-09-23");
    // A rewritten commit with only an author date still has a day.
    assert_eq!(log[1].day, "2026-09-08");
    assert_eq!(log[2].day, "", "no date is no day, and not a failure");
    assert_eq!(sha_tag(&log[1].sha), "sha-70c07a9");

    // A listing with no dates at all is still a listing.
    assert!(
        parse_commit_log(COMMITS)
            .unwrap()
            .iter()
            .all(|c| c.day.is_empty())
    );
    assert!(parse_commit_log("[]").is_err());
}

#[test]
fn one_commit_payload_yields_its_day() {
    assert_eq!(
        parse_commit_day(
            r#"{"sha":"70c07a9","commit":{"committer":{"date":"2026-09-08T11:00:00Z"}}}"#
        )
        .as_deref(),
        Some("2026-09-08")
    );
    // A commit GitHub will not talk about has no day to print.
    assert_eq!(parse_commit_day(r#"{"message":"Not Found"}"#), None);
    assert_eq!(parse_commit_day("<html>"), None);
}

#[test]
fn a_short_sha_is_the_first_seven_characters() {
    assert_eq!(sha_tag("b1d6c31f4b2f"), "sha-b1d6c31");
    assert_eq!(sha_tag("abc"), "sha-abc");
}
