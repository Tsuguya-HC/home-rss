//! Retrieving and storing a single feed.
//!
//! Both the periodic fetcher and the server (which fetches on `POST /api/feeds`)
//! go through here, so the two paths cannot drift apart in what they write.

use anyhow::{Result, bail};
use feed_rs::parser;
use spin_sdk::http::body::IncomingBodyExt;
use spin_sdk::http::{EmptyBody, Request, Response, StatusCode, send};
use spin_sdk::pg::{Connection, ParameterValue};

/// A feed that has been retrieved and parsed, but not yet written.
pub struct ParsedFeed {
    pub title: Option<String>,
    pub site_url: Option<String>,
    pub etag: Option<String>,
    pub last_modified: Option<String>,
    pub entries: Vec<ParsedEntry>,
}

pub struct ParsedEntry {
    pub url: String,
    pub title: String,
    pub content: Option<String>,
    pub author: Option<String>,
    pub published_at: Option<String>,
}

pub enum Fetched {
    /// The origin answered 304, so there is nothing to write.
    NotModified,
    Modified(ParsedFeed),
}

/// Only `https` is accepted. The cluster policy (CiliumNetworkPolicy) opens
/// world:443 and nothing else, so a plaintext feed would hang rather than fail,
/// and an article body fetched over a channel anyone can rewrite ends up
/// rendered in the UI.
pub fn validate_url(url: &str) -> Result<()> {
    let rest = match url.get(..8) {
        Some(scheme) if scheme.eq_ignore_ascii_case("https://") => &url[8..],
        _ => bail!("feed URL must start with https://"),
    };

    if rest.split(['/', '?', '#']).next().unwrap_or("").is_empty() {
        bail!("feed URL has no host");
    }

    Ok(())
}

pub async fn fetch(url: &str, etag: Option<&str>, last_modified: Option<&str>) -> Result<Fetched> {
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
        return Ok(Fetched::NotModified);
    }
    if resp.status() != StatusCode::OK {
        bail!("HTTP {} fetching {url}", resp.status());
    }

    let etag = header_string(&resp, "etag");
    let last_modified = header_string(&resp, "last-modified");

    let body = resp.into_body().bytes().await?;
    to_parsed_feed(body.as_ref(), etag, last_modified).map(Fetched::Modified)
}

