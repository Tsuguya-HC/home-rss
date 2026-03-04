use anyhow::Result;
use chrono::NaiveDateTime;
use feed_rs::parser;
use home_rss_shared::db;
use spin_sdk::http::{send, Method, Request, Response};
use spin_sdk::pg4::{Connection, Decode};

fn main() {
    if let Err(e) = spin_sdk::http::run(fetch_all_feeds()) {
        eprintln!("fetcher error: {e:#}");
        std::process::exit(1);
    }
}

async fn fetch_all_feeds() -> Result<()> {
    let conn = db::connect()?;
    let result = conn.query(
        "SELECT id::text, url, etag, last_modified FROM feeds",
        &[],
    )?;

    for row in &result.rows {
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

async fn process_feed(
    conn: &Connection,
    feed_id: &str,
    url: &str,
    etag: Option<&str>,
    last_modified: Option<&str>,
) -> Result<()> {
    let mut builder = Request::builder();
    builder
        .method(Method::Get)
        .uri(url)
        .header("user-agent", "home-rss-fetcher/0.1");
    if let Some(etag) = etag {
        builder.header("if-none-match", etag);
    }
    if let Some(lm) = last_modified {
        builder.header("if-modified-since", lm);
    }
    let req = builder.build();

    let resp: Response = send(req).await?;

    if *resp.status() == 304 {
        return Ok(());
    }
    if *resp.status() != 200 {
        anyhow::bail!("HTTP {} fetching {url}", resp.status());
    }

    let new_etag = resp
        .header("etag")
        .and_then(|v| v.as_str())
        .map(str::to_owned);
    let new_last_modified = resp
        .header("last-modified")
        .and_then(|v| v.as_str())
        .map(str::to_owned);

    let body = resp.into_body();
    let feed = parser::parse(body.as_slice())?;

    let feed_title = feed.title.as_ref().map(|t| t.content.clone());
    let site_url = feed.links.first().map(|l| l.href.clone());

    for entry in &feed.entries {
        let entry_url = match entry.links.first() {
            Some(l) => &l.href,
            None => continue,
        };
        let entry_title = entry
            .title
            .as_ref()
            .map(|t| t.content.as_str())
            .unwrap_or("(no title)");
        let content = entry
            .content
            .as_ref()
            .and_then(|c| c.body.as_deref())
            .or_else(|| entry.summary.as_ref().map(|s| s.content.as_str()));
        let author = entry.authors.first().map(|a| a.name.as_str());
        let published_at: Option<NaiveDateTime> =
            entry.published.or(entry.updated).map(|dt| dt.naive_utc());

        conn.execute(
            "INSERT INTO articles (feed_id, url, title, content, author, published_at) \
             VALUES ($1::uuid, $2, $3, $4, $5, $6) ON CONFLICT DO NOTHING",
            &[
                feed_id.to_owned().into(),
                entry_url.to_owned().into(),
                entry_title.to_owned().into(),
                content.map(str::to_owned).into(),
                author.map(str::to_owned).into(),
                published_at.into(),
            ],
        )?;
    }

    conn.execute(
        "UPDATE feeds SET title = $1, site_url = $2, etag = $3, last_modified = $4, \
         last_fetched_at = NOW() WHERE id = $5::uuid",
        &[
            feed_title.into(),
            site_url.into(),
            new_etag.into(),
            new_last_modified.into(),
            feed_id.to_owned().into(),
        ],
    )?;

    Ok(())
}
