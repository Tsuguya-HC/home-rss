use anyhow::Result;
use feed_rs::parser;
use home_rss_shared::db;
use home_rss_shared::http::{Resp, empty, json};
use home_rss_shared::models::{Article, CreateFeedRequest, Feed};
use quick_xml::Reader;
use quick_xml::events::Event;
use spin_sdk::http::body::IncomingBodyExt;
use spin_sdk::http::{EmptyBody, Method, Request, Response, StatusCode, send};
use spin_sdk::http_service;
use spin_sdk::pg::{Connection, Decode, ParameterValue, Row};

#[http_service]
async fn handle(req: Request) -> Resp {
    match route(req).await {
        Ok(resp) => resp,
        Err(e) => {
            eprintln!("home-rss-server: {e:#}");
            error_response(StatusCode::INTERNAL_SERVER_ERROR, "internal server error")
        }
    }
}

async fn route(req: Request) -> Result<Resp> {
    let method = req.method().clone();
    let path = req.uri().path().to_owned();
    let query = req.uri().query().unwrap_or_default().to_owned();
    let segments: Vec<&str> = path.trim_matches('/').split('/').collect();

    match (&method, segments.as_slice()) {
        (&Method::GET, ["api", "feeds"]) => list_feeds().await,
        (&Method::POST, ["api", "feeds"]) => add_feed(req).await,
        (&Method::DELETE, ["api", "feeds", id]) => delete_feed(id).await,
        (&Method::GET, ["api", "articles"]) => list_articles(&query).await,
        (&Method::POST, ["api", "articles", "read-all"]) => mark_all_read().await,
        (&Method::POST, ["api", "articles", id, "read"]) => mark_read(id).await,
        (&Method::POST, ["api", "import", "opml"]) => import_opml(req).await,
        (&Method::GET, ["api", "stats"]) => get_stats().await,
        _ => Ok(error_response(StatusCode::NOT_FOUND, "not found")),
    }
}

fn json_ok(body: impl Into<String>) -> Result<Resp> {
    Ok(json(StatusCode::OK, body.into()))
}