/// Split out of [`fetch`] so the field mapping — which is what breaks first
/// when a real-world feed is shaped unexpectedly — is testable without a
/// network round trip.
fn to_parsed_feed(
    body: &[u8],
    etag: Option<String>,
    last_modified: Option<String>,
) -> Result<ParsedFeed> {
    let feed = parser::parse(body)?;

    let entries = feed
        .entries
        .iter()
        .filter_map(|entry| {
            let url = entry.links.first()?.href.clone();
            Some(ParsedEntry {
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
        .collect();

    Ok(ParsedFeed {
        title: feed.title.as_ref().map(|t| t.content.clone()),
        site_url: feed.links.first().map(|l| l.href.clone()),
        etag,
        last_modified,
        entries,
    })
}

/// Writes a parsed feed to `feed_id`. Articles already present are left alone
/// (`ON CONFLICT DO NOTHING`), so a re-fetch neither duplicates rows nor
/// resurrects read state.
pub async fn store(conn: &Connection, feed_id: &str, feed: &ParsedFeed) -> Result<()> {
    for entry in &feed.entries {
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
            feed.title.clone().into(),
            feed.site_url.clone().into(),
            feed.etag.clone().into(),
            feed.last_modified.clone().into(),
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

#[cfg(test)]
mod tests {
    use super::{to_parsed_feed, validate_url};

    #[test]
    fn maps_an_rss_feed_and_its_items() {
        let xml = br#"<rss version="2.0"><channel>
            <title>Example</title>
            <link>https://example.com</link>
            <item>
                <title>First</title>
                <link>https://example.com/1</link>
                <description>summary text</description>
                <author>alice@example.com</author>
                <pubDate>Wed, 02 Oct 2024 13:00:00 GMT</pubDate>
            </item>
            <item>
                <link>https://example.com/2</link>
            </item>
            <item>
                <title>no link, so not addressable</title>
            </item>
        </channel></rss>"#;

        let feed = to_parsed_feed(xml, Some("etag-1".into()), None).unwrap();

        assert_eq!(feed.title.as_deref(), Some("Example"));
        // feed-rs normalizes link URLs, so the trailing slash appears here.
        assert_eq!(feed.site_url.as_deref(), Some("https://example.com/"));
        assert_eq!(feed.etag.as_deref(), Some("etag-1"));
        assert_eq!(feed.last_modified, None);

        // The entry without a link is dropped: an article is only addressable
        // by its URL, and the articles table is keyed on it.
        assert_eq!(feed.entries.len(), 2);

        assert_eq!(feed.entries[0].url, "https://example.com/1");
        assert_eq!(feed.entries[0].title, "First");
        // RSS has no content, so the description carries the body.
        assert_eq!(feed.entries[0].content.as_deref(), Some("summary text"));
        assert!(
            feed.entries[0].author.is_some(),
            "an RSS <author> should populate author"
        );
        assert!(
            feed.entries[0].published_at.is_some(),
            "pubDate should populate published_at"
        );

        assert_eq!(feed.entries[1].url, "https://example.com/2");
        assert_eq!(feed.entries[1].title, "(no title)");
    }

    #[test]
    fn maps_an_atom_feed_preferring_content_over_summary() {
        let xml = br#"<?xml version="1.0" encoding="utf-8"?>
        <feed xmlns="http://www.w3.org/2005/Atom">
            <title>Atom Example</title>
            <link href="https://atom.example/"/>
            <updated>2024-10-02T13:00:00Z</updated>
            <entry>
                <title>Post</title>
                <link href="https://atom.example/post"/>
                <updated>2024-10-02T13:00:00Z</updated>
                <summary>the summary</summary>
                <content>the full body</content>
                <author><name>Bob</name></author>
            </entry>
        </feed>"#;

        let feed = to_parsed_feed(xml, None, None).unwrap();

        assert_eq!(feed.title.as_deref(), Some("Atom Example"));
        assert_eq!(feed.site_url.as_deref(), Some("https://atom.example/"));
        assert_eq!(feed.entries.len(), 1);

        let entry = &feed.entries[0];
        assert_eq!(entry.url, "https://atom.example/post");
        assert_eq!(entry.content.as_deref(), Some("the full body"));
        assert_eq!(entry.author.as_deref(), Some("Bob"));
        // No <published>, so <updated> stands in.
        assert_eq!(
            entry.published_at.as_deref(),
            Some("2024-10-02T13:00:00+00:00")
        );
    }

    #[test]
    fn errors_on_a_body_that_is_not_a_feed() {
        assert!(to_parsed_feed(b"<html><body>404</body></html>", None, None).is_err());
    }

    #[test]
    fn accepts_https_with_a_host() {
        assert!(validate_url("https://example.com/feed.xml").is_ok());
        assert!(validate_url("https://example.com").is_ok());
        assert!(validate_url("https://example.com:8443/feed?x=1").is_ok());
        assert!(validate_url("HTTPS://example.com/feed").is_ok());
    }

    #[test]
    fn rejects_plaintext_and_other_schemes() {
        assert!(validate_url("http://example.com/feed").is_err());
        assert!(validate_url("ftp://example.com/feed").is_err());
        assert!(validate_url("file:///etc/passwd").is_err());
    }

    #[test]
    fn rejects_urls_without_a_host() {
        assert!(validate_url("").is_err());
        assert!(validate_url("https://").is_err());
        assert!(validate_url("https:///feed.xml").is_err());
        assert!(validate_url("example.com/feed.xml").is_err());
    }

    #[test]
    fn does_not_split_a_multibyte_scheme_prefix() {
        // Slicing `url[..8]` on a byte index would panic here; `get` must be used.
        assert!(validate_url("フィード.xml").is_err());
    }
}
