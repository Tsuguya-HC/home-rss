use home_rss_shared::fetch::FetchAndStoreOutcome;

/// 定期取得の1件分の結果を、失敗の記録・記録の削除・何もしない、のいずれかに
/// 振り分ける純粋関数 (#245)。DB には触らない。実際の UPDATE は呼び出し側
/// (process_feed) が行う。fetcher に置く: server からは見えない配置なので、
/// 即時取得がこの記録を書きも消しもしないことが配置で保たれる。
#[derive(Debug, PartialEq, Eq)]
pub enum FetchFailureAction {
    /// 失敗として記録する。reason は feeds.last_fetch_error に入る文面。
    Record { reason: String },
    /// 成功したので記録を消す。
    Clear,
    /// 記録を変えない。
    Keep,
}

/// 失敗の理由の上限。DB の TEXT に合わせて「200 文字で切る」(#245)。
/// 切り詰めは `truncate_reason` が文字単位で行う。
pub const MAX_FAILURE_REASON_CHARS: usize = 200;

/// 理由の文面を上限の文字数に収める。`char_indices` で境界を取るので、
/// マルチバイト文字の途中で切って壊れた文字列を作らない。
pub fn truncate_reason(reason: &str) -> String {
    match reason.char_indices().nth(MAX_FAILURE_REASON_CHARS) {
        Some((cut, _)) => reason[..cut].to_owned(),
        None => reason.to_owned(),
    }
}

pub fn decide_fetch_failure_action(
    outcome: &FetchAndStoreOutcome,
    ever_fetched: bool,
) -> FetchFailureAction {
    match outcome {
        FetchAndStoreOutcome::Stored(_) | FetchAndStoreOutcome::NotModified => {
            FetchFailureAction::Clear
        }
        FetchAndStoreOutcome::StoreFailed(_) => FetchFailureAction::Keep,
        FetchAndStoreOutcome::FetchFailed(e) | FetchAndStoreOutcome::Unparseable(e) => {
            // OPML から入れた直後など、一度も取得していないフィードには印を付けない。
            if !ever_fetched {
                return FetchFailureAction::Keep;
            }
            FetchFailureAction::Record {
                reason: truncate_reason(&format!("{e:#}")),
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::truncate_reason;

    #[test]
    fn keeps_short_reasons_untouched() {
        assert_eq!(truncate_reason("HTTP 404"), "HTTP 404");
    }

    #[test]
    fn cuts_exactly_at_the_character_limit() {
        let long = "x".repeat(super::MAX_FAILURE_REASON_CHARS + 50);
        assert_eq!(
            truncate_reason(&long).chars().count(),
            super::MAX_FAILURE_REASON_CHARS
        );
    }

    /// `nth` が返すバイト位置が文字境界であることを固定する: 変換の
    /// オフバイワンで `..cut` が panic しないこと (200 文字目の次が
    /// マルチバイトでも壊さず切る)。
    #[test]
    fn never_splits_a_multibyte_character() {
        let long = format!(
            "{}{}",
            "y".repeat(super::MAX_FAILURE_REASON_CHARS),
            "あいう"
        );
        let cut = truncate_reason(&long);
        assert_eq!(cut.chars().count(), super::MAX_FAILURE_REASON_CHARS);
        assert!(cut.is_char_boundary(cut.len()));
    }

    use super::decide_fetch_failure_action;
    use crate::failure::FetchFailureAction;
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

    fn failed_outcome() -> home_rss_shared::fetch::FetchAndStoreOutcome {
        home_rss_shared::fetch::FetchAndStoreOutcome::FetchFailed(anyhow::anyhow!(
            "HTTP 404 fetching https://example.com/feed"
        ))
    }

    #[test]
    fn fetch_failed_is_recorded() {
        // 送信・受信のエラー、URL ガードでの拒否、想定外ステータスは
        // どれも FetchFailed に正規化されるので、ここでは記録することだけを固定する。
        match decide_fetch_failure_action(&failed_outcome(), true) {
            FetchFailureAction::Record { reason } => {
                assert!(!reason.is_empty());
                assert!(reason.chars().count() <= 200);
            }
            other => panic!("FetchFailed must be recorded, got {other:?}"),
        }
    }

    #[test]
    fn unparseable_body_is_recorded() {
        match decide_fetch_failure_action(
            &home_rss_shared::fetch::FetchAndStoreOutcome::Unparseable(anyhow::anyhow!(
                "feed at https://example.com/feed could not be parsed"
            )),
            true,
        ) {
            FetchFailureAction::Record { .. } => {}
            other => panic!("Unparseable must be recorded, got {other:?}"),
        }
    }

    #[test]
    fn stored_feed_clears_the_record() {
        assert_eq!(
            decide_fetch_failure_action(
                &home_rss_shared::fetch::FetchAndStoreOutcome::Stored(Box::new(stored_feed())),
                true
            ),
            FetchFailureAction::Clear
        );
    }

    #[test]
    fn not_modified_clears_the_record() {
        assert_eq!(
            decide_fetch_failure_action(
                &home_rss_shared::fetch::FetchAndStoreOutcome::NotModified,
                true
            ),
            FetchFailureAction::Clear
        );
    }

    #[test]
    fn store_failure_leaves_the_record_alone() {
        // DB への保存の失敗は失敗の記録を変えない。残っていれば残すし、
        // 無ければ作らない (#245)。
        for ever_fetched in [true, false] {
            assert_eq!(
                decide_fetch_failure_action(
                    &home_rss_shared::fetch::FetchAndStoreOutcome::StoreFailed(anyhow::anyhow!(
                        "db is down"
                    )),
                    ever_fetched,
                ),
                FetchFailureAction::Keep,
                "ever_fetched = {ever_fetched}"
            );
        }
    }

    #[test]
    fn failure_before_any_successful_fetch_leaves_no_mark() {
        // OPML から入れた直後など、一度も取得していないフィードには印を付けない。
        for outcome in [
            failed_outcome(),
            home_rss_shared::fetch::FetchAndStoreOutcome::Unparseable(anyhow::anyhow!(
                "not a feed"
            )),
        ] {
            assert_eq!(
                decide_fetch_failure_action(&outcome, false),
                FetchFailureAction::Keep,
                "got {outcome:?}"
            );
        }
    }

    #[test]
    fn failure_reason_is_truncated_to_200_chars() {
        let long = format!(
            "HTTP 500 fetching https://example.com/feed: {}",
            "x".repeat(500)
        );
        match decide_fetch_failure_action(
            &home_rss_shared::fetch::FetchAndStoreOutcome::FetchFailed(anyhow::anyhow!(long)),
            true,
        ) {
            FetchFailureAction::Record { reason } => {
                assert_eq!(reason.chars().count(), 200);
            }
            other => panic!("FetchFailed must be recorded, got {other:?}"),
        }
    }
}
