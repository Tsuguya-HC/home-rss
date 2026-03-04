use anyhow::Result;
use home_rss_shared::db;
use home_rss_shared::models::{Article, CreateFeedRequest, Feed};
use quick_xml::events::Event;
use quick_xml::Reader;
use spin_sdk::http::{IntoResponse, Params, Request, Response, Router};
use spin_sdk::http_component;
use spin_sdk::pg4::{Decode, ParameterValue};

#[http_component]
fn handle(req: Request) -> Result<impl IntoResponse> {
    let mut router = Router::default();
    router.get("/api/feeds", list_feeds);
    router.post("/api/feeds", add_feed);
    router.delete("/api/feeds/:id", delete_feed);
    router.get("/api/articles", list_articles);
    router.post("/api/articles/:id/read", mark_read);
    router.post("/api/articles/read-all", mark_all_read);
    router.post("/api/import/opml", import_opml);
    router.get("/api/stats", get_stats);
    Ok(router.handle(req))
}

fn json_ok(body: impl Into<String>) -> Result<Response> {
    Ok(Response::builder()
        .status(200)
        .header("content-type", "application/json")
        .body(body.into())
        .build())
}

fn error_response(status: u16, message: &str) -> Result<Response> {
    Ok(Response::builder()
        .status(status)
        .header("content-type", "application/json")
        .body(format!(r#"{{"error":{}}}"#, serde_json::to_string(message)?))
        .build())
}

fn parse_query(uri: &str) -> std::collections::HashMap<String, String> {
    let query = uri.splitn(2, '?').nth(1).unwrap_or("");
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

const FEED_SELECT: &str =
    "SELECT id::text, url, title, site_url, etag, last_modified, \
     EXTRACT(EPOCH FROM last_fetched_at)::bigint, \
     EXTRACT(EPOCH FROM created_at)::bigint \
     FROM feeds";

const ARTICLE_SELECT: &str =
    "SELECT a.id::text, a.feed_id::text, a.url, a.title, a.content, a.author, \
     EXTRACT(EPOCH FROM a.published_at)::bigint, \
     EXTRACT(EPOCH FROM a.fetched_at)::bigint \
     FROM articles a";

fn row_to_feed(row: &[spin_sdk::pg4::DbValue]) -> Result<Feed> {
    Ok(Feed {
        id: String::decode(&row[0]).map_err(|e| anyhow::anyhow!("{e}"))?,
        url: String::decode(&row[1]).map_err(|e| anyhow::anyhow!("{e}"))?,
        title: Option::<String>::decode(&row[2]).map_err(|e| anyhow::anyhow!("{e}"))?,
        site_url: Option::<String>::decode(&row[3]).map_err(|e| anyhow::anyhow!("{e}"))?,
        etag: Option::<String>::decode(&row[4]).map_err(|e| anyhow::anyhow!("{e}"))?,
        last_modified: Option::<String>::decode(&row[5])
            .map_err(|e| anyhow::anyhow!("{e}"))?,
        last_fetched_at: Option::<i64>::decode(&row[6])
            .map_err(|e| anyhow::anyhow!("{e}"))?,
        created_at: Option::<i64>::decode(&row[7]).map_err(|e| anyhow::anyhow!("{e}"))?,
    })
}

fn row_to_article(row: &[spin_sdk::pg4::DbValue]) -> Result<Article> {
    Ok(Article {
        id: String::decode(&row[0]).map_err(|e| anyhow::anyhow!("{e}"))?,
        feed_id: String::decode(&row[1]).map_err(|e| anyhow::anyhow!("{e}"))?,
        url: String::decode(&row[2]).map_err(|e| anyhow::anyhow!("{e}"))?,
        title: String::decode(&row[3]).map_err(|e| anyhow::anyhow!("{e}"))?,
        content: Option::<String>::decode(&row[4]).map_err(|e| anyhow::anyhow!("{e}"))?,
        author: Option::<String>::decode(&row[5]).map_err(|e| anyhow::anyhow!("{e}"))?,
        published_at: Option::<i64>::decode(&row[6])
            .map_err(|e| anyhow::anyhow!("{e}"))?,
        fetched_at: Option::<i64>::decode(&row[7]).map_err(|e| anyhow::anyhow!("{e}"))?,
    })
}

fn list_feeds(_req: Request, _params: Params) -> Result<Response> {
    let conn = db::connect()?;
    let result = conn.query(
        &format!("{FEED_SELECT} ORDER BY created_at DESC"),
        &[],
    )?;
    let feeds: Vec<Feed> = result
        .rows
        .iter()
        .map(|row| row_to_feed(row))
        .collect::<Result<_>>()?;
    json_ok(serde_json::to_string(&feeds)?)
}

fn add_feed(req: Request, _params: Params) -> Result<Response> {
    let create_req: CreateFeedRequest = match serde_json::from_slice(req.body()) {
        Ok(r) => r,
        Err(_) => return error_response(400, "invalid JSON body"),
    };

    let conn = db::connect()?;
    let result = conn.query(
        &format!(
            "INSERT INTO feeds (url) VALUES ($1) \
             ON CONFLICT (url) DO UPDATE SET url = EXCLUDED.url \
             RETURNING id::text, url, title, site_url, etag, last_modified, \
             EXTRACT(EPOCH FROM last_fetched_at)::bigint, \
             EXTRACT(EPOCH FROM created_at)::bigint"
        ),
        &[ParameterValue::Str(create_req.url)],
    )?;

    match result.rows.first() {
        Some(row) => {
            let feed = row_to_feed(row)?;
            Ok(Response::builder()
                .status(201)
                .header("content-type", "application/json")
                .body(serde_json::to_string(&feed)?)
                .build())
        }
        None => error_response(500, "failed to insert feed"),
    }
}

fn delete_feed(_req: Request, params: Params) -> Result<Response> {
    let id = match params.get("id") {
        Some(id) => id.to_string(),
        None => return error_response(400, "missing id"),
    };

    let conn = db::connect()?;
    let rows = conn.execute(
        "DELETE FROM feeds WHERE id = $1::uuid",
        &[ParameterValue::Str(id)],
    )?;

    if rows == 0 {
        error_response(404, "feed not found")
    } else {
        Ok(Response::builder().status(204).body(()).build())
    }
}

fn list_articles(req: Request, _params: Params) -> Result<Response> {
    let params_map = parse_query(req.uri());
    let feed_id = params_map.get("feed_id").cloned();
    let unread = params_map
        .get("unread")
        .map(|s| s == "true")
        .unwrap_or(false);

    let conn = db::connect()?;

    let (sql, query_params): (String, Vec<ParameterValue>) = match (feed_id, unread) {
        (Some(fid), true) => (
            format!(
                "{ARTICLE_SELECT} \
                 LEFT JOIN read_status rs ON a.id = rs.article_id \
                 WHERE a.feed_id = $1::uuid AND rs.article_id IS NULL \
                 ORDER BY a.published_at DESC NULLS LAST"
            ),
            vec![ParameterValue::Str(fid)],
        ),
        (Some(fid), false) => (
            format!(
                "{ARTICLE_SELECT} \
                 WHERE a.feed_id = $1::uuid \
                 ORDER BY a.published_at DESC NULLS LAST"
            ),
            vec![ParameterValue::Str(fid)],
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

    let result = conn.query(&sql, &query_params)?;
    let articles: Vec<Article> = result
        .rows
        .iter()
        .map(|row| row_to_article(row))
        .collect::<Result<_>>()?;
    json_ok(serde_json::to_string(&articles)?)
}

fn mark_read(_req: Request, params: Params) -> Result<Response> {
    let id = match params.get("id") {
        Some(id) => id.to_string(),
        None => return error_response(400, "missing id"),
    };

    let conn = db::connect()?;
    conn.execute(
        "INSERT INTO read_status (article_id) VALUES ($1::uuid) ON CONFLICT DO NOTHING",
        &[ParameterValue::Str(id)],
    )?;

    Ok(Response::builder().status(204).body(()).build())
}

fn mark_all_read(_req: Request, _params: Params) -> Result<Response> {
    let conn = db::connect()?;
    conn.execute(
        "INSERT INTO read_status (article_id) \
         SELECT id FROM articles \
         WHERE id NOT IN (SELECT article_id FROM read_status) \
         ON CONFLICT DO NOTHING",
        &[],
    )?;

    Ok(Response::builder().status(204).body(()).build())
}

fn import_opml(req: Request, _params: Params) -> Result<Response> {
    let urls = parse_opml(req.body())?;

    if urls.is_empty() {
        return error_response(400, "no feeds found in OPML");
    }

    let conn = db::connect()?;
    let mut imported = 0u32;
    for url in &urls {
        conn.execute(
            "INSERT INTO feeds (url) VALUES ($1) ON CONFLICT (url) DO NOTHING",
            &[ParameterValue::Str(url.clone())],
        )?;
        imported += 1;
    }

    json_ok(format!(r#"{{"imported":{imported}}}"#))
}

fn parse_opml(data: &[u8]) -> Result<Vec<String>> {
    let mut reader = Reader::from_reader(data);
    let mut urls = Vec::new();
    let mut buf = Vec::new();

    loop {
        match reader.read_event_into(&mut buf) {
            Ok(Event::Empty(ref e)) | Ok(Event::Start(ref e)) => {
                if e.name().as_ref().eq_ignore_ascii_case(b"outline") {
                    for attr in e.attributes().flatten() {
                        if attr.key.as_ref().eq_ignore_ascii_case(b"xmlUrl") {
                            if let Ok(val) = std::str::from_utf8(&attr.value) {
                                let url = val.trim().to_string();
                                if !url.is_empty() {
                                    urls.push(url);
                                }
                            }
                        }
                    }
                }
            }
            Ok(Event::Eof) => break,
            Err(e) => return Err(anyhow::anyhow!("OPML parse error: {e}")),
            _ => {}
        }
        buf.clear();
    }

    Ok(urls)
}

fn get_stats(_req: Request, _params: Params) -> Result<Response> {
    let conn = db::connect()?;
    let result = conn.query(
        "SELECT \
         (SELECT COUNT(*)::bigint FROM feeds) AS feeds, \
         (SELECT COUNT(*)::bigint FROM articles \
          WHERE id NOT IN (SELECT article_id FROM read_status)) AS unread",
        &[],
    )?;

    let (feeds, unread) = match result.rows.first() {
        Some(row) => {
            let feeds = i64::decode(&row[0]).map_err(|e| anyhow::anyhow!("{e}"))?;
            let unread = i64::decode(&row[1]).map_err(|e| anyhow::anyhow!("{e}"))?;
            (feeds, unread)
        }
        None => (0, 0),
    };

    json_ok(format!(r#"{{"feeds":{feeds},"unread":{unread}}}"#))
}
