use anyhow::Result;
use home_rss_shared::db;
use home_rss_shared::fetch::{FetchAndStoreOutcome, fetch_and_store};
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
async fn process_feed(
    conn: &Connection,
    feed_id: &str,
    url: &str,
    etag: Option<&str>,
    last_modified: Option<&str>,
) -> Result<()> {
    match fetch_and_store(conn, feed_id, url, etag, last_modified, None).await {
        FetchAndStoreOutcome::NotModified | FetchAndStoreOutcome::Stored(_) => Ok(()),
        FetchAndStoreOutcome::FetchFailed(e)
        | FetchAndStoreOutcome::Unparseable(e)
        | FetchAndStoreOutcome::StoreFailed(e) => Err(e),
    }
}
