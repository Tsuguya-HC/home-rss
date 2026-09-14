use spin_sdk::http::body::IncomingBodyExt;
use spin_sdk::http::{EmptyBody, Request, Response, StatusCode, send};
use spin_sdk::pg::{Connection, ParameterValue};

#[derive(Debug, PartialEq, Eq)]
pub enum FetchOutcome {
    Fetched { articles: u64 },
    NotModified,
}

#[derive(Debug)]
pub enum FetchError {
    Http(String),
    Parse(String),
}

impl std::fmt::Display for FetchError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            FetchError::Http(msg) => {
                write!(f, "feed is unreachable or returned an error: {msg}")
            }
            FetchError::Parse(msg) => write!(f, "feed could not be parsed: {msg}"),
        }
    }
}

impl std::error::Error for FetchError {}

pub async fn fetch_and_store(
    conn: &Connection,
    feed_id: &str,
    url: &str,
    etag: Option<&str>,
    last_modified: Option<&str>,
) -> std::result::Result<FetchOutcome, FetchError> {
    let body = fetch_feed(url, etag, last_modified).await?;
    let Some(body) = body else {
        return Ok(FetchOutcome::NotModified);
    };

    let feed =
        feed_rs::parser::parse(body.as_ref()).map_err(|e| FetchError::Parse(e.to_string()))?;

    let feed_title = feed.title.as_ref().map(|t| t.content.clone());
    let site_url = feed.links.first().map(|l| l.href.clone());

    let mut articles = 0u64;
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
        let published_at: Option<String> =
            entry.published.or(entry.updated).map(|dt| dt.to_rfc3339());

        let inserted = conn
            .execute(
                "INSERT INTO articles (feed_id, url, title, content, author, published_at) \
                 VALUES ($1, $2, $3, $4, $5, $6::text::timestamptz) ON CONFLICT DO NOTHING",
                vec![
                    ParameterValue::Uuid(feed_id.to_owned()),
                    entry_url.to_owned().into(),
                    entry_title.to_owned().into(),
                    content.map(str::to_owned).into(),
                    author.map(str::to_owned).into(),
                    published_at.into(),
                ],
            )
            .await
            .map_err(|e| FetchError::Http(format!("{e:#}")))?;
        articles += inserted;
    }

    conn.execute(
        "UPDATE feeds SET title = $1, site_url = $2, etag = $3, last_modified = $4, \
         last_fetched_at = NOW() WHERE id = $5",
        vec![
            feed_title.into(),
            site_url.into(),
            body.etag.into(),
            body.last_modified.into(),
            ParameterValue::Uuid(feed_id.to_owned()),
        ],
    )
    .await
    .map_err(|e| FetchError::Http(format!("{e:#}")))?;

    Ok(FetchOutcome::Fetched { articles })
}

struct FetchedBody {
    bytes: Vec<u8>,
    etag: Option<String>,
    last_modified: Option<String>,
}

impl AsRef<[u8]> for FetchedBody {
    fn as_ref(&self) -> &[u8] {
        &self.bytes
    }
}

async fn fetch_feed(
    url: &str,
    etag: Option<&str>,
    last_modified: Option<&str>,
) -> std::result::Result<Option<FetchedBody>, FetchError> {
    require_https(url)?;

    let mut builder = Request::get(url).header("user-agent", "home-rss-fetcher/0.1");
    if let Some(etag) = etag {
        builder = builder.header("if-none-match", etag);
    }
    if let Some(lm) = last_modified {
        builder = builder.header("if-modified-since", lm);
    }
    let req = builder
        .body(EmptyBody::new())
        .map_err(|e| FetchError::Http(e.to_string()))?;

    let resp: Response = send(req)
        .await
        .map_err(|e| FetchError::Http(e.to_string()))?;

    if resp.status() == StatusCode::NOT_MODIFIED {
        return Ok(None);
    }
    if resp.status() != StatusCode::OK {
        return Err(FetchError::Http(format!(
            "HTTP {} fetching {url}",
            resp.status()
        )));
    }

    let etag = header_string(&resp, "etag");
    let last_modified = header_string(&resp, "last-modified");

    let bytes = resp
        .into_body()
        .bytes()
        .await
        .map_err(|e| FetchError::Http(e.to_string()))?;

    Ok(Some(FetchedBody {
        bytes: bytes.to_vec(),
        etag,
        last_modified,
    }))
}

pub fn require_https(url: &str) -> std::result::Result<(), FetchError> {
    let lower = url.trim_start().to_ascii_lowercase();
    if lower.starts_with("https://") {
        Ok(())
    } else {
        Err(FetchError::Http(format!(
            "only https feed URLs are supported: {url}"
        )))
    }
}

fn header_string(resp: &Response, name: &str) -> Option<String> {
    resp.headers()
        .get(name)
        .and_then(|v| v.to_str().ok())
        .map(str::to_owned)
}

#[cfg(test)]
mod tests {
    use super::require_https;

    #[test]
    fn accepts_https_urls() {
        assert!(require_https("https://example.com/feed.xml").is_ok());
        assert!(require_https("HTTPS://example.com/feed.xml").is_ok());
    }

    #[test]
    fn rejects_non_https_urls() {
        assert!(require_https("http://example.com/feed.xml").is_err());
        assert!(require_https("ftp://example.com/feed.xml").is_err());
        assert!(require_https("example.com/feed.xml").is_err());
        assert!(require_https("").is_err());
    }
}
