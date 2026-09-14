use anyhow::Result;
use home_rss_shared::db;
use home_rss_shared::feed::{self, Fetched};
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
            fetch_feed(&conn, &id, &url, etag.as_deref(), last_modified.as_deref()).await
        {
            // One unreachable feed must not stop the rest of the batch.
            eprintln!("Failed to process feed {url}: {e:#}");
        }
    }

    Ok(())
}

async fn fetch_feed(
    conn: &Connection,
    feed_id: &str,
    url: &str,
    etag: Option<&str>,
    last_modified: Option<&str>,
) -> Result<()> {
    if let Fetched::Modified(parsed) = feed::fetch(url, etag, last_modified).await? {
        feed::store(conn, feed_id, &parsed).await?;
    }
    Ok(())
}
