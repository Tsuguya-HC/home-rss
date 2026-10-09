use crate::failure::{FetchFailureAction, decide_fetch_failure_action};
use anyhow::Result;
use home_rss_shared::db;
use home_rss_shared::fetch::{FetchAndStoreOutcome, fetch_and_store};

pub mod failure;
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
/// 既存の定期取得の振る舞い（打ち切りなし）をそのまま保つ。失敗の記録は
/// 共通の `store()` ではなくここで独立の UPDATE として書く・消す (#245):
/// `store()` は即時取得も通るため、そちらがこの記録に触れないよう、
/// 判定 (`decide_fetch_failure_action`) と UPDATE の両方を fetcher 側に置く。
/// 304 では失敗の列だけを書き、`last_fetched_at` には触れない。
async fn process_feed(
    conn: &Connection,
    feed_id: &str,
    url: &str,
    etag: Option<&str>,
    last_modified: Option<&str>,
    ever_fetched: bool,
) -> Result<()> {
    let outcome = fetch_and_store(conn, feed_id, url, etag, last_modified, None).await;
    let result = match decide_fetch_failure_action(&outcome, ever_fetched) {
        FetchFailureAction::Keep => Ok(()),
        FetchFailureAction::Clear => clear_fetch_failure(conn, feed_id).await,
        FetchFailureAction::Record { reason } => record_fetch_failure(conn, feed_id, &reason).await,
    };
    match outcome {
        FetchAndStoreOutcome::NotModified | FetchAndStoreOutcome::Stored(_) => result,
        FetchAndStoreOutcome::FetchFailed(e)
        | FetchAndStoreOutcome::Unparseable(e)
        | FetchAndStoreOutcome::StoreFailed(e) => Err(e),
    }
}

/// 直近の失敗を記録する。連続失敗の始まり (`fetch_failing_since`) は、
/// 失敗が続いている間は上書きしない。
async fn record_fetch_failure(conn: &Connection, feed_id: &str, reason: &str) -> Result<()> {
    conn.execute(
        "UPDATE feeds SET last_fetch_error = $1, \
         fetch_failing_since = COALESCE(fetch_failing_since, NOW()) WHERE id = $2",
        vec![
            spin_sdk::pg::ParameterValue::Str(reason.to_owned()),
            spin_sdk::pg::ParameterValue::Uuid(feed_id.to_owned()),
        ],
    )
    .await?;
    Ok(())
}

/// 成功したので記録を消す。200 でも 304 でも、失敗の列だけを NULL に戻す。
async fn clear_fetch_failure(conn: &Connection, feed_id: &str) -> Result<()> {
    conn.execute(
        "UPDATE feeds SET last_fetch_error = NULL, fetch_failing_since = NULL WHERE id = $1",
        vec![spin_sdk::pg::ParameterValue::Uuid(feed_id.to_owned())],
    )
    .await?;
    Ok(())
}
