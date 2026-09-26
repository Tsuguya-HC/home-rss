use anyhow::Result;
use home_rss_shared::db;
use home_rss_shared::fetch::{FetchAndStoreOutcome, decode_feed_row, fetch_and_store};
use home_rss_shared::http::{Resp, empty, json};
use home_rss_shared::models::{Article, CreateFeedRequest, Feed};
use home_rss_shared::ssrf::reject_internal_feed_url;
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
        (&Method::POST, ["api", "articles", id, "favorite"]) => mark_favorite(id).await,
        (&Method::DELETE, ["api", "articles", id, "favorite"]) => unmark_favorite(id).await,
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
/// 取得の失敗（到達不能・パース不能）はフィード取得自体の失敗としてユーザーに伝え、
/// DB 書き込みの失敗はサーバ側の障害として区別して伝える。成功パスのみ 201 を使う。
#[derive(Debug)]
enum ImmediateFetchOutcome {
    /// その場で取得でき、記事が入った（通常の 201 応答）。
    /// 応答に含めるのは取得後の最新行（RETURNING の結果）。ただし
    /// FetchAndStoreOutcome::NotModified（追加直後は etag が無いため実際には
    /// 起こらないはずの防御的な分岐、#106 R10）の場合だけ、取得前の行が
    /// そのまま使われる。
    Fetched(Feed),
    /// 到達不能など、記事を取得できなかった（ユーザーに失敗として伝える）
    FetchFailed,
    /// 本文がフィードとしてパース不能（ユーザーに失敗として伝える）
    Unparseable,
    /// 取得には成功したが、DB への保存に失敗した。フィード取得自体は正常なので
    /// 502 ではなくサーバ側のエラーとして伝える。
    StoreFailed,
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
        ImmediateFetchOutcome::StoreFailed => error_response(
            StatusCode::INTERNAL_SERVER_ERROR,
            "feed was fetched but failed to save",
        ),
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
     EXTRACT(EPOCH FROM a.fetched_at)::bigint, a.image_url, \
     EXISTS (SELECT 1 FROM favorites f WHERE f.article_id = a.id) \
     FROM articles a";

fn row_to_feed(row: &Row) -> Result<Feed> {
    decode_feed_row(row)
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
        image_url: Option::<String>::decode(&row[8])?,
        is_favorite: bool::decode(&row[9])?,
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

    // 検証済みの正規化後 URL を保存する (#106 U7): 検証層 (Url::parse) と
    // 保存・fetch する側が違う文字列を見ていると、先頭空白・末尾改行等が
    // ガードを素通りしたまま feeds 行を作ってしまい、以後 fetch_and_store が
    // 永遠に失敗し続ける。大文字スキーム/ホストの正規化も兼ねるので、
    // UNIQUE(url) が素のテキスト比較でも大小違いによる二重登録を防げる。
    let url = match reject_internal_feed_url(&create_req.url) {
        Ok(u) => u,
        Err(msg) => return Ok(error_response(StatusCode::BAD_REQUEST, msg)),
    };

    let conn = db::connect().await?;
    let rows = conn
        .query(
            "INSERT INTO feeds (url) VALUES ($1) \
             ON CONFLICT (url) DO UPDATE SET url = EXCLUDED.url \
             RETURNING id::text, url, title, site_url, etag, last_modified, \
             EXTRACT(EPOCH FROM last_fetched_at)::bigint, \
             EXTRACT(EPOCH FROM created_at)::bigint",
            vec![ParameterValue::Str(url.to_string())],
        )
        .await?
        .collect()
        .await?;

    match rows.first() {
        Some(row) => {
            let feed = row_to_feed(row)?;
            let outcome = immediate_fetch(&conn, &feed).await;
            Ok(immediate_fetch_response(&outcome))
        }
        None => Ok(error_response(
            StatusCode::INTERNAL_SERVER_ERROR,
            "failed to insert feed",
        )),
    }
}

/// ハング対策 (#106): 応答を返さないホストへの send はタイムアウト無しに待ち続ける。
/// WASI の outbound HTTP にタイムアウト API が無いため、一定時間で打ち切る。
///
/// `ui/src/api.ts` の `ADD_FEED_TIMEOUT_MS`（= `UI_ADD_FEED_TIMEOUT_SECS`）は、
/// この値の**2倍**（send + body の各段階）に `UI_TIMEOUT_MARGIN_SECS` の
/// 余裕を足した値を前提にしている (#106 R8/U9、
/// `fetch_timeout_is_positive_and_matches_ui_expectation` で検査)。
/// ここを変えたら `ADD_FEED_TIMEOUT_MS` とそのコメントも見直すこと。
const FETCH_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(15);

/// フィード追加直後の即時取得 (#106)。取得〜保存の実処理は
/// home_rss_shared::fetch::fetch_and_store に一本化されており、定期取得
/// (fetcher) と同じコードを通る。失敗しても追加自体は残す。
async fn immediate_fetch(conn: &spin_sdk::pg::Connection, feed: &Feed) -> ImmediateFetchOutcome {
    match fetch_and_store(conn, &feed.id, &feed.url, None, None, Some(FETCH_TIMEOUT)).await {
        FetchAndStoreOutcome::Stored(updated) => ImmediateFetchOutcome::Fetched(updated),
        // 追加直後は etag/last_modified が無いので 304 は起こらないはずだが、
        // 万一起きた場合は変更無し = 直前に取得済み（＝挿入直後）の行をそのまま
        // 返す。Fetched のコメントの通り、この経路だけは「取得後」ではなく
        // 「取得前」の行になる (#106 R10)。
        FetchAndStoreOutcome::NotModified => ImmediateFetchOutcome::Fetched(feed.clone()),
        FetchAndStoreOutcome::FetchFailed(e) => {
            eprintln!("immediate_fetch {}: {e:#}", feed.url);
            ImmediateFetchOutcome::FetchFailed
        }
        FetchAndStoreOutcome::Unparseable(e) => {
            eprintln!("immediate_fetch {}: {e:#}", feed.url);
            ImmediateFetchOutcome::Unparseable
        }
        FetchAndStoreOutcome::StoreFailed(e) => {
            eprintln!("immediate_fetch {}: {e:#}", feed.url);
            ImmediateFetchOutcome::StoreFailed
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
    let favorite_only = params_map
        .get("favorite")
        .map(|s| s == "true")
        .unwrap_or(false);

    let conn = db::connect().await?;

    let mut sql = ARTICLE_SELECT.to_owned();
    let mut conditions: Vec<String> = Vec::new();
    let mut query_params: Vec<ParameterValue> = Vec::new();

    if let Some(fid) = feed_id {
        query_params.push(ParameterValue::Uuid(fid));
        conditions.push(format!("a.feed_id = ${}", query_params.len()));
    }
    if unread {
        sql.push_str(" LEFT JOIN read_status rs ON a.id = rs.article_id");
        conditions.push("rs.article_id IS NULL".to_owned());
    }
    if favorite_only {
        conditions.push("EXISTS (SELECT 1 FROM favorites f WHERE f.article_id = a.id)".to_owned());
    }
    if !conditions.is_empty() {
        sql.push_str(" WHERE ");
        sql.push_str(&conditions.join(" AND "));
    }
    sql.push_str(" ORDER BY a.published_at DESC NULLS LAST");

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

async fn article_exists(conn: &spin_sdk::pg::Connection, id: &str) -> Result<bool> {
    let rows = conn
        .query(
            "SELECT 1 FROM articles WHERE id = $1",
            vec![ParameterValue::Uuid(id.to_owned())],
        )
        .await?
        .collect()
        .await?;
    Ok(!rows.is_empty())
}

async fn mark_favorite(id: &str) -> Result<Resp> {
    let conn = db::connect().await?;
    // INSERT と存在確認を 1 往復に畳む。確認→INSERT の 2 ホップでは間に
    // フィード削除のカスケードが割り込んで FK 違反で 500 になっていた。
    let affected = conn
        .execute(
            "INSERT INTO favorites (article_id) \
             SELECT $1 WHERE EXISTS (SELECT 1 FROM articles WHERE id = $1) \
             ON CONFLICT DO NOTHING",
            vec![ParameterValue::Uuid(id.to_owned())],
        )
        .await?;
    if affected == 0 && !article_exists(&conn, id).await? {
        return Ok(error_response(StatusCode::NOT_FOUND, "article not found"));
    }

    Ok(empty(StatusCode::NO_CONTENT))
}

async fn unmark_favorite(id: &str) -> Result<Resp> {
    let conn = db::connect().await?;
    if !article_exists(&conn, id).await? {
        return Ok(error_response(StatusCode::NOT_FOUND, "article not found"));
    }
    conn.execute(
        "DELETE FROM favorites WHERE article_id = $1",
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
    let (urls, skipped_invalid) = parse_opml(body.as_ref())?;

    if urls.is_empty() {
        return Ok(error_response(
            StatusCode::BAD_REQUEST,
            "no feeds found in OPML",
        ));
    }

    let conn = db::connect().await?;
    let mut imported = 0u64;
    let mut already_present = 0u64;
    let mut skipped_blocked = 0u64;
    for url in &urls {
        // OPML は任意のホストを持ち込める入力なので、DB に入れる前に add_feed
        // と同じ SSRF ガードを通す (#106 R2)。理由（壊れた OPML エントリか、
        // 内部ホスト宛で拒否されたか）を区別して返す (#106 U2)。保存するのは
        // 正規化後の URL（#106 U7、add_feed と同じ理由）。
        let normalized = match reject_internal_feed_url(url) {
            Ok(u) => u,
            Err(_) => {
                skipped_blocked += 1;
                continue;
            }
        };
        // affected row 数は ON CONFLICT DO NOTHING で 0 のことがある（既存の
        // feed と同じ URL）。0 も imported/skipped のどちらにも数えないと
        // 合計が入力件数と一致しなくなる (#106 U8)。
        let affected = conn
            .execute(
                "INSERT INTO feeds (url) VALUES ($1) ON CONFLICT (url) DO NOTHING",
                vec![ParameterValue::Str(normalized.to_string())],
            )
            .await?;
        if affected > 0 {
            imported += 1;
        } else {
            already_present += 1;
        }
    }

    // imported + already_present + skipped_blocked + skipped_invalid ==
    // OPML 内の xmlUrl 属性の総数、になるようにする (#106 U8)。
    json_ok(format!(
        r#"{{"imported":{imported},"already_present":{already_present},"skipped_invalid":{skipped_invalid},"skipped_blocked":{skipped_blocked}}}"#
    ))
}

/// OPML から xmlUrl を抜き出す。戻り値は (有効な URL 一覧, xmlUrl 属性はあった
/// がその場で使えなかった件数)。後者は空/空白のみ/エンコーディング破損
/// (`\u{FFFD}`) の場合で、xmlUrl 属性自体が無い outline（フォルダ等の
/// 構造要素）はカウントしない (#106 U2)。
fn parse_opml(data: &[u8]) -> Result<(Vec<String>, u64)> {
    let text = String::from_utf8_lossy(data);
    let mut reader = Reader::from_str(&text);
    let mut urls = Vec::new();
    let mut skipped_invalid = 0u64;

    loop {
        match reader.read_event() {
            Ok(Event::Empty(ref e)) | Ok(Event::Start(ref e)) => {
                if e.name().as_ref().eq_ignore_ascii_case("outline") {
                    for attr in e.attributes().flatten() {
                        if attr.key.as_ref().eq_ignore_ascii_case("xmlUrl") {
                            let url = attr.value.trim().to_string();
                            if !url.is_empty() && !url.contains('\u{FFFD}') {
                                urls.push(url);
                            } else {
                                skipped_invalid += 1;
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

    Ok((urls, skipped_invalid))
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
        let (urls, skipped_invalid) = parse_opml(xml).unwrap();
        assert_eq!(urls, vec!["http://a.example/feed", "http://b.example/feed"]);
        assert_eq!(skipped_invalid, 0);
    }

    #[test]
    fn ignores_missing_empty_and_whitespace_only_xml_url() {
        let xml = br#"<opml><body>
            <outline text="no url" />
            <outline text="empty" xmlUrl="" />
            <outline text="whitespace" xmlUrl="   " />
            <outline text="ok" xmlUrl="http://ok.example/feed" />
        </body></opml>"#;
        let (urls, skipped_invalid) = parse_opml(xml).unwrap();
        assert_eq!(urls, vec!["http://ok.example/feed"]);
        // "no url" は xmlUrl 属性自体が無いのでカウントしない。
        // "empty" と "whitespace" はカウントする。
        assert_eq!(skipped_invalid, 2);
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
        let (urls, skipped_invalid) = parse_opml(&xml).unwrap();
        assert_eq!(
            urls,
            vec!["http://broken-text.example/feed", "http://ok.example/feed"]
        );
        assert_eq!(skipped_invalid, 0);
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
        let (urls, skipped_invalid) = parse_opml(&xml).unwrap();
        assert_eq!(urls, vec!["http://ok.example/feed"]);
        assert_eq!(skipped_invalid, 1);
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
        // Stored 由来の Fetched が運ぶのは取得後の最新行。更新前の None のまま
        // 返してはならない（NotModified 由来の Fetched だけが例外で取得前の
        // 行を運ぶ。#106 U5/U10、ImmediateFetchOutcome::Fetched の doc 参照）。
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

    #[test]
    fn store_failure_is_surfaced_as_server_error_not_bad_gateway() {
        use super::{ImmediateFetchOutcome, immediate_fetch_response};
        // 取得自体は成功しているので、上流障害を示す 502 ではなく
        // サーバ側の失敗を示す 500 で返す (#106 B)。
        let resp = immediate_fetch_response(&ImmediateFetchOutcome::StoreFailed);
        assert_eq!(
            resp.status(),
            spin_sdk::http::StatusCode::INTERNAL_SERVER_ERROR
        );
        assert_ne!(resp.status(), spin_sdk::http::StatusCode::BAD_GATEWAY);
    }

    #[test]
    fn fetch_timeout_is_positive_and_matches_ui_expectation() {
        // ui/src/api.ts の ADD_FEED_TIMEOUT_MS (= UI_ADD_FEED_TIMEOUT_SECS) は
        // この値の2倍 (send/body 各段階) + UI_TIMEOUT_MARGIN_SECS の余裕を
        // 前提にしている。0秒化や余裕の食いつぶしのような劣化を検出する
        // sanity テスト (#106 R8/U9)。上限を緩くしすぎると「2倍+余裕」を
        // 守れないまま緑になる（例: 22秒でも旧テストは通った）。
        const UI_ADD_FEED_TIMEOUT_SECS: u64 = 45;
        const UI_TIMEOUT_MARGIN_SECS: u64 = 10;

        assert!(super::FETCH_TIMEOUT.as_secs() > 0);
        assert!(
            super::FETCH_TIMEOUT.as_secs() * 2 + UI_TIMEOUT_MARGIN_SECS <= UI_ADD_FEED_TIMEOUT_SECS
        );
    }
}
