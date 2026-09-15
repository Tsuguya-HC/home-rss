use anyhow::Result;
use feed_rs::parser;
use home_rss_shared::db;
use home_rss_shared::http::{Resp, text};
use spin_sdk::http::body::IncomingBodyExt;
use spin_sdk::http::{EmptyBody, Request, Response, StatusCode, send};
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

async fn process_feed(
    conn: &Connection,
    feed_id: &str,
    url: &str,
    etag: Option<&str>,
    last_modified: Option<&str>,
) -> Result<()> {
    let mut builder = Request::get(url).header("user-agent", "home-rss-fetcher/0.1");
    if let Some(etag) = etag {
        builder = builder.header("if-none-match", etag);
    }
    if let Some(lm) = last_modified {
        builder = builder.header("if-modified-since", lm);
    }
    let req = builder.body(EmptyBody::new())?;

    let resp: Response = send(req).await?;

    if resp.status() == StatusCode::NOT_MODIFIED {
        return Ok(());
    }
    if resp.status() != StatusCode::OK {
        anyhow::bail!("HTTP {} fetching {url}", resp.status());
    }

    let new_etag = header_string(&resp, "etag");
    let new_last_modified = header_string(&resp, "last-modified");

    let body = resp.into_body().bytes().await?;
    let parsed = parse_feed_bytes(body.as_ref())?;

    for entry in &parsed.entries {
        conn.execute(
            "INSERT INTO articles (feed_id, url, title, content, author, published_at) \
             VALUES ($1, $2, $3, $4, $5, $6::text::timestamptz) ON CONFLICT DO NOTHING",
            vec![
                ParameterValue::Uuid(feed_id.to_owned()),
                entry.url.clone().into(),
                entry.title.clone().into(),
                entry.content.clone().into(),
                entry.author.clone().into(),
                entry.published_at.clone().into(),
            ],
        )
        .await?;
    }

    conn.execute(
        "UPDATE feeds SET title = $1, site_url = $2, etag = $3, last_modified = $4, \
         last_fetched_at = NOW() WHERE id = $5",
        vec![
            parsed.title.into(),
            parsed.site_url.into(),
            new_etag.into(),
            new_last_modified.into(),
            ParameterValue::Uuid(feed_id.to_owned()),
        ],
    )
    .await?;

    Ok(())
}

fn header_string(resp: &Response, name: &str) -> Option<String> {
    resp.headers()
        .get(name)
        .and_then(|v| v.to_str().ok())
        .map(str::to_owned)
}

/// POST /api/feeds の即時取得と定期取得で共有する純粋なパース (#106)。
/// HTTP 取得・DB 書き込みは含まず、本文バイトから記事レコードの材料だけを取り出す。
/// パース不能は Err として返し、呼び出し側がユーザーへの通知に使う。
#[derive(Debug, PartialEq)]
struct ParsedEntry {
    url: String,
    title: String,
    content: Option<String>,
    author: Option<String>,
    /// RFC 3339。entry.published が無ければ entry.updated を使う。
    published_at: Option<String>,
}

#[derive(Debug, PartialEq)]
struct ParsedFeed {
    title: Option<String>,
    site_url: Option<String>,
    entries: Vec<ParsedEntry>,
}

fn parse_feed_bytes(body: &[u8]) -> Result<ParsedFeed> {
    let feed = parser::parse(body)?;
    Ok(ParsedFeed {
        title: feed.title.as_ref().map(|t| t.content.clone()),
        site_url: feed.links.first().map(|l| l.href.clone()),
        entries: feed
            .entries
            .iter()
            .filter_map(|entry| {
                let entry_url = entry.links.first()?.href.clone();
                let entry_title = entry
                    .title
                    .as_ref()
                    .map(|t| t.content.clone())
                    .unwrap_or_else(|| "(no title)".to_owned());
                let content = entry
                    .content
                    .as_ref()
                    .and_then(|c| c.body.clone())
                    .or_else(|| entry.summary.as_ref().map(|s| s.content.clone()));
                let author = entry.authors.first().map(|a| a.name.clone());
                let published_at = entry
                    .published
                    .or(entry.updated)
                    .map(|dt| dt.to_rfc3339());
                Some(ParsedEntry {
                    url: entry_url,
                    title: entry_title,
                    content,
                    author,
                    published_at,
                })
            })
            .collect(),
    })
}

#[cfg(test)]
mod tests {
    use super::parse_feed_bytes;

    const RSS: &[u8] = br#"<?xml version="1.0"?>
<rss version="2.0"><channel>
<title>Example Feed</title>
<link>https://example.com/</link>
<item>
<title>Hello</title>
<link>https://example.com/hello</link>
<description>world</description>
<pubDate>Mon, 15 Sep 2026 00:00:00 GMT</pubDate>
</item>
</channel></rss>"#;

    #[test]
    fn parses_feed_title_site_url_and_entry() {
        let feed = parse_feed_bytes(RSS).unwrap();
        assert_eq!(feed.title.as_deref(), Some("Example Feed"));
        assert_eq!(feed.site_url.as_deref(), Some("https://example.com/"));
        assert_eq!(feed.entries.len(), 1);
        assert_eq!(feed.entries[0].url, "https://example.com/hello");
        assert_eq!(feed.entries[0].title, "Hello");
    }

    #[test]
    fn entry_without_link_is_skipped() {
        let xml = br#"<?xml version="1.0"?>
<rss version="2.0"><channel>
<title>T</title>
<item><title>No link</title></item>
<item><title>Has link</title><link>https://example.com/x</link></item>
</channel></rss>"#;
        let feed = parse_feed_bytes(xml).unwrap();
        assert_eq!(feed.entries.len(), 1);
        assert_eq!(feed.entries[0].url, "https://example.com/x");
    }

    #[test]
    fn entry_without_title_gets_default() {
        let xml = br#"<?xml version="1.0"?>
<rss version="2.0"><channel>
<title>T</title>
<item><link>https://example.com/x</link></item>
</channel></rss>"#;
        let feed = parse_feed_bytes(xml).unwrap();
        assert_eq!(feed.entries.len(), 1);
        assert_eq!(feed.entries[0].title, "(no title)");
    }

    #[test]
    fn unparseable_body_is_an_error() {
        assert!(parse_feed_bytes(b"this is not a feed").is_err());
    }
}