fn error_response(status: StatusCode, message: &str) -> Resp {
    let body = serde_json::to_string(message).unwrap_or_else(|_| "\"error\"".to_owned());
    json(status, format!(r#"{{"error":{body}}}"#))
}

/// Outcome of an immediate (synchronous) fetch attempt for a single feed.
///
/// Pure decision logic for issue #106: given the fetch result, what must
/// `POST /api/feeds` report back? Mapping to HTTP happens in
/// [`add_feed_status`]. Kept separate from I/O so it is unit-testable
/// without DB or network.
#[derive(Debug, PartialEq, Eq)]
enum ImmediateFetchOutcome {
    /// Feed fetched and parsed; articles stored.
    Fetched,
    /// Transport-level or HTTP-error failure (unreachable, non-2xx, timeout).
    Unreachable,
    /// Bytes arrived but no usable entries (parse failure or empty feed).
    Unparseable,
}

/// Maps an immediate-fetch outcome to the `POST /api/feeds` response status.
/// Per issue #106 a fetch failure must reach the user as the result of the
/// add operation: success stays 201 Created, any fetch failure is an error.
/// An unreachable upstream is a gateway problem (502); bytes that arrive but
/// cannot be turned into entries are a problem with the given feed (422).
fn add_feed_status(outcome: &ImmediateFetchOutcome) -> StatusCode {
    match outcome {
        ImmediateFetchOutcome::Fetched => StatusCode::CREATED,
        ImmediateFetchOutcome::Unreachable => StatusCode::BAD_GATEWAY,
        ImmediateFetchOutcome::Unparseable => StatusCode::UNPROCESSABLE_ENTITY,
    }
}

fn parse_query(query: &str) -> std::collections::HashMap<String, String> {
    query
        .split('&')
        .filter_map(|pair| {
            let mut kv = pair.splitn(2, '=');
            let k = kv.next()?.to_string();
            let v = kv.next().unwrap_or("").to_string();
            if k.is_empty() { None } else { Some((k, v)) }
        })
        .collect()
}

const FEED_SELECT: &str = "SELECT id::text, url, title, site_url, etag, last_modified, \
     EXTRACT(EPOCH FROM last_fetched_at)::bigint, \
     EXTRACT(EPOCH FROM created_at)::bigint \
     FROM feeds";

const ARTICLE_SELECT: &str = "SELECT a.id::text, a.feed_id::text, a.url, a.title, a.content, a.author, \
     EXTRACT(EPOCH FROM a.published_at)::bigint, \
     EXTRACT(EPOCH FROM a.fetched_at)::bigint \
     FROM articles a";

fn row_to_feed(row: &Row) -> Result<Feed> {
    Ok(Feed {
        id: String::decode(&row[0])?,
        url: String::decode(&row[1])?,
        title: Option::<String>::decode(&row[2])?,
        site_url: Option::<String>::decode(&row[3])?,
        etag: Option::<String>::decode(&row[4])?,
        last_modified: Option::<String>::decode(&row[5])?,
        last_fetched_at: Option::<i64>::decode(&row[6])?,
        created_at: Option::<i64>::decode(&row[7])?,
    })
}

fn row_to_article(row: &Row) -> Result<Article> {
    Ok(Article {
        id: String::decode(&row[0])?,
        feed_id: String::decode(&row[1])?,
        url: String::decode(&row[2])?,
        title: String::decode(&row[3])?,
        content: Option::<String>::decode(&row[4])?,
        author: Option::<String>::decode(&row[5])?,
        published_at: Option::<i64>::decode(&row[6])?,
        fetched_at: Option::<i64>::decode(&row[7])?,
    })
}

async fn list_feeds() -> Result<Resp> {
    let conn = db::connect().await?;
    let rows = conn
        .query(format!("{FEED_SELECT} ORDER BY created_at DESC"), vec![])
        .await?
        .collect()
        .await?;
    let feeds: Vec<Feed> = rows.iter().map(row_to_feed).collect::<Result<_>>()?;
    json_ok(serde_json::to_string(&feeds)?)
}

async fn add_feed(req: Request) -> Result<Resp> {
    let body = req.into_body().bytes().await?;
    let create_req: CreateFeedRequest = match serde_json::from_slice(body.as_ref()) {
        Ok(r) => r,
        Err(_) => return Ok(error_response(StatusCode::BAD_REQUEST, "invalid JSON body")),
    };

    let conn = db::connect().await?;
    let rows = conn
        .query(
            "INSERT INTO feeds (url) VALUES ($1) \
             ON CONFLICT (url) DO UPDATE SET url = EXCLUDED.url \
             RETURNING id::text, url, title, site_url, etag, last_modified, \
             EXTRACT(EPOCH FROM last_fetched_at)::bigint, \
             EXTRACT(EPOCH FROM created_at)::bigint",
            vec![ParameterValue::Str(create_req.url)],
        )
        .await?
        .collect()
        .await?;

    let (feed_id, feed_url) = match rows.first() {
        Some(row) => {
            let feed = row_to_feed(row)?;
            (feed.id.clone(), feed.url.clone())
        }
        None => {
            return Ok(error_response(
                StatusCode::INTERNAL_SERVER_ERROR,
                "failed to insert feed",
            ));
        }
    };

    // Issue #106: fetch the new feed right away so articles are visible
    // immediately instead of only after the next fetcher CronJob run.
    // A fetch failure is reported as the result of this add operation;
    // the feed row stays so the periodic fetcher can retry it later.
    // Transport/parse problems are classified inside immediate_fetch;
    // only genuine internal (DB) errors propagate as Err here (-> 500).
    let outcome = immediate_fetch(&conn, &feed_id, &feed_url).await?;

    let rows = conn
        .query(
            "SELECT id::text, url, title, site_url, etag, last_modified, \
             EXTRACT(EPOCH FROM last_fetched_at)::bigint, \
             EXTRACT(EPOCH FROM created_at)::bigint \
             FROM feeds WHERE id = $1",
            vec![ParameterValue::Uuid(feed_id)],
        )
        .await?
        .collect()
        .await?;
    let feed = match rows.first() {
        // Feed metadata (title, site_url) may have been filled in by the fetch.
        Some(row) => row_to_feed(row)?,
        None => {
            return Ok(error_response(
                StatusCode::INTERNAL_SERVER_ERROR,
                "failed to read back feed",
            ));
        }
    };

    let status = add_feed_status(&outcome);
    if outcome == ImmediateFetchOutcome::Fetched {
        Ok(json(status, serde_json::to_string(&feed)?))
    } else {
        Ok(json(
            status,
            serde_json::to_string(&serde_json::json!({
                "feed": feed,
                "error": match outcome {
                    ImmediateFetchOutcome::Unreachable =>
                        "failed to fetch feed: unreachable or HTTP error",
                    ImmediateFetchOutcome::Unparseable =>
                        "failed to fetch feed: could not parse any entries",
                    ImmediateFetchOutcome::Fetched => unreachable!(),
                },
            }))?,
        ))
    }
}

/// Fetches a single feed immediately and stores its articles.
///
/// Same semantics as the periodic fetcher (`fetcher/src/lib.rs`): conditional
/// GET, update feed metadata, insert entries with `ON CONFLICT DO NOTHING`.
/// Classifies the result as [`ImmediateFetchOutcome`] so `add_feed` can
/// report it. Transport problems and non-2xx statuses are `Unreachable`;
/// bytes that `feed-rs` cannot parse, or that parse but yield no usable
/// entries (and hence no visible articles), are `Unparseable`.
async fn immediate_fetch(
    conn: &Connection,
    feed_id: &str,
    url: &str,
) -> Result<ImmediateFetchOutcome> {
    let req = Request::get(url)
        .header("user-agent", "home-rss-server/0.1")
        .body(EmptyBody::new())?;
    let resp: Response = match send(req).await {
        Ok(resp) => resp,
        Err(_) => return Ok(ImmediateFetchOutcome::Unreachable),
    };

    if resp.status() == StatusCode::NOT_MODIFIED {
        return Ok(ImmediateFetchOutcome::Fetched);
    }
    if !resp.status().is_success() {
        return Ok(ImmediateFetchOutcome::Unreachable);
    }

    let new_etag = header_string(&resp, "etag");
    let new_last_modified = header_string(&resp, "last-modified");

    let body = resp.into_body().bytes().await?;
    let feed = match parser::parse(body.as_ref()) {
        Ok(feed) => feed,
        Err(_) => return Ok(ImmediateFetchOutcome::Unparseable),
    };

    let feed_title = feed.title.as_ref().map(|t| t.content.clone());
    let site_url = feed.links.first().map(|l| l.href.clone());

    let mut stored = 0u64;
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

        stored += conn
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
            .await?;
    }

    if feed.entries.is_empty() || stored == 0 && article_count(conn, feed_id).await? == 0 {
        // Parsed but nothing to show: either genuinely entry-less or every
        // entry was unusable (no link). Report it as unparseable so the user
        // knows the add did not produce articles.
        return Ok(ImmediateFetchOutcome::Unparseable);
    }

    conn.execute(
        "UPDATE feeds SET title = $1, site_url = $2, etag = $3, last_modified = $4, \
         last_fetched_at = NOW() WHERE id = $5",
        vec![
            feed_title.into(),
            site_url.into(),
            new_etag.into(),
            new_last_modified.into(),
            ParameterValue::Uuid(feed_id.to_owned()),
        ],
    )
    .await?;

    Ok(ImmediateFetchOutcome::Fetched)
}

async fn article_count(conn: &Connection, feed_id: &str) -> Result<u64> {
    let rows = conn
        .query(
            "SELECT COUNT(*)::bigint FROM articles WHERE feed_id = $1",
            vec![ParameterValue::Uuid(feed_id.to_owned())],
        )
        .await?
        .collect()
        .await?;
    match rows.first() {
        Some(row) => Ok(i64::decode(&row[0])? as u64),
        None => Ok(0),
    }
}

fn header_string(resp: &Response, name: &str) -> Option<String> {
    resp.headers()
        .get(name)
        .and_then(|v| v.to_str().ok())
        .map(str::to_owned)
}

async fn delete_feed(id: &str) -> Result<Resp> {
    let conn = db::connect().await?;
    let rows = conn
        .execute(
            "DELETE FROM feeds WHERE id = $1",
            vec![ParameterValue::Uuid(id.to_owned())],
        )
        .await?;

    if rows == 0 {
        Ok(error_response(StatusCode::NOT_FOUND, "feed not found"))
    } else {
        Ok(empty(StatusCode::NO_CONTENT))
    }
}

async fn list_articles(query: &str) -> Result<Resp> {
    let params_map = parse_query(query);
    let feed_id = params_map.get("feed_id").cloned();
    let unread = params_map
        .get("unread")
        .map(|s| s == "true")
        .unwrap_or(false);

    let conn = db::connect().await?;

    let (sql, query_params): (String, Vec<ParameterValue>) = match (feed_id, unread) {
        (Some(fid), true) => (
            format!(
                "{ARTICLE_SELECT} \
                 LEFT JOIN read_status rs ON a.id = rs.article_id \
                 WHERE a.feed_id = $1 AND rs.article_id IS NULL \
                 ORDER BY a.published_at DESC NULLS LAST"
            ),
            vec![ParameterValue::Uuid(fid)],
        ),
        (Some(fid), false) => (
            format!(
                "{ARTICLE_SELECT} \
                 WHERE a.feed_id = $1 \
                 ORDER BY a.published_at DESC NULLS LAST"
            ),
            vec![ParameterValue::Uuid(fid)],
        ),
        (None, true) => (
            format!(
                "{ARTICLE_SELECT} \
                 LEFT JOIN read_status rs ON a.id = rs.article_id \
                 WHERE rs.article_id IS NULL \
                 ORDER BY a.published_at DESC NULLS LAST"
            ),
            vec![],
        ),
        (None, false) => (
            format!("{ARTICLE_SELECT} ORDER BY a.published_at DESC NULLS LAST"),
            vec![],
        ),
    };

    let rows = conn.query(sql, query_params).await?.collect().await?;
    let articles: Vec<Article> = rows.iter().map(row_to_article).collect::<Result<_>>()?;
    json_ok(serde_json::to_string(&articles)?)
}

async fn mark_read(id: &str) -> Result<Resp> {
    let conn = db::connect().await?;
    conn.execute(
        "INSERT INTO read_status (article_id) VALUES ($1) ON CONFLICT DO NOTHING",
        vec![ParameterValue::Uuid(id.to_owned())],
    )
    .await?;

    Ok(empty(StatusCode::NO_CONTENT))
}

async fn mark_all_read() -> Result<Resp> {
    let conn = db::connect().await?;
    conn.execute(
        "INSERT INTO read_status (article_id) \
         SELECT id FROM articles a \
         WHERE NOT EXISTS (SELECT 1 FROM read_status rs WHERE rs.article_id = a.id) \
         ON CONFLICT DO NOTHING",
        vec![],
    )
    .await?;

    Ok(empty(StatusCode::NO_CONTENT))
}

async fn import_opml(req: Request) -> Result<Resp> {
    let body = req.into_body().bytes().await?;
    let urls = parse_opml(body.as_ref())?;

    if urls.is_empty() {
        return Ok(error_response(
            StatusCode::BAD_REQUEST,
            "no feeds found in OPML",
        ));
    }

    let conn = db::connect().await?;
    let mut imported = 0u64;
    for url in &urls {
        imported += conn
            .execute(
                "INSERT INTO feeds (url) VALUES ($1) ON CONFLICT (url) DO NOTHING",
                vec![ParameterValue::Str(url.clone())],
            )
            .await?;
    }

    json_ok(format!(r#"{{"imported":{imported}}}"#))
}

fn parse_opml(data: &[u8]) -> Result<Vec<String>> {
    let text = String::from_utf8_lossy(data);
    let mut reader = Reader::from_str(&text);
    let mut urls = Vec::new();

    loop {
        match reader.read_event() {
            Ok(Event::Empty(ref e)) | Ok(Event::Start(ref e)) => {
                if e.name().as_ref().eq_ignore_ascii_case("outline") {
                    for attr in e.attributes().flatten() {
                        if attr.key.as_ref().eq_ignore_ascii_case("xmlUrl") {
                            let url = attr.value.trim().to_string();
                            if !url.is_empty() && !url.contains('\u{FFFD}') {
                                urls.push(url);
                            }
                        }
                    }
                }
            }
            Ok(Event::Eof) => break,
            Err(e) => return Err(anyhow::anyhow!("OPML parse error: {e}")),
            _ => {}
        }
    }

    Ok(urls)
}

async fn get_stats() -> Result<Resp> {
    let conn = db::connect().await?;
    let rows = conn
        .query(
            "SELECT \
             (SELECT COUNT(*)::bigint FROM feeds) AS feeds, \
             (SELECT COUNT(*)::bigint FROM articles a \
              WHERE NOT EXISTS (SELECT 1 FROM read_status rs WHERE rs.article_id = a.id)) AS unread",
            vec![],
        )
        .await?
        .collect()
        .await?;

    let (feeds, unread) = match rows.first() {
        Some(row) => (i64::decode(&row[0])?, i64::decode(&row[1])?),
        None => (0, 0),
    };

    json_ok(format!(r#"{{"feeds":{feeds},"unread":{unread}}}"#))
}

#[cfg(test)]
mod tests {
    use super::{ImmediateFetchOutcome, add_feed_status, parse_opml};
    use spin_sdk::http::StatusCode;

    #[test]
    fn fetch_success_keeps_created() {
        assert_eq!(
            add_feed_status(&ImmediateFetchOutcome::Fetched),
            StatusCode::CREATED
        );
    }

    #[test]
    fn unreachable_feed_is_reported_as_error() {
        let status = add_feed_status(&ImmediateFetchOutcome::Unreachable);
        assert!(
            status.is_client_error() || status.is_server_error(),
            "unreachable fetch must be an error status, got {status}"
        );
    }

    #[test]
    fn unparseable_feed_is_reported_as_error() {
        let status = add_feed_status(&ImmediateFetchOutcome::Unparseable);
        assert!(
            status.is_client_error() || status.is_server_error(),
            "unparseable fetch must be an error status, got {status}"
        );
    }

    #[test]
    fn extracts_urls_from_nested_and_self_closing_outlines() {
        let xml = br#"<opml><body>
            <outline text="A" xmlUrl="http://a.example/feed" />
            <outline text="Folder">
                <outline text="B" xmlUrl="http://b.example/feed"></outline>
            </outline>
        </body></opml>"#;
        let urls = parse_opml(xml).unwrap();
        assert_eq!(urls, vec!["http://a.example/feed", "http://b.example/feed"]);
    }

    #[test]
    fn ignores_missing_empty_and_whitespace_only_xml_url() {
        let xml = br#"<opml><body>
            <outline text="no url" />
            <outline text="empty" xmlUrl="" />
            <outline text="whitespace" xmlUrl="   " />
            <outline text="ok" xmlUrl="http://ok.example/feed" />
        </body></opml>"#;
        let urls = parse_opml(xml).unwrap();
        assert_eq!(urls, vec!["http://ok.example/feed"]);
    }

    #[test]
    fn recovers_urls_when_only_text_attribute_has_invalid_utf8() {
        let mut xml = Vec::new();
        xml.extend_from_slice(br#"<opml><body><outline text=""#);
        xml.push(0xFF);
        xml.extend_from_slice(br#"" xmlUrl="http://broken-text.example/feed" />"#);
        xml.extend_from_slice(
            br#"<outline text="ok" xmlUrl="http://ok.example/feed" /></body></opml>"#,
        );
        let urls = parse_opml(&xml).unwrap();
        assert_eq!(
            urls,
            vec!["http://broken-text.example/feed", "http://ok.example/feed"]
        );
    }

    #[test]
    fn skips_entry_when_xml_url_itself_has_invalid_utf8() {
        let mut xml = Vec::new();
        xml.extend_from_slice(br#"<opml><body><outline text="bad" xmlUrl="http://broken"#);
        xml.push(0xFF);
        xml.extend_from_slice(br#".example/feed" />"#);
        xml.extend_from_slice(
            br#"<outline text="ok" xmlUrl="http://ok.example/feed" /></body></opml>"#,
        );
        let urls = parse_opml(&xml).unwrap();
        assert_eq!(urls, vec!["http://ok.example/feed"]);
    }

    #[test]
    fn errors_on_malformed_xml() {
        let xml = br#"<opml><body><outline xmlUrl="http://a.example/feed"></body></opml>"#;
        assert!(parse_opml(xml).is_err());
    }
}
