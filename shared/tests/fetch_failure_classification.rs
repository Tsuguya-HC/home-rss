use home_rss_shared::fetch_failure::{
    FetchFailureAction, FetchOutcomeKind, decide_fetch_failure_action,
};

#[test]
fn fetch_failed_with_prior_fetch_is_recorded() {
    // Catches the FetchFailed branch falling into Keep or Clear.
    match decide_fetch_failure_action(FetchOutcomeKind::FetchFailed, "HTTP 404", true) {
        FetchFailureAction::Record { reason } => {
            assert!(reason.contains("404"));
            assert!(reason.chars().count() <= 200);
        }
        other => panic!("FetchFailed must be recorded, got {other:?}"),
    }
}

#[test]
fn unparseable_with_prior_fetch_is_recorded() {
    // Catches the Unparseable branch falling into Keep or Clear.
    match decide_fetch_failure_action(
        FetchOutcomeKind::Unparseable,
        "feed could not be parsed",
        true,
    ) {
        FetchFailureAction::Record { reason } => {
            assert!(!reason.is_empty());
            assert!(reason.chars().count() <= 200);
        }
        other => panic!("Unparseable must be recorded, got {other:?}"),
    }
}

#[test]
fn stored_clears_the_record() {
    // Catches a success path leaving a stale failure mark behind.
    assert_eq!(
        decide_fetch_failure_action(FetchOutcomeKind::Stored, "", true),
        FetchFailureAction::Clear
    );
}

#[test]
fn not_modified_clears_the_record() {
    // Catches 304 leaving a stale failure mark behind.
    assert_eq!(
        decide_fetch_failure_action(FetchOutcomeKind::NotModified, "", true),
        FetchFailureAction::Clear
    );
}

#[test]
fn store_failed_keeps_the_record_untouched() {
    // Catches StoreFailed clearing or overwriting the failure record.
    assert_eq!(
        decide_fetch_failure_action(FetchOutcomeKind::StoreFailed, "db is down", true),
        FetchFailureAction::Keep
    );
    assert_eq!(
        decide_fetch_failure_action(FetchOutcomeKind::StoreFailed, "db is down", false),
        FetchFailureAction::Keep
    );
}

#[test]
fn failure_of_never_fetched_feed_leaves_no_mark() {
    // Catches OPML-imported rows getting marked before their first fetch.
    assert_eq!(
        decide_fetch_failure_action(FetchOutcomeKind::FetchFailed, "HTTP 500", false),
        FetchFailureAction::Keep
    );
    assert_eq!(
        decide_fetch_failure_action(
            FetchOutcomeKind::Unparseable,
            "feed could not be parsed",
            false
        ),
        FetchFailureAction::Keep
    );
}

#[test]
fn record_reason_is_cut_at_200_chars() {
    // Catches a missing truncation and byte-based truncation: the multibyte
    // input must still be exactly 200 chars, not 200 bytes.
    match decide_fetch_failure_action(FetchOutcomeKind::FetchFailed, &"e".repeat(250), true) {
        FetchFailureAction::Record { reason } => {
            assert_eq!(reason.chars().count(), 200);
        }
        other => panic!("FetchFailed must be recorded, got {other:?}"),
    }
    match decide_fetch_failure_action(FetchOutcomeKind::FetchFailed, &"あ".repeat(250), true) {
        FetchFailureAction::Record { reason } => {
            assert_eq!(reason.chars().count(), 200);
        }
        other => panic!("FetchFailed must be recorded, got {other:?}"),
    }
}
