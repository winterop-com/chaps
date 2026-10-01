use super::*;

#[test]
fn the_token_is_the_first_variable_with_something_in_it() {
    assert_eq!(
        token_of(&[Some("ghp_one"), Some("ghp_two")]).as_deref(),
        Some("ghp_one")
    );
    // GITHUB_TOKEN unset, GH_TOKEN set: the `gh` CLI's variable.
    assert_eq!(
        token_of(&[None, Some("ghp_two")]).as_deref(),
        Some("ghp_two")
    );
    // Set to nothing is not set: a CI secret that did not resolve must
    // not send an empty Bearer.
    assert_eq!(
        token_of(&[Some(""), Some("ghp_two")]).as_deref(),
        Some("ghp_two")
    );
    assert_eq!(token_of(&[Some("   "), None]), None);
    assert_eq!(token_of(&[None, None]), None);
    assert_eq!(token_of(&[]), None);
    // Whitespace around a value pasted out of a terminal is not part of it.
    assert_eq!(token_of(&[Some(" ghp_one\n")]).as_deref(), Some("ghp_one"));
}

#[test]
fn every_request_carries_the_media_type_and_the_api_version() {
    let anonymous = headers(None);
    assert_eq!(
        anonymous,
        vec![
            ("Accept", ACCEPT.to_string()),
            ("X-GitHub-Api-Version", "2022-11-28".to_string()),
        ]
    );
    assert!(
        !anonymous.iter().any(|(name, _)| *name == "Authorization"),
        "no token, no Authorization header"
    );

    let authenticated = headers(Some("ghp_secret"));
    assert_eq!(
        authenticated.last(),
        Some(&("Authorization", "Bearer ghp_secret".to_string()))
    );
    // The other two are still there, unchanged.
    assert_eq!(&authenticated[..2], &anonymous[..]);
}

#[test]
fn the_base_url_falls_back_to_public_github() {
    assert_eq!(
        base_of(Some("http://127.0.0.1:18099/".into()), DEFAULT_API),
        "http://127.0.0.1:18099"
    );
    assert_eq!(base_of(None, DEFAULT_API), DEFAULT_API);
    assert_eq!(base_of(Some(String::new()), DEFAULT_API), DEFAULT_API);
    assert_eq!(base_of(Some("  ".into()), DEFAULT_API), DEFAULT_API);
    assert!(rate_limit_url().ends_with("/rate_limit"));
}

/// The whole point of the module: a 403 that is the anonymous limit says
/// so, and says what to do about it.
#[test]
fn a_used_up_anonymous_limit_says_what_it_is_and_what_to_do() {
    // 2026-09-25T13:04:00Z, whatever this machine calls that hour.
    let reset = 1_758_805_440;
    let text = refusal(403, Some(0), Some(reset), false).expect("a rate limit reads as one");
    assert!(
        text.starts_with(
            "GitHub's rate limit is used up (unauthenticated, 60 requests an hour per \
                 address); set GITHUB_TOKEN for 5000, or wait until "
        ),
        "{text}"
    );
    assert!(text.ends_with(&crate::output::local_clock(reset)), "{text}");

    // A secondary limit arrives as a 429 and is the same sentence.
    assert_eq!(
        refusal(429, Some(0), Some(reset), false),
        refusal(403, Some(0), Some(reset), false)
    );

    // No reset header: the sentence still stands, with nothing to wait for.
    let text = refusal(403, Some(0), None, false).expect("still the limit");
    assert!(text.ends_with("or wait until -"), "{text}");
}

#[test]
fn a_403_with_a_token_points_at_the_token() {
    let text = refusal(403, Some(0), Some(1_758_805_440), true).expect("a refusal");
    assert_eq!(text, REFUSED_WITH_TOKEN);
    assert!(text.contains("GITHUB_TOKEN's scopes"), "{text}");
    // Not the anonymous advice, which would be wrong here.
    assert!(!text.contains("set GITHUB_TOKEN for"), "{text}");
    // The quota headers make no difference once a token was sent.
    assert_eq!(
        refusal(403, None, None, true).as_deref(),
        Some(REFUSED_WITH_TOKEN)
    );
}

/// Everything else keeps the wording it always had, which is the bare
/// `HTTP <status> from <url>` the caller builds.
#[test]
fn every_other_status_says_only_its_number() {
    assert_eq!(refusal(404, Some(59), None, false), None);
    assert_eq!(refusal(500, None, None, true), None);
    // A 403 that is not the limit - requests left, or no header at all -
    // is not the rate limit and must not claim to be.
    assert_eq!(refusal(403, Some(42), None, false), None);
    assert_eq!(refusal(403, None, None, false), None);
    assert_eq!(refusal(429, Some(7), None, false), None);
}

/// The error a caller sees: the sentence where there is one, the typed
/// [`ChapError::Http`] otherwise, because `release_exists` matches on it.
#[test]
fn the_answer_turns_into_the_error_its_status_deserves() {
    let url = "https://api.github.com/repos/dhis2-chap/chap-core/releases/latest";
    let limited = Answer {
        status: 403,
        body: r#"{"message":"API rate limit exceeded"}"#.to_string(),
        rate: RateLimit {
            limit: Some(60),
            remaining: Some(0),
            reset: Some(1_758_805_440),
        },
        token: false,
    };
    let err = limited.error(url);
    assert!(err.to_string().contains("rate limit is used up"), "{err}");
    assert!(err.downcast_ref::<ChapError>().is_none(), "{err}");

    let missing = Answer {
        status: 404,
        body: String::new(),
        rate: RateLimit::default(),
        token: false,
    };
    match missing.error(url).downcast_ref::<ChapError>() {
        Some(ChapError::Http { status, .. }) => assert_eq!(*status, 404),
        other => panic!("a 404 has to stay a typed HTTP error: {other:?}"),
    }
    assert!(!missing.ok());
    assert!(
        Answer {
            status: 200,
            body: String::new(),
            rate: RateLimit::default(),
            token: true,
        }
        .ok()
    );
}

#[test]
fn the_quota_payload_yields_the_core_bucket() {
    const BODY: &str = r#"{
          "resources": {
            "core": {"limit": 5000, "remaining": 4990, "reset": 1758805440, "used": 10},
            "search": {"limit": 30, "remaining": 30, "reset": 1758805440}
          },
          "rate": {"limit": 5000, "remaining": 4990, "reset": 1758805440}
        }"#;
    assert_eq!(parse_quota(BODY), Some((5000, 4990, Some(1_758_805_440))));

    // Only the older `rate` field: still an answer.
    assert_eq!(
        parse_quota(r#"{"rate":{"limit":60,"remaining":43,"reset":1758805440}}"#),
        Some((60, 43, Some(1_758_805_440)))
    );
    // Nothing to read is no quota rather than a wrong one.
    assert_eq!(parse_quota(r#"{"message":"Not Found"}"#), None);
    assert_eq!(parse_quota("<html>"), None);
    assert_eq!(parse_quota(r#"{"rate":{"limit":60}}"#), None);
}

/// Port 9 is the discard service, so this covers the transport-error path
/// without a network.
#[test]
fn an_unreachable_host_is_an_error_not_a_hang() {
    let err = get_ok("http://127.0.0.1:9/rate_limit", Duration::from_secs(2))
        .expect_err("nothing is listening on port 9");
    assert!(err.to_string().contains("127.0.0.1:9"), "{err}");
}
