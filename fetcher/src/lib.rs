use anyhow::Result;
use home_rss_shared::db;
use home_rss_shared::fetch::FetchFailureAction;
use home_rss_shared::fetch::{FetchAndStoreOutcome, classify_fetch_failure, fetch_and_store};
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
            "SELECT id::text, url, etag, last_modified FROM feeds",
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

        if let Err(e) =
            process_feed(&conn, &id, &url, etag.as_deref(), last_modified.as_deref()).await
        {
            eprintln!("Failed to process feed {url}: {e:#}");
        }
    }

    Ok(())
}

/// POST /api/feeds の即時取得と定期取得で共有する「取得して保存する」処理は
/// home_rss_shared::fetch::fetch_and_store に置く (#106)。timeout に None を渡し、
/// 既存の定期取得の振る舞い（打ち切りなし）をそのまま保つ。
/// 失敗の記録 (#245) は取得と保存の共有処理 (`store()`) には足さず、ここで
/// 独立の UPDATE として書く・消す。即時取得 (`POST /api/feeds`) は `store_fetched()`
/// 経由で `store()` だけを通るので、この記録を書きも消しもしない。
async fn process_feed(
    conn: &Connection,
    feed_id: &str,
    url: &str,
    etag: Option<&str>,
    last_modified: Option<&str>,
) -> Result<()> {
    let outcome = fetch_and_store(conn, feed_id, url, etag, last_modified, None).await;
    let action = classify_fetch_failure(&outcome);
    match action {
        FetchFailureAction::Record { reason } => {
            record_failure(conn, feed_id, &reason).await?;
        }
        FetchFailureAction::Clear => {
            clear_failure(conn, feed_id).await?;
        }
        FetchFailureAction::Keep => {}
    }
    match outcome {
        FetchAndStoreOutcome::NotModified | FetchAndStoreOutcome::Stored(_) => Ok(()),
        FetchAndStoreOutcome::FetchFailed(e)
        | FetchAndStoreOutcome::Unparseable(e)
        | FetchAndStoreOutcome::StoreFailed(e) => Err(e),
    }
}

/// 失敗の記録。連続失敗の始まり (`fetch_failing_since`) は上書きせず、
/// 成功で NULL に戻るまでは最初の失敗の時刻を保つ。一度も取得していない
/// フィード (`last_fetched_at IS NULL`、OPML から入れた直後など) には
/// 印を付けない。
async fn record_failure(conn: &Connection, feed_id: &str, reason: &str) -> Result<()> {
    conn.execute(
        "UPDATE feeds SET last_fetch_error = $1, \
         fetch_failing_since = COALESCE(fetch_failing_since, NOW()) \
         WHERE id = $2 AND last_fetched_at IS NOT NULL",
        vec![
            ParameterValue::Str(reason.to_owned()),
            ParameterValue::Uuid(feed_id.to_owned()),
        ],
    )
    .await?;
    Ok(())
}

/// 成功の記録消し。失敗の列だけを NULL に戻す。304 は `last_fetched_at`
/// を更新しない既存動作のまま (#245) なので 304 用の文はこの 2 列だけを
/// SET し、200 は `store()` が `last_fetched_at` まで更新済みなのでここでは
/// 失敗の列だけを消す。どちらも同じ文になる。
async fn clear_failure(conn: &Connection, feed_id: &str) -> Result<()> {
    conn.execute(
        "UPDATE feeds SET last_fetch_error = NULL, fetch_failing_since = NULL \
         WHERE id = $1",
        vec![ParameterValue::Uuid(feed_id.to_owned())],
    )
    .await?;
    Ok(())
}
