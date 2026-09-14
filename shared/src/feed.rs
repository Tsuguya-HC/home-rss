use anyhow::Result;
use feed_rs::parser;
use spin_sdk::http::body::IncomingBodyExt;
use spin_sdk::http::{EmptyBody, Request, Response, StatusCode, send};
use spin_sdk::pg::{Connection, ParameterValue};

/// Immediate outcome of fetching one feed.
#[derive(Debug)]
pub enum FetchOutcome {
    /// The feed answered 304 Not Modified — everything already stored.
    NotModified,
    /// The feed was fetched and its articles stored.
    Stored { new_articles: u64 },
}

/// Failures that are the feed's fault, not ours. The server's add-feed
/// endpoint surfaces these to the user; anything else (e.g. DB errors)
/// stays a plain anyhow error and reports as a 500.
#[derive(Debug)]
pub enum FetchFailure {
    /// Could not retrieve the feed: connection error or non-2xx status.
    Unreachable { url: String, reason: String },
    /// Retrieved, but the body is not a recognizable feed.
    Unparsable { url: String, reason: String },
}

impl std::fmt::Display for FetchFailure {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Unreachable { url, reason } => {
                write!(f, "could not fetch feed at {url}: {reason}")
            }
            Self::Unparsable { url, reason } => {
                write!(f, "response from {url} is not a valid feed: {reason}")
            }
        }
    }
}

impl std::error::Error for FetchFailure {}

/// Download one feed and store its articles. Used by the fetcher (cron) and
/// by the server's add-feed endpoint (immediate fetch on add).
pub async fn fetch_and_store(
    conn: &Connection,
    feed_id: &str,
    url: &str,
    etag: Option<&str>,
    last_modified: Option<&str>,
) -> Result<FetchOutcome> {
    let mut builder = Request::get(url).header("user-agent", "home-rss/0.1");
    if let Some(etag) = etag {
        builder = builder.header("if-none-match", etag);
    }
    if let Some(lm) = last_modified {
        builder = builder.header("if-modified-since", lm);
    }
    let req = builder.body(EmptyBody::new())?;

    let resp: Response = send(req).await.map_err(|e| FetchFailure::Unreachable {
        url: url.to_owned(),
        reason: e.to_string(),
    })?;

    if resp.status() == StatusCode::NOT_MODIFIED {
        return Ok(FetchOutcome::NotModified);
    }
    if resp.status() != StatusCode::OK {
        return Err(FetchFailure::Unreachable {
            url: url.to_owned(),
            reason: format!("HTTP {}", resp.status()),
        }
        .into());
    }

    let new_etag = header_string(&resp, "etag");
    let new_last_modified = header_string(&resp, "last-modified");

    let body = resp.into_body().bytes().await?;
    let feed = parser::parse(body.as_ref()).map_err(|e| FetchFailure::Unparsable {
        url: url.to_owned(),
        reason: e.to_string(),
    })?;

    let mut new_articles = 0u64;
    for article in &extract_articles(&feed) {
        new_articles += conn
            .execute(
                "INSERT INTO articles (feed_id, url, title, content, author, published_at) \
                 VALUES ($1, $2, $3, $4, $5, $6::text::timestamptz) ON CONFLICT DO NOTHING",
                vec![
                    ParameterValue::Uuid(feed_id.to_owned()),
                    article.url.clone().into(),
                    article.title.clone().into(),
                    article.content.clone().into(),
                    article.author.clone().into(),
                    article.published_at.clone().into(),
                ],
            )
            .await?;
    }

    conn.execute(
        "UPDATE feeds SET title = $1, site_url = $2, etag = $3, last_modified = $4, \
         last_fetched_at = NOW() WHERE id = $5",
        vec![
            feed.title.as_ref().map(|t| t.content.clone()).into(),
            feed.links.first().map(|l| l.href.clone()).into(),
            new_etag.into(),
            new_last_modified.into(),
            ParameterValue::Uuid(feed_id.to_owned()),
        ],
    )
    .await?;

    Ok(FetchOutcome::Stored { new_articles })
}

