use anyhow::Result;
use home_rss_shared::db;
use home_rss_shared::http::{Resp, empty, json};
use home_rss_shared::models::{Article, CreateFeedRequest, Feed};
use quick_xml::Reader;
use quick_xml::events::Event;
use spin_sdk::http::body::IncomingBodyExt;
use spin_sdk::http::{Method, Request, StatusCode};
use spin_sdk::http_service;
use spin_sdk::pg::{Decode, ParameterValue, Row};

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

    match rows.first() {
        Some(row) => {
            let feed = row_to_feed(row)?;
            Ok(json(StatusCode::CREATED, serde_json::to_string(&feed)?))
        }
        None => Ok(error_response(
            StatusCode::INTERNAL_SERVER_ERROR,
            "failed to insert feed",
        )),
    }
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
    use super::parse_opml;

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
