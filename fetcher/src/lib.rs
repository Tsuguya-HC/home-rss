use anyhow::Result;
use home_rss_shared::db;
use home_rss_shared::fetch::{FetchAndStoreOutcome, fetch_and_store};
use home_rss_shared::fetch_failure::{
    FailureAction, ScheduledFetchOutcome, classify_fetch_error, decide_failure_action,
};
use home_rss_shared::http::{Resp, text};
use spin_sdk::http::{Request, StatusCode};
use spin_sdk::http_service;
use spin_sdk::pg::{Connection, Decode, ParameterValue};

#[http_service]
async fn handle_fetch(_req: Request) -> Resp {
    match fetch_all_feeds().await {
        Ok(()) => text(StatusCode::OK, "ok"),
        Err(e) => {
            eprintln!("fetcher error: {e:#}");
            text(StatusCode::INTERNAL_SERVER_ERROR, format!("error: {e:#}"))
        }
    }
}

async fn fetch_all_feeds() -> Result<()> {
    let conn = db::connect().await?;
    let rows = conn
        .query(
            "SELECT id::text, url, etag, last_modified, last_fetched_at IS NOT NULL FROM feeds",
            vec![],
        )
        .await?
        .collect()
        .await?;

    for row in &rows {
        let id = String::decode(&row[0])?;
        let url = String::decode(&row[1])?;
        let etag = Option::<String>::decode(&row[2])?;
        let last_modified = Option::<String>::decode(&row[3])?;
        let ever_fetched = bool::decode(&row[4])?;

        if let Err(e) = process_feed(
            &conn,
            &id,
            &url,
            etag.as_deref(),
            last_modified.as_deref(),
            ever_fetched,
        )
        .await
        {
            eprintln!("Failed to process feed {url}: {e:#}");
        }
    }

    Ok(())
}

/// POST /api/feeds の即時取得と定期取得で共有する「取得して保存する」処理は
/// home_rss_shared::fetch::fetch_and_store に置く (#106)。timeout に None を渡し、
/// 既存の定期取得の振る舞い（打ち切りなし）をそのまま保つ。取得結果に応じた
/// 失敗の記録・消去は `shared::fetch_failure` の振り分けに従う独立の
/// `UPDATE feeds` で行う (#245)。共通の `store()` には足さない（即時取得も
/// 通るため）。応答しないフィード（打ち切らない定期取得がその回に返さない）
/// の記録は範囲外で、ここには来ない。
async fn process_feed(
    conn: &Connection,
    feed_id: &str,
    url: &str,
    etag: Option<&str>,
    last_modified: Option<&str>,
    ever_fetched: bool,
) -> Result<()> {
    let outcome = fetch_and_store(conn, feed_id, url, etag, last_modified, None).await;
    let scheduled = match &outcome {
        FetchAndStoreOutcome::NotModified => ScheduledFetchOutcome::NotModified,
        FetchAndStoreOutcome::Stored(_) => ScheduledFetchOutcome::Stored,
        FetchAndStoreOutcome::FetchFailed(e) => {
            ScheduledFetchOutcome::FetchFailed(classify_fetch_error(&format!("{e:#}")))
        }
        FetchAndStoreOutcome::Unparseable(_) => ScheduledFetchOutcome::Unparseable,
        FetchAndStoreOutcome::StoreFailed(_) => ScheduledFetchOutcome::StoreFailed,
    };
    match decide_failure_action(&scheduled, ever_fetched) {
        FailureAction::Record { reason } => {
            conn.execute(
                "UPDATE feeds SET last_fetch_error = $1, \
                 fetch_failing_since = COALESCE(fetch_failing_since, NOW()) \
                 WHERE id = $2",
                vec![
                    ParameterValue::Str(reason),
                    ParameterValue::Uuid(feed_id.to_owned()),
                ],
            )
            .await?;
        }
        FailureAction::Clear => {
            conn.execute(
                "UPDATE feeds SET last_fetch_error = NULL, fetch_failing_since = NULL \
                 WHERE id = $1",
                vec![ParameterValue::Uuid(feed_id.to_owned())],
            )
            .await?;
        }
        FailureAction::Keep => {}
    }
    match outcome {
        FetchAndStoreOutcome::NotModified | FetchAndStoreOutcome::Stored(_) => Ok(()),
        FetchAndStoreOutcome::FetchFailed(e)
        | FetchAndStoreOutcome::Unparseable(e)
        | FetchAndStoreOutcome::StoreFailed(e) => Err(e),
    }
}
