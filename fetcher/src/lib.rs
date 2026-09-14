use anyhow::Result;
use home_rss_shared::db;
use home_rss_shared::feed::{FetchOutcome, fetch_and_store};
use home_rss_shared::http::{Resp, text};
use spin_sdk::http::{Request, StatusCode};
use spin_sdk::http_service;
use spin_sdk::pg::Decode;

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

        match fetch_and_store(&conn, &id, &url, etag.as_deref(), last_modified.as_deref()).await {
            Ok(FetchOutcome::Stored { new_articles }) => {
                eprintln!("Fetched {url}: {new_articles} new articles");
            }
            Ok(FetchOutcome::NotModified) => {}
            Err(e) => eprintln!("Failed to process feed {url}: {e:#}"),
        }
    }

    Ok(())
}
