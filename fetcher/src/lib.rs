use anyhow::Result;
use home_rss_shared::db;
use home_rss_shared::fetch::{FetchAndStoreOutcome, classify_fetch_failure, fetch_and_store};
use home_rss_shared::http::{Resp, text};
use spin_sdk::http::{Request, StatusCode};
use spin_sdk::http_service;
use spin_sdk::pg::{Connection, Decode};

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
            "SELECT id::text, url, etag, last_modified, \
             (last_fetched_at IS NOT NULL) FROM feeds",
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
/// 既存の定期取得の振る舞い（打ち切りなし）をそのまま保つ。
async fn process_feed(
    conn: &Connection,
    feed_id: &str,
    url: &str,
    etag: Option<&str>,
    last_modified: Option<&str>,
    ever_fetched: bool,
) -> Result<()> {
    let outcome = fetch_and_store(conn, feed_id, url, etag, last_modified, None).await;
    record_fetch_failure(conn, feed_id, &outcome, ever_fetched).await?;
    match outcome {
        FetchAndStoreOutcome::NotModified | FetchAndStoreOutcome::Stored(_) => Ok(()),
        FetchAndStoreOutcome::FetchFailed(e)
        | FetchAndStoreOutcome::Unparseable(e)
        | FetchAndStoreOutcome::StoreFailed(e) => Err(e),
    }
}

/// 定期取得の結果に応じた失敗記録の独立した書き込み (#245)。`store()` とは
/// 別の列だけを触るので、即時取得の保存と干渉しない。304 は `last_fetched_at`
/// を更新せず失敗の列だけ消す。
async fn record_fetch_failure(
    conn: &Connection,
    feed_id: &str,
    outcome: &FetchAndStoreOutcome,
    ever_fetched: bool,
) -> Result<()> {
    use home_rss_shared::fetch::FetchFailureAction;
    use spin_sdk::pg::ParameterValue;
    match classify_fetch_failure(outcome, ever_fetched) {
        FetchFailureAction::Keep => Ok(()),
        FetchFailureAction::Record(reason) => {
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
            Ok(())
        }
        FetchFailureAction::Clear => {
            conn.execute(
                "UPDATE feeds SET last_fetch_error = NULL, fetch_failing_since = NULL \
                 WHERE id = $1",
                vec![ParameterValue::Uuid(feed_id.to_owned())],
            )
            .await?;
            Ok(())
        }
    }
}
