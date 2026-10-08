//! 定期取得の結果を失敗記録 (`last_fetch_error` / `fetch_failing_since`) へ振り分ける純粋関数 (#245)。

use home_rss_shared::fetch::FetchAndStoreOutcome;

/// 取得結果に対する失敗記録の扱い。
#[derive(Debug, PartialEq, Eq)]
#[allow(dead_code)]
pub enum FailureAction {
    /// 失敗を記録する。reason は 200 文字以内に切り詰め済み。
    Record { reason: String },
    /// 成功したので記録を消す。
    Clear,
    /// 記録を変えない。
    Keep,
}

/// `outcome` を記録・消去・無変更に振り分ける。`ever_fetched` はそのフィードを
/// 一度でも取得できたこと (`last_fetched_at IS NOT NULL`) で、一度も取得して
/// いない行には印を付けない。SQL (`UPDATE feeds ...`) は `process_feed` 側が書く。
#[allow(dead_code)]
pub fn decide_failure_action(outcome: &FetchAndStoreOutcome, ever_fetched: bool) -> FailureAction {
    match outcome {
        FetchAndStoreOutcome::Stored(_) | FetchAndStoreOutcome::NotModified => FailureAction::Clear,
        FetchAndStoreOutcome::FetchFailed(e) | FetchAndStoreOutcome::Unparseable(e) => {
            if ever_fetched {
                FailureAction::Record {
                    reason: failure_reason(e),
                }
            } else {
                FailureAction::Keep
            }
        }
        FetchAndStoreOutcome::StoreFailed(_) => FailureAction::Keep,
    }
}

/// anyhow のエラーを `last_fetch_error` に入れる短い理由にする。200 文字
/// （文字数。バイト数ではない）で切る。ガード拒否と解釈不能は種類が分かる
/// 文言を付け、想定外ステータスは `fetch_only` が作る `HTTP {status} ...`
/// の文面をそのまま残す (#245)。
fn failure_reason(e: &anyhow::Error) -> String {
    let raw = format!("{e:#}");
    let reasoned = if raw.contains("SSRF guard rejected") {
        format!("URL ガードで拒否: {raw}")
    } else if raw.contains("could not be parsed") {
        format!("フィードとして解釈できない: {raw}")
    } else {
        raw
    };
    truncate_chars(&reasoned, 200)
}

/// 先頭から `limit` 文字だけ残す。マルチバイト文字の途中で切らない (#245)。
fn truncate_chars(s: &str, limit: usize) -> String {
    match s.char_indices().nth(limit) {
        Some((idx, _)) => s[..idx].to_owned(),
        None => s.to_owned(),
    }
}

#[cfg(test)]
mod tests {
    use super::{FailureAction, decide_failure_action};
    use home_rss_shared::fetch::FetchAndStoreOutcome;
    use home_rss_shared::models::Feed;

    fn stored_feed() -> Feed {
        Feed {
            id: "00000000-0000-0000-0000-000000000000".to_owned(),
            url: "https://example.com/feed".to_owned(),
            title: None,
            site_url: None,
            etag: None,
            last_modified: None,
            last_fetched_at: None,
            created_at: None,
            last_fetch_error: None,
            fetch_failing_since: None,
        }
    }

    fn record_reason(outcome: &FetchAndStoreOutcome, ever_fetched: bool) -> String {
        match decide_failure_action(outcome, ever_fetched) {
            FailureAction::Record { reason } => reason,
            other => panic!("must record the failure, got {other:?}"),
        }
    }

    #[test]
    fn fetch_failed_records_when_previously_fetched() {
        // 常に Keep を返す退行を捕まえる: 取得済みの失敗は記録しなければならない。
        let outcome = FetchAndStoreOutcome::FetchFailed(anyhow::anyhow!("HTTP 500 fetching x"));
        let reason = record_reason(&outcome, true);
        assert!(!reason.is_empty());
        assert!(reason.chars().count() <= 200);
    }

    #[test]
    fn fetch_failed_leaves_never_fetched_feed_unmarked() {
        // 一度も取得していない行には印を付けない (OPML 直後など)。
        let outcome = FetchAndStoreOutcome::FetchFailed(anyhow::anyhow!("HTTP 500 fetching x"));
        assert_eq!(decide_failure_action(&outcome, false), FailureAction::Keep);
    }

    #[test]
    fn unparseable_body_is_recorded_as_failure() {
        // 解釈不能の本文も失敗に数える。
        let outcome =
            FetchAndStoreOutcome::Unparseable(anyhow::anyhow!("feed at x could not be parsed"));
        let reason = record_reason(&outcome, true);
        assert!(!reason.is_empty());
        assert!(reason.chars().count() <= 200);
    }

    #[test]
    fn unparseable_body_leaves_never_fetched_feed_unmarked() {
        let outcome =
            FetchAndStoreOutcome::Unparseable(anyhow::anyhow!("feed at x could not be parsed"));
        assert_eq!(decide_failure_action(&outcome, false), FailureAction::Keep);
    }

    #[test]
    fn stored_success_clears_failure_record() {
        // 200 の成功は記録を消す。常に Keep だと消えずに残る。
        let outcome = FetchAndStoreOutcome::Stored(Box::new(stored_feed()));
        assert_eq!(decide_failure_action(&outcome, true), FailureAction::Clear);
    }

    #[test]
    fn not_modified_clears_failure_record() {
        // 304 も記録を消す。常に Keep だと消えずに残る。
        assert_eq!(
            decide_failure_action(&FetchAndStoreOutcome::NotModified, true),
            FailureAction::Clear
        );
    }

    #[test]
    fn store_failure_leaves_record_untouched() {
        // StoreFailed は記録を変えない (残っていれば残す、無ければ作らない)。
        // Record 側への倒し込みを捕まえる。
        let outcome = FetchAndStoreOutcome::StoreFailed(anyhow::anyhow!("db is down"));
        assert_eq!(decide_failure_action(&outcome, true), FailureAction::Keep);
        assert_eq!(decide_failure_action(&outcome, false), FailureAction::Keep);
    }

    #[test]
    fn guard_rejection_reason_mentions_guard() {
        // URL ガードでの拒否は理由にガードと分かる文言を入れる。
        let outcome = FetchAndStoreOutcome::FetchFailed(anyhow::anyhow!(
            "SSRF guard rejected feed f: https://localhost/feed (must not point to internal)"
        ));
        assert!(
            record_reason(&outcome, true).contains("ガード"),
            "guard rejection must be recognizable in the reason"
        );
    }

    #[test]
    fn http_status_reason_mentions_status() {
        // 想定外ステータスは番号が残る (例「HTTP 404」)。
        let outcome = FetchAndStoreOutcome::FetchFailed(anyhow::anyhow!(
            "HTTP 404 fetching https://example.com/feed"
        ));
        assert!(
            record_reason(&outcome, true).contains("404"),
            "unexpected status must keep its number in the reason"
        );
    }

    #[test]
    fn unparseable_reason_mentions_parse_problem() {
        let outcome =
            FetchAndStoreOutcome::Unparseable(anyhow::anyhow!("feed at x could not be parsed"));
        assert!(
            record_reason(&outcome, true).contains("解釈"),
            "unparseable body must be recognizable in the reason"
        );
    }

    #[test]
    fn reason_is_truncated_to_200_chars() {
        // 理由は 200 文字で切る。文字数 (バイト数ではない) で数える。
        let long = format!("送信エラー: {}", "あ".repeat(500));
        let outcome = FetchAndStoreOutcome::FetchFailed(anyhow::anyhow!(long));
        assert!(record_reason(&outcome, true).chars().count() <= 200);
    }
}
