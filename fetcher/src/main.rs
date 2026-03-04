use anyhow::{Context, Result};
use feed_rs::parser;
use home_rss_shared::db;
use spin_sdk::http::{Request, Response};
use spin_sdk::pg4::Connection;
use uuid::Uuid;

fn main() {
    if let Err(e) = spin_sdk::http::run(run()) {
        eprintln!("fetcher error: {e:#}");
        std::process::exit(1);
    }
}

async fn run() -> Result<()> {
    let conn = db::connect()?;
    let feeds = get_feeds(&conn)?;

    println!("home-rss-fetcher: processing {} feed(s)", feeds.len());

    for feed in feeds {
        if let Err(e) = process_feed(&conn, &feed).await {
            eprintln!("feed {}: {e:#}", feed.url);
        }
    }

    println!("home-rss-fetcher: done");
    Ok(())
}

struct FeedRecord {
    id: Uuid,
    url: String,
    etag: Option<String>,
    last_modified: Option<String>,
}

fn get_feeds(conn: &Connection) -> Result<Vec<FeedRecord>> {
    let rowset = conn
        .query("SELECT id, url, etag, last_modified FROM feeds", &[])
        .context("query feeds")?;

    let mut feeds = Vec::new();
    for row in rowset.rows() {
        let id: Uuid = row.get("id").ok_or_else(|| anyhow::anyhow!("missing id"))?;
        let url: String = row.get("url").ok_or_else(|| anyhow::anyhow!("missing url"))?;
        let etag: Option<String> = row.get::<String>("etag");
        let last_modified: Option<String> = row.get::<String>("last_modified");
        feeds.push(FeedRecord { id, url, etag, last_modified });
    }

    Ok(feeds)
}

async fn process_feed(conn: &Connection, feed: &FeedRecord) -> Result<()> {
    let mut builder = Request::get(&feed.url);
    if let Some(etag) = &feed.etag {
        builder.header("If-None-Match", etag);
    }
    if let Some(lm) = &feed.last_modified {
        builder.header("If-Modified-Since", lm);
    }
    let req = builder.build();

    let resp: Response = spin_sdk::http::send(req).await.context("send HTTP request")?;

    if *resp.status() == 304 {
        return Ok(());
    }
    if *resp.status() != 200 {
        anyhow::bail!("HTTP {}", resp.status());
    }

    let new_etag = resp.header("etag").and_then(|h| h.as_str()).map(String::from);
    let new_lm = resp
        .header("last-modified")
        .and_then(|h| h.as_str())
        .map(String::from);
    let body = resp.into_body();

    let feed_data = parser::parse(&body[..]).context("parse feed")?;

    let title = feed_data.title.map(|t| t.content);
    let site_url = feed_data
        .links
        .into_iter()
        .find(|l| l.rel.as_deref() != Some("self"))
        .map(|l| l.href);

    conn.execute(
        "UPDATE feeds \
         SET title=$1, site_url=$2, etag=$3, last_modified=$4, last_fetched_at=NOW() \
         WHERE id=$5",
        &[
            title.into(),
            site_url.into(),
            new_etag.into(),
            new_lm.into(),
            feed.id.into(),
        ],
    )
    .context("update feed")?;

    let mut count = 0usize;
    for entry in feed_data.entries {
        let url = match entry.links.into_iter().next() {
            Some(link) => link.href,
            None => continue,
        };
        let entry_title = entry.title.map(|t| t.content).unwrap_or_default();
        let content = entry
            .content
            .and_then(|c| c.body)
            .or_else(|| entry.summary.map(|s| s.content));
        let author = entry.authors.into_iter().next().map(|a| a.name);
        let published_at = entry
            .published
            .or(entry.updated)
            .map(|dt| dt.to_rfc3339());

        conn.execute(
            "INSERT INTO articles (feed_id, url, title, content, author, published_at) \
             VALUES ($1, $2, $3, $4, $5, $6::timestamptz) \
             ON CONFLICT (feed_id, url) DO NOTHING",
            &[
                feed.id.into(),
                url.into(),
                entry_title.into(),
                content.into(),
                author.into(),
                published_at.into(),
            ],
        )
        .context("insert article")?;
        count += 1;
    }

    println!("feed {}: {} article(s) processed", feed.url, count);
    Ok(())
}
