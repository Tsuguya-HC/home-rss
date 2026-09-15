use anyhow::Result;
use feed_rs::parser;

/// POST /api/feeds の即時取得と定期取得で共有する純粋なパース (#106)。
/// HTTP 取得・DB 書き込みは含まず、本文バイトから記事レコードの材料だけを取り出す。
/// パース不能は Err として返し、呼び出し側がユーザーへの通知に使う。
#[derive(Debug, PartialEq)]
pub struct ParsedEntry {
    pub url: String,
    pub title: String,
    pub content: Option<String>,
    pub author: Option<String>,
    /// RFC 3339。entry.published が無ければ entry.updated を使う。
    pub published_at: Option<String>,
}

#[derive(Debug, PartialEq)]
pub struct ParsedFeed {
    pub title: Option<String>,
    pub site_url: Option<String>,
    pub entries: Vec<ParsedEntry>,
}

pub fn parse_feed_bytes(body: &[u8]) -> Result<ParsedFeed> {
    let feed = parser::parse(body)?;
    Ok(ParsedFeed {
        title: feed.title.as_ref().map(|t| t.content.clone()),
        site_url: feed.links.first().map(|l| l.href.clone()),
        entries: feed
            .entries
            .iter()
            .filter_map(|entry| {
                // #109: rel="alternate"（記事 URL）を rel="self"（フィード URL）より優先する。
                // feed-rs は文書順を保持し、rel 省略時は "alternate" を補うため、
                // alternate が無ければ従来通り先頭を使う。
                let entry_url = entry
                    .links
                    .iter()
                    .find(|l| l.rel.as_deref() == Some("alternate"))
                    .or(entry.links.first())?
                    .href
                    .clone();
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
        assert_eq!(feed.entries[0].content.as_deref(), Some("world"));
        assert_eq!(
            feed.entries[0].published_at.as_deref(),
            Some("2026-09-15T00:00:00+00:00")
        );
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
    fn prefers_alternate_link_over_self_link() {
        // #109: feeds that list a <link rel="self"> (feed URL) before the
        // <link rel="alternate"> (article URL) must store the article URL,
        // so the reader can link to the original page. feed-rs preserves
        // document order, so taking links.first() stores the self URL.
        let xml = br#"<?xml version="1.0" encoding="utf-8"?>
<feed xmlns="http://www.w3.org/2005/Atom">
<title>Example</title>
<entry>
<title>An article</title>
<link rel="self" href="https://example.com/feed.xml"/>
<link rel="alternate" href="https://example.com/article"/>
</entry>
</feed>"#;
        let feed = parse_feed_bytes(xml).unwrap();
        assert_eq!(feed.entries.len(), 1);
        assert_eq!(feed.entries[0].url, "https://example.com/article");
    }

    #[test]
    fn unparseable_body_is_an_error() {
        assert!(parse_feed_bytes(b"this is not a feed").is_err());
    }
}