struct ExtractedArticle {
    url: String,
    title: String,
    content: Option<String>,
    author: Option<String>,
    published_at: Option<String>,
}

fn extract_articles(feed: &feed_rs::model::Feed) -> Vec<ExtractedArticle> {
    feed.entries
        .iter()
        .filter_map(|entry| {
            let url = entry.links.first()?.href.clone();
            Some(ExtractedArticle {
                url,
                title: entry
                    .title
                    .as_ref()
                    .map(|t| t.content.clone())
                    .unwrap_or_else(|| "(no title)".to_owned()),
                content: entry
                    .content
                    .as_ref()
                    .and_then(|c| c.body.clone())
                    .or_else(|| entry.summary.as_ref().map(|s| s.content.clone())),
                author: entry.authors.first().map(|a| a.name.clone()),
                published_at: entry.published.or(entry.updated).map(|dt| dt.to_rfc3339()),
            })
        })
        .collect()
}

fn header_string(resp: &Response, name: &str) -> Option<String> {
    resp.headers()
        .get(name)
        .and_then(|v| v.to_str().ok())
        .map(str::to_owned)
}

#[cfg(test)]
mod tests {
    use super::extract_articles;
    use feed_rs::parser;

    #[test]
    fn extracts_fields_from_atom_entries() {
        let xml = br#"<?xml version="1.0" encoding="utf-8"?>
<feed xmlns="http://www.w3.org/2005/Atom">
  <title>t</title>
  <id>urn:feed</id>
  <updated>2026-01-01T00:00:00Z</updated>
  <entry>
    <id>urn:1</id>
    <title>One</title>
    <link href="https://e.example/1"/>
    <updated>2026-01-02T03:04:05Z</updated>
    <summary>sum</summary>
    <author><name>Alice</name></author>
  </entry>
  <entry>
    <id>urn:2</id>
    <link href="https://e.example/2"/>
    <content>body</content>
  </entry>
  <entry>
    <id>urn:3</id>
    <title>No link</title>
  </entry>
</feed>"#;

        let articles = extract_articles(&parser::parse(&xml[..]).unwrap());

        assert_eq!(articles.len(), 2);
        assert_eq!(articles[0].url, "https://e.example/1");
        assert_eq!(articles[0].title, "One");
        // summary is used when there is no content
        assert_eq!(articles[0].content.as_deref(), Some("sum"));
        assert_eq!(articles[0].author.as_deref(), Some("Alice"));
        // published falls back to updated
        assert_eq!(
            articles[0].published_at.as_deref(),
            Some("2026-01-02T03:04:05+00:00")
        );

        assert_eq!(articles[1].url, "https://e.example/2");
        assert_eq!(articles[1].title, "(no title)");
        assert_eq!(articles[1].content.as_deref(), Some("body"));
        assert_eq!(articles[1].author, None);
        assert_eq!(articles[1].published_at, None);
    }

    #[test]
    fn prefers_content_over_summary_in_rss_entries() {
        let xml = br#"<?xml version="1.0"?>
<rss version="2.0" xmlns:content="http://purl.org/rss/1.0/modules/content/">
<channel>
  <title>t</title>
  <link>https://site.example</link>
  <description>d</description>
  <item>
    <title>One</title>
    <link>https://e.example/1</link>
    <description>sum</description>
    <pubDate>Fri, 02 Jan 2026 03:04:05 GMT</pubDate>
    <content:encoded><![CDATA[<p>full</p>]]></content:encoded>
  </item>
</channel>
</rss>"#;

        let articles = extract_articles(&parser::parse(&xml[..]).unwrap());

        assert_eq!(articles.len(), 1);
        assert_eq!(articles[0].url, "https://e.example/1");
        assert_eq!(articles[0].title, "One");
        assert_eq!(articles[0].content.as_deref(), Some("<p>full</p>"));
        assert_eq!(
            articles[0].published_at.as_deref(),
            Some("2026-01-02T03:04:05+00:00")
        );
    }
}
