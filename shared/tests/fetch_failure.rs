//! #245: 定期取得の 1 回分を「記録する / 消す / 変えない」に振り分ける純粋関数の全分岐。
//! 公開 API (`home_rss_shared::fetch_failure`) だけを通す。

use home_rss_shared::fetch_failure::{
    FailureAction, FetchFailureKind, ScheduledFetchOutcome, classify_fetch_error,
    decide_failure_action, truncate_reason,
};

fn failed(kind: FetchFailureKind) -> ScheduledFetchOutcome {
    ScheduledFetchOutcome::FetchFailed(kind)
}

#[test]
fn guard_rejection_is_recorded_with_reason() {
    // URL ガードで弾かれた定期取得は失敗として記録される (#245 が e2e でも確かめる分岐)。
    let action = decide_failure_action(
        &failed(FetchFailureKind::GuardRejected("private host".to_owned())),
        true,
    );
    match action {
        FailureAction::Record { reason } => assert!(
            reason.contains("ガード"),
            "guard rejection must be recorded as a guard refusal, got {reason:?}"
        ),
        other => panic!("guard rejection must be recorded, got {other:?}"),
    }
}

#[test]
fn unexpected_status_is_recorded_with_status_code() {
    // 200 / 304 以外の応答 (3xx を含む) は失敗として記録される。
    for status in [301, 404, 500] {
        let action =
            decide_failure_action(&failed(FetchFailureKind::UnexpectedStatus(status)), true);
        match action {
            FailureAction::Record { reason } => assert!(
                reason.contains(&status.to_string()),
                "HTTP {status} must appear in the recorded reason, got {reason:?}"
            ),
            other => panic!("HTTP {status} must be recorded, got {other:?}"),
        }
    }
}

#[test]
fn send_error_is_recorded_with_reason() {
    let action = decide_failure_action(
        &failed(FetchFailureKind::Send("connection reset".to_owned())),
        true,
    );
    match action {
        FailureAction::Record { reason } => assert!(
            reason.contains("送信エラー"),
            "send errors must be recorded as send errors, got {reason:?}"
        ),
        other => panic!("send errors must be recorded, got {other:?}"),
    }
}

#[test]
fn other_fetch_failures_are_recorded() {
    // リクエストの組み立て失敗と本文の読み取り失敗も `FetchFailed` として記録される。
    // 理由には短い詳細が入る。
    for (outcome, detail) in [
        (
            failed(FetchFailureKind::RequestBuild("bad builder".to_owned())),
            "bad builder",
        ),
        (
            failed(FetchFailureKind::BodyRead("truncated".to_owned())),
            "truncated",
        ),
    ] {
        match decide_failure_action(&outcome, true) {
            FailureAction::Record { reason } => assert!(
                reason.contains(detail),
                "recorded reason must carry the detail {detail:?}, got {reason:?}"
            ),
            other => panic!("{outcome:?} must be recorded, got {other:?}"),
        }
    }
}

#[test]
fn unparseable_body_is_recorded() {
    let action = decide_failure_action(&ScheduledFetchOutcome::Unparseable, true);
    match action {
        FailureAction::Record { reason } => assert!(
            reason.contains("フィードとして解釈できない"),
            "unparseable body must be recorded as unparseable, got {reason:?}"
        ),
        other => panic!("unparseable body must be recorded, got {other:?}"),
    }
}

#[test]
fn stored_and_not_modified_clear_the_record() {
    // 200 か 304 なら記録を消す。
    assert_eq!(
        decide_failure_action(&ScheduledFetchOutcome::Stored, true),
        FailureAction::Clear
    );
    assert_eq!(
        decide_failure_action(&ScheduledFetchOutcome::NotModified, true),
        FailureAction::Clear
    );
}

#[test]
fn store_failure_keeps_the_record() {
    // `StoreFailed` (DB への保存の失敗) は記録を変えない。
    assert_eq!(
        decide_failure_action(&ScheduledFetchOutcome::StoreFailed, true),
        FailureAction::Keep
    );
    assert_eq!(
        decide_failure_action(&ScheduledFetchOutcome::StoreFailed, false),
        FailureAction::Keep
    );
}

#[test]
fn never_fetched_feeds_are_never_marked() {
    // 一度も取得していないフィード (OPML 直後など) には印を付けない。
    for outcome in [
        failed(FetchFailureKind::Send("down".to_owned())),
        failed(FetchFailureKind::GuardRejected("blocked".to_owned())),
        failed(FetchFailureKind::UnexpectedStatus(404)),
        ScheduledFetchOutcome::Unparseable,
    ] {
        assert_eq!(
            decide_failure_action(&outcome, false),
            FailureAction::Keep,
            "{outcome:?} on a never-fetched feed must not be recorded"
        );
    }
}

#[test]
fn recorded_reasons_are_capped_at_200_chars() {
    let long = format!("送信エラー: {}", "あ".repeat(300));
    let capped = truncate_reason(&long);
    assert_eq!(capped.chars().count(), 200);
    assert!(long.starts_with(&capped));
}

#[test]
fn short_reasons_pass_through_truncation() {
    assert_eq!(truncate_reason("HTTP 404"), "HTTP 404");
}

#[test]
fn fetch_error_messages_are_classified_back_into_kinds() {
    // `fetch_and_store` が返す `FetchFailed` の文面を種類に戻す。組み立て側
    // (`fetch.rs`) の文面を変えたらここも変える必要がある。
    match classify_fetch_error("SSRF guard rejected feed abc: https://x (feed URL must use https)")
    {
        FetchFailureKind::GuardRejected(_) => {}
        other => panic!("guard message must classify as guard, got {other:?}"),
    }
    match classify_fetch_error("failed to build request for https://x: bad") {
        FetchFailureKind::RequestBuild(_) => {}
        other => panic!("build message must classify as build, got {other:?}"),
    }
    match classify_fetch_error("request failed for https://x: reset") {
        FetchFailureKind::Send(_) => {}
        other => panic!("send message must classify as send, got {other:?}"),
    }
    match classify_fetch_error("request timed out for https://x") {
        FetchFailureKind::Send(_) => {}
        other => panic!("timeout message must classify as send, got {other:?}"),
    }
    match classify_fetch_error("HTTP 404 fetching https://x") {
        FetchFailureKind::UnexpectedStatus(404) => {}
        other => panic!("status message must classify as status, got {other:?}"),
    }
    match classify_fetch_error("HTTP nope") {
        FetchFailureKind::Send(_) => {}
        other => panic!("unparsable status must fall back to send, got {other:?}"),
    }
    match classify_fetch_error("failed to read body for https://x: truncated") {
        FetchFailureKind::BodyRead(_) => {}
        other => panic!("body message must classify as body, got {other:?}"),
    }
    match classify_fetch_error("timed out reading body for https://x") {
        FetchFailureKind::BodyRead(_) => {}
        other => panic!("body timeout must classify as body, got {other:?}"),
    }
    match classify_fetch_error("something entirely new") {
        FetchFailureKind::Send(_) => {}
        other => panic!("unknown messages fall back to send, got {other:?}"),
    }
}
