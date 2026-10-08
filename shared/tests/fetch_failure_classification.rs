//! #245 の赤: 定期取得の 1 回分の結果を失敗記録の振る舞い
//! (記録する/消す/変えない) に振り分ける `classify_fetch_failure` の全分岐を
//! 固定する。

use home_rss_shared::fetch::{FetchAndStoreOutcome, FetchFailureAction, classify_fetch_failure};
use home_rss_shared::models::Feed;

fn stored_feed() -> Feed {
    Feed {
        id: "00000000-0000-0000-0000-000000000000".to_owned(),
        url: "https://example.com/feed".to_owned(),
        title: Some("Example".to_owned()),
        site_url: None,
        etag: None,
        last_modified: None,
        last_fetched_at: Some(1_700_000_000),
        created_at: None,
        last_fetch_error: None,
        fetch_failing_since: None,
    }
}

fn fetch_failed_outcome() -> FetchAndStoreOutcome {
    FetchAndStoreOutcome::FetchFailed(anyhow::anyhow!(
        "HTTP 404 fetching https://example.com/feed"
    ))
}

fn unparseable_outcome() -> FetchAndStoreOutcome {
    FetchAndStoreOutcome::Unparseable(anyhow::anyhow!(
        "feed at https://example.com/feed could not be parsed: not XML"
    ))
}

#[test]
fn fetch_failed_records_a_reason_when_previously_fetched() {
    match classify_fetch_failure(&fetch_failed_outcome(), true) {
        FetchFailureAction::Record(reason) => {
            assert!(
                reason.contains("404"),
                "the record must carry the failure kind, got {reason:?}"
            );
            assert!(
                reason.chars().count() <= 200,
                "the record must be cut at 200 chars, got {} chars",
                reason.chars().count()
            );
        }
        other => panic!("FetchFailed must record a reason, got {other:?}"),
    }
}

#[test]
fn unparseable_records_a_reason_when_previously_fetched() {
    match classify_fetch_failure(&unparseable_outcome(), true) {
        FetchFailureAction::Record(reason) => {
            assert!(!reason.is_empty(), "the record must carry a reason");
            assert!(
                reason.chars().count() <= 200,
                "the record must be cut at 200 chars, got {} chars",
                reason.chars().count()
            );
        }
        other => panic!("Unparseable must record a reason, got {other:?}"),
    }
}

#[test]
fn feeds_never_fetched_gain_no_record() {
    assert_eq!(
        classify_fetch_failure(&fetch_failed_outcome(), false),
        FetchFailureAction::Keep
    );
    assert_eq!(
        classify_fetch_failure(&unparseable_outcome(), false),
        FetchFailureAction::Keep
    );
    assert!(
        matches!(
            classify_fetch_failure(&fetch_failed_outcome(), true),
            FetchFailureAction::Record(_)
        ),
        "the same failure must record once the feed was fetched before"
    );
}

#[test]
fn stored_clears_the_record() {
    assert_eq!(
        classify_fetch_failure(&FetchAndStoreOutcome::Stored(Box::new(stored_feed())), true),
        FetchFailureAction::Clear
    );
}

#[test]
fn not_modified_clears_the_record() {
    assert_eq!(
        classify_fetch_failure(&FetchAndStoreOutcome::NotModified, true),
        FetchFailureAction::Clear
    );
}

#[test]
fn store_failed_leaves_the_record_alone_while_fetch_failed_records() {
    assert_eq!(
        classify_fetch_failure(
            &FetchAndStoreOutcome::StoreFailed(anyhow::anyhow!("failed to update feed x")),
            true
        ),
        FetchFailureAction::Keep
    );
    assert!(
        matches!(
            classify_fetch_failure(&fetch_failed_outcome(), true),
            FetchFailureAction::Record(_)
        ),
        "FetchFailed must record while StoreFailed keeps the record"
    );
}

#[test]
fn record_reason_is_cut_at_200_chars() {
    let long = "e".repeat(300);
    let outcome = FetchAndStoreOutcome::FetchFailed(anyhow::anyhow!(
        "HTTP 500 fetching https://example.com/feed: {long}"
    ));
    match classify_fetch_failure(&outcome, true) {
        FetchFailureAction::Record(reason) => assert!(
            reason.chars().count() <= 200,
            "the record must be cut at 200 chars, got {} chars",
            reason.chars().count()
        ),
        other => panic!("FetchFailed must record a reason, got {other:?}"),
    }
}
