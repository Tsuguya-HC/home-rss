use anyhow::Result;
use home_rss_shared::db;
use home_rss_shared::feed::parse_feed_bytes;
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

/// POST /api/feeds の追加直後取得の結果 → HTTP 応答への写像 (#106)。
/// 取得の失敗（到達不能・パース不能）は追加操作の結果としてユーザーに伝え、
/// DB insert 後の成功パスのみ 201 を使う。
#[derive(Debug)]
enum ImmediateFetchOutcome {
    /// その場で取得でき、記事が入った（通常の 201 応答）。
    /// 応答に含めるのは取得後の最新行（RETURNING の結果）。
    Fetched(Feed),
    /// 到達不能など、記事を取得できなかった（ユーザーに失敗として伝える）
    FetchFailed,
    /// 本文がフィードとしてパース不能（ユーザーに失敗として伝える）
    Unparseable,
}

fn immediate_fetch_response(outcome: &ImmediateFetchOutcome) -> Resp {
    match outcome {
        ImmediateFetchOutcome::Fetched(feed) => match serde_json::to_string(feed) {
            Ok(body) => json(StatusCode::CREATED, body),
            Err(_) => error_response(
                StatusCode::INTERNAL_SERVER_ERROR,
                "failed to serialize feed",
            ),
        },
        ImmediateFetchOutcome::FetchFailed => {
            error_response(StatusCode::BAD_GATEWAY, "failed to fetch feed")
        }
        ImmediateFetchOutcome::Unparseable => {
            error_response(StatusCode::UNPROCESSABLE_ENTITY, "feed could not be parsed")
        }
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

    match rows.first() {
        Some(row) => {
            let feed = row_to_feed(row)?;
            let conn = db::connect().await?;
            let outcome = immediate_fetch(&conn, &feed).await;
            Ok(immediate_fetch_response(&outcome))
        }
        None => Ok(error_response(
            StatusCode::INTERNAL_SERVER_ERROR,
            "failed to insert feed",
        )),
    }
}

/// フィード追加直後の即時取得 (#106)。`process_feed` 相当を server 側で実行し、
/// 結果を呼び出し側がユーザーへ通知できる形で返す。失敗しても追加自体は残す。
async fn immediate_fetch(conn: &spin_sdk::pg::Connection, feed: &Feed) -> ImmediateFetchOutcome {
    use spin_sdk::http::{EmptyBody, Request, Response, send};

    // ハング対策 (#106): 応答を返さないホストへの send はタイムアウト無しに待ち続ける。
    // WASI の outbound HTTP にタイムアウト API が無いため、一定時間で打ち切って
    // FetchFailed として返す。
    const FETCH_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(15);

    async fn with_timeout<T>(
        duration: std::time::Duration,
        fut: impl std::future::Future<Output = T>,
    ) -> Option<T> {
        use futures_util::FutureExt;
        futures_util::select! {
            result = fut.fuse() => Some(result),
            _ = spin_sdk::time::sleep(duration).fuse() => None,
        }
    }

    let req = match Request::get(&feed.url)
        .header("user-agent", "home-rss-fetcher/0.1")
        .body(EmptyBody::new())
    {
        Ok(r) => r,
        Err(e) => {
            eprintln!(
                "immediate_fetch {}: failed to build request: {e:#}",
                feed.url
            );
            return ImmediateFetchOutcome::FetchFailed;
        }
    };
    let resp: Response = match with_timeout(FETCH_TIMEOUT, send(req)).await {
        Some(Ok(r)) => r,
        Some(Err(e)) => {
            eprintln!("immediate_fetch {}: request failed: {e:#}", feed.url);
            return ImmediateFetchOutcome::FetchFailed;
        }
        None => {
            eprintln!("immediate_fetch {}: request timed out", feed.url);
            return ImmediateFetchOutcome::FetchFailed;
        }
    };
    if resp.status() != StatusCode::OK {
        eprintln!(
            "immediate_fetch {}: unexpected HTTP status {}",
            feed.url,
            resp.status()
        );
        return ImmediateFetchOutcome::FetchFailed;
    }
    let body = match with_timeout(FETCH_TIMEOUT, resp.into_body().bytes()).await {
        Some(Ok(b)) => b,
        Some(Err(e)) => {
            eprintln!("immediate_fetch {}: failed to read body: {e:#}", feed.url);
            return ImmediateFetchOutcome::FetchFailed;
        }
        None => {
            eprintln!("immediate_fetch {}: timed out reading body", feed.url);
            return ImmediateFetchOutcome::FetchFailed;
        }
    };
    let parsed = match parse_feed_bytes(body.as_ref()) {
        Ok(f) => f,
        Err(e) => {
            eprintln!("immediate_fetch {}: unparseable feed: {e:#}", feed.url);
            return ImmediateFetchOutcome::Unparseable;
        }
    };

    let feed_title = parsed.title.clone();
    let site_url = parsed.site_url.clone();

    for entry in &parsed.entries {
        if let Err(e) = conn
            .execute(
                "INSERT INTO articles (feed_id, url, title, content, author, published_at) \
                 VALUES ($1, $2, $3, $4, $5, $6::text::timestamptz) ON CONFLICT DO NOTHING",
                vec![
                    ParameterValue::Uuid(feed.id.clone()),
                    entry.url.clone().into(),
                    entry.title.clone().into(),
                    entry.content.clone().into(),
                    entry.author.clone().into(),
                    entry.published_at.clone().into(),
                ],
            )
            .await
        {
            eprintln!(
                "immediate_fetch {}: failed to insert article: {e:#}",
                feed.url
            );
            return ImmediateFetchOutcome::FetchFailed;
        }
    }

    match conn
        .query(
            "UPDATE feeds SET title = $1, site_url = $2, last_fetched_at = NOW() WHERE id = $3 \
             RETURNING id::text, url, title, site_url, etag, last_modified, \
             EXTRACT(EPOCH FROM last_fetched_at)::bigint, \
             EXTRACT(EPOCH FROM created_at)::bigint",
            vec![
                feed_title.into(),
                site_url.into(),
                ParameterValue::Uuid(feed.id.clone()),
            ],
        )
        .await
    {
        Ok(rows) => match rows.collect().await {
            Ok(rows) => match rows.first() {
                Some(row) => match row_to_feed(row) {
                    Ok(updated) => ImmediateFetchOutcome::Fetched(updated),
                    Err(e) => {
                        eprintln!(
                            "immediate_fetch {}: failed to decode updated feed: {e:#}",
                            feed.url
                        );
                        ImmediateFetchOutcome::FetchFailed
                    }
                },
                None => {
                    eprintln!(
                        "immediate_fetch {}: UPDATE RETURNING returned no rows",
                        feed.url
                    );
                    ImmediateFetchOutcome::FetchFailed
                }
            },
            Err(e) => {
                eprintln!(
                    "immediate_fetch {}: failed to collect updated feed: {e:#}",
                    feed.url
                );
                ImmediateFetchOutcome::FetchFailed
            }
        },
        Err(e) => {
            eprintln!("immediate_fetch {}: failed to update feed: {e:#}", feed.url);
            ImmediateFetchOutcome::FetchFailed
        }
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

    fn test_feed() -> home_rss_shared::models::Feed {
        home_rss_shared::models::Feed {
            id: "00000000-0000-0000-0000-000000000000".to_owned(),
            url: "https://example.com/feed".to_owned(),
            title: None,
            site_url: None,
            etag: None,
            last_modified: None,
            last_fetched_at: None,
            created_at: None,
        }
    }

    #[test]
    fn fetched_feed_returns_created() {
        use super::{ImmediateFetchOutcome, immediate_fetch_response};
        let resp = immediate_fetch_response(&ImmediateFetchOutcome::Fetched(test_feed()));
        assert_eq!(resp.status(), spin_sdk::http::StatusCode::CREATED);
    }

    #[test]
    fn fetched_response_body_reflects_updated_feed() {
        use super::{ImmediateFetchOutcome, immediate_fetch_response};
        let mut updated = test_feed();
        updated.title = Some("Example Feed".to_owned());
        updated.site_url = Some("https://example.com/".to_owned());
        updated.last_fetched_at = Some(1_757_894_400);
        let resp = immediate_fetch_response(&ImmediateFetchOutcome::Fetched(updated));
        let body = resp
            .into_body()
            .into_inner()
            .expect("immediate_fetch_response body");
        let feed: home_rss_shared::models::Feed = serde_json::from_slice(body.as_ref()).unwrap();
        // Fetched が運ぶのは取得後の最新行。更新前の None のまま返してはならない。
        assert_eq!(feed.title.as_deref(), Some("Example Feed"));
        assert_eq!(feed.site_url.as_deref(), Some("https://example.com/"));
        assert_eq!(feed.last_fetched_at, Some(1_757_894_400));
    }

    #[test]
    fn fetch_failure_is_surfaced_not_created() {
        use super::{ImmediateFetchOutcome, immediate_fetch_response};
        let resp = immediate_fetch_response(&ImmediateFetchOutcome::FetchFailed);
        assert_ne!(resp.status(), spin_sdk::http::StatusCode::CREATED);
    }

    #[test]
    fn unparseable_feed_is_surfaced_not_created() {
        use super::{ImmediateFetchOutcome, immediate_fetch_response};
        let resp = immediate_fetch_response(&ImmediateFetchOutcome::Unparseable);
        assert_ne!(resp.status(), spin_sdk::http::StatusCode::CREATED);
    }
}
