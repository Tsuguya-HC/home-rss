//! End-to-end scenarios: the server, the cleaner, and the fetcher run under
//! `spin up` against a real PostgreSQL, and each test talks to them over HTTP
//! while seeding and checking rows directly in the database. Run through `e2e/run.sh`.
//!
//! Every test starts from empty tables, so they must not run concurrently
//! (`run.sh` passes `--test-threads=1`).

use serde_json::Value;
use tokio_postgres::{Client, NoTls};

fn env(key: &str) -> String {
    std::env::var(key)
        .unwrap_or_else(|_| panic!("{key} is not set; run the suite through e2e/run.sh"))
}

async fn fresh_db() -> Client {
    let (client, conn) = tokio_postgres::connect(&env("E2E_DATABASE_URL"), NoTls)
        .await
        .expect("connect to the e2e database");
    tokio::spawn(async move {
        conn.await.expect("e2e database connection");
    });
    client
        .batch_execute("TRUNCATE feeds, articles, read_status CASCADE")
        .await
        .expect("reset tables");
    client
}

async fn seed_feed(db: &Client, url: &str) -> String {
    db.query_one(
        "INSERT INTO feeds (url) VALUES ($1) RETURNING id::text",
        &[&url],
    )
    .await
    .expect("insert feed")
    .get(0)
}

async fn seed_article(db: &Client, feed_id: &str, slug: &str, age_days: i32) -> String {
    db.query_one(
        "INSERT INTO articles (feed_id, url, title, published_at, fetched_at) \
         VALUES ($1::text::uuid, $2, $3, \
                 now() - make_interval(days => $4), now() - make_interval(days => $4)) \
         RETURNING id::text",
        &[
            &feed_id,
            &format!("https://example.com/{slug}"),
            &slug,
            &age_days,
        ],
    )
    .await
    .expect("insert article")
    .get(0)
}

async fn mark_read_in_db(db: &Client, article_id: &str) {
    db.execute(
        "INSERT INTO read_status (article_id) VALUES ($1::text::uuid)",
        &[&article_id],
    )
    .await
    .expect("insert read_status");
}

async fn count(db: &Client, sql: &str) -> i64 {
    db.query_one(sql, &[]).await.expect(sql).get(0)
}

fn server(path: &str) -> String {
    format!("{}{path}", env("E2E_SERVER_URL"))
}

fn fetcher(path: &str) -> String {
    format!("{}{path}", env("E2E_FETCHER_URL"))
}

async fn post_fetcher(path: &str) -> u16 {
    reqwest::Client::new()
        .post(fetcher(path))
        .send()
        .await
        .expect("POST fetcher")
        .status()
        .as_u16()
}

async fn get_json(path: &str) -> (u16, Value) {
    let resp = reqwest::get(server(path)).await.expect("GET");
    let status = resp.status().as_u16();
    (status, resp.json().await.expect("JSON body"))
}

async fn post(path: &str, body: Option<&str>) -> u16 {
    let mut req = reqwest::Client::new().post(server(path));
    if let Some(body) = body {
        req = req
            .header("content-type", "application/json")
            .body(body.to_owned());
    }
    req.send().await.expect("POST").status().as_u16()
}

async fn delete(path: &str) -> u16 {
    reqwest::Client::new()
        .delete(server(path))
        .send()
        .await
        .expect("DELETE")
        .status()
        .as_u16()
}

fn titles(articles: &Value) -> Vec<&str> {
    let mut titles: Vec<&str> = articles
        .as_array()
        .expect("array of articles")
        .iter()
        .map(|a| a["title"].as_str().expect("title"))
        .collect();
    titles.sort_unstable();
    titles
}

#[tokio::test]
async fn stats_count_feeds_and_unread_articles() {
    let db = fresh_db().await;
    let feed = seed_feed(&db, "https://a.example/feed").await;
    let read = seed_article(&db, &feed, "read", 1).await;
    seed_article(&db, &feed, "unread", 1).await;
    mark_read_in_db(&db, &read).await;

    let (status, body) = get_json("/api/stats").await;
    assert_eq!(status, 200);
    assert_eq!(body, serde_json::json!({"feeds": 1, "unread": 1}));
}

#[tokio::test]
async fn list_feeds_returns_stored_feeds() {
    let db = fresh_db().await;
    let id = seed_feed(&db, "https://a.example/feed").await;

    let (status, body) = get_json("/api/feeds").await;
    assert_eq!(status, 200);
    let feeds = body.as_array().expect("array of feeds");
    assert_eq!(feeds.len(), 1);
    assert_eq!(feeds[0]["id"], id.as_str());
    assert_eq!(feeds[0]["url"], "https://a.example/feed");
}

#[tokio::test]
async fn deleting_a_feed_removes_its_articles_and_read_state() {
    let db = fresh_db().await;
    let feed = seed_feed(&db, "https://a.example/feed").await;
    let article = seed_article(&db, &feed, "one", 1).await;
    mark_read_in_db(&db, &article).await;

    assert_eq!(delete(&format!("/api/feeds/{feed}")).await, 204);
    assert_eq!(count(&db, "SELECT COUNT(*) FROM articles").await, 0);
    assert_eq!(count(&db, "SELECT COUNT(*) FROM read_status").await, 0);

    assert_eq!(delete(&format!("/api/feeds/{feed}")).await, 404);
}

#[tokio::test]
async fn article_list_filters_by_feed_and_unread() {
    let db = fresh_db().await;
    let a = seed_feed(&db, "https://a.example/feed").await;
    let b = seed_feed(&db, "https://b.example/feed").await;
    let a_read = seed_article(&db, &a, "a-read", 1).await;
    seed_article(&db, &a, "a-unread", 1).await;
    seed_article(&db, &b, "b-unread", 1).await;
    mark_read_in_db(&db, &a_read).await;

    let (_, all) = get_json("/api/articles").await;
    assert_eq!(titles(&all), ["a-read", "a-unread", "b-unread"]);

    let (_, unread) = get_json("/api/articles?unread=true").await;
    assert_eq!(titles(&unread), ["a-unread", "b-unread"]);

    let (_, of_a) = get_json(&format!("/api/articles?feed_id={a}")).await;
    assert_eq!(titles(&of_a), ["a-read", "a-unread"]);

    let (_, unread_of_a) = get_json(&format!("/api/articles?feed_id={a}&unread=true")).await;
    assert_eq!(titles(&unread_of_a), ["a-unread"]);
}

#[tokio::test]
async fn marking_read_is_idempotent_and_read_all_clears_unread() {
    let db = fresh_db().await;
    let feed = seed_feed(&db, "https://a.example/feed").await;
    let first = seed_article(&db, &feed, "first", 1).await;
    seed_article(&db, &feed, "second", 1).await;

    assert_eq!(
        post(&format!("/api/articles/{first}/read"), None).await,
        204
    );
    assert_eq!(
        post(&format!("/api/articles/{first}/read"), None).await,
        204
    );
    assert_eq!(count(&db, "SELECT COUNT(*) FROM read_status").await, 1);

    assert_eq!(post("/api/articles/read-all", None).await, 204);
    let (_, stats) = get_json("/api/stats").await;
    assert_eq!(stats["unread"], 0);
}

#[tokio::test]
async fn adding_a_feed_rejects_urls_the_fetcher_must_not_reach() {
    let db = fresh_db().await;
    // Each of these is refused before any fetch, so no network is touched.
    for body in [
        r#"{"url":"http://a.example/feed"}"#,
        r#"{"url":"https://a.example:8443/feed"}"#,
        r#"{"url":"https://127.0.0.1/feed"}"#,
        r#"not json"#,
    ] {
        assert_eq!(post("/api/feeds", Some(body)).await, 400, "{body}");
    }
    assert_eq!(count(&db, "SELECT COUNT(*) FROM feeds").await, 0);
}

#[tokio::test]
async fn adding_a_feed_whose_first_fetch_fails_leaves_no_feed_behind() {
    // Catches an add without rollback: the feed row is inserted before the
    // first fetch runs, so a fetch failure must undo the insert instead of
    // leaving a zero-article row behind (#148 started from such a leftover,
    // left by a feed answering 302, that had to be deleted by hand).
    // `.invalid` never resolves, so the fetch fails without touching an
    // external network, and fast (about 90ms through Spin, measured 2026-09-30
    // by POSTing this URL to a local `spin up`).
    let db = fresh_db().await;
    let body = r#"{"url":"https://no-such-feed.invalid/feed"}"#;
    assert_eq!(post("/api/feeds", Some(body)).await, 502);
    assert_eq!(count(&db, "SELECT COUNT(*) FROM feeds").await, 0);
    assert_eq!(count(&db, "SELECT COUNT(*) FROM articles").await, 0);
    // Retrying the same URL must look like a fresh add: no rows pile up and
    // nothing left by the first attempt gets in the way.
    assert_eq!(post("/api/feeds", Some(body)).await, 502);
    assert_eq!(count(&db, "SELECT COUNT(*) FROM feeds").await, 0);
}

#[tokio::test]
async fn deleting_a_feed_is_not_blocked_by_a_concurrent_failing_readd() {
    // Pins the non-blocking shape of #148's fix: the re-add fetches outside
    // any transaction, so nothing holds the row's lock while the fetch hangs.
    // 203.0.0.1 (TEST-NET-3) is unreachable, so the fetch hangs until the
    // server's 15s send timeout (measured 15.0s through Spin, 2026-09-30).
    // Before the fix the re-add's INSERT..ON CONFLICT held the lock across
    // the whole fetch and DELETE waited ~15s; fixed, it answers in well
    // under that.
    let db = fresh_db().await;
    let url = "https://203.0.0.1/concurrent-readd-feed";
    let id: String = db
        .query_one(
            "INSERT INTO feeds (url) VALUES ($1) RETURNING id::text",
            &[&url],
        )
        .await
        .expect("seed feed")
        .get(0);
    let body = format!(r#"{{"url":"{url}"}}"#);

    let server_url = server("/api/feeds");
    let readd = tokio::spawn(async move {
        reqwest::Client::new()
            .post(server_url)
            .header("content-type", "application/json")
            .body(body)
            .send()
            .await
            .expect("POST re-add")
            .status()
            .as_u16()
    });
    // Give the re-add a head start into its ~15s fetch so the DELETE lands
    // while the fetch is still hanging. Fixed, the DELETE answers without
    // waiting for the fetch either way.
    tokio::time::sleep(std::time::Duration::from_secs(2)).await;

    let start = std::time::Instant::now();
    let status = delete(&format!("/api/feeds/{id}")).await;
    let elapsed = start.elapsed();
    assert_eq!(status, 204);
    // The fetch hangs ~15s; a DELETE that waits for it lands at 10s+. Fixed,
    // the DELETE never touches the locked row's wait queue. 5s keeps clear of
    // both the failure mode and normal jitter.
    assert!(
        elapsed < std::time::Duration::from_secs(5),
        "DELETE waited {elapsed:?} for a concurrent re-add's fetch"
    );
    // A re-add that started from the same pre-existing row must keep it, so
    // the DELETE is last and the feed is gone either way.
    assert_eq!(readd.await.expect("re-add task"), 502);
    assert_eq!(count(&db, "SELECT COUNT(*) FROM feeds").await, 0);
}

#[tokio::test]
async fn readding_an_existing_feed_that_fails_to_fetch_keeps_it_unchanged() {
    // Guards the naive fix for the test above: deleting the feed row when the
    // first fetch fails would also wipe a feed that was already registered.
    // The same never-resolving `.invalid` URL fails the re-add's fetch.
    let db = fresh_db().await;
    let url = "https://no-such-feed.invalid/feed";
    let id = seed_feed(&db, url).await;
    db.execute(
        "UPDATE feeds SET title = $1 WHERE id = $2::text::uuid",
        &[&"Original Title", &id],
    )
    .await
    .expect("set feed title");
    seed_article(&db, &id, "kept", 1).await;

    let body = format!(r#"{{"url":"{url}"}}"#);
    assert_eq!(post("/api/feeds", Some(&body)).await, 502);

    assert_eq!(count(&db, "SELECT COUNT(*) FROM feeds").await, 1);
    let row = db
        .query_one(
            "SELECT id::text, url, title, last_fetch_error, fetch_failing_since FROM feeds",
            &[],
        )
        .await
        .expect("read feed");
    let kept_id: String = row.get(0);
    let kept_url: String = row.get(1);
    let kept_title: Option<String> = row.get(2);
    assert_eq!(kept_id, id);
    assert_eq!(kept_url, url);
    assert_eq!(kept_title.as_deref(), Some("Original Title"));
    // The immediate fetch must neither write nor clear the scheduled-fetch
    // failure record (#245).
    let kept_error: Option<String> = row.get(3);
    let kept_since: Option<std::time::SystemTime> = row.get(4);
    assert_eq!(kept_error, None);
    assert!(kept_since.is_none());
    assert_eq!(count(&db, "SELECT COUNT(*) FROM articles").await, 1);
}

#[tokio::test]
async fn fetcher_records_a_guard_rejection_but_leaves_never_fetched_feeds_unmarked() {
    // Catches the fetcher swallowing a per-feed failure without a trace: the
    // internal URL never leaves the box, the SSRF guard rejects it, and the
    // rejection must land in the failure columns (#245). The never-fetched
    // row pins the OPML-imported exception in the same run.
    let db = fresh_db().await;
    let failing: String = db
        .query_one(
            "INSERT INTO feeds (url, last_fetched_at) VALUES ($1, now() - make_interval(days => 1)) RETURNING id::text",
            &[&"https://localhost/feed"],
        )
        .await
        .expect("insert ever-fetched feed")
        .get(0);
    let fresh: String = db
        .query_one(
            "INSERT INTO feeds (url) VALUES ($1) RETURNING id::text",
            &[&"https://localhost/never-fetched"],
        )
        .await
        .expect("insert never-fetched feed")
        .get(0);

    assert_eq!(post_fetcher("/fetch").await, 200);

    let row = db
        .query_one(
            "SELECT last_fetch_error, EXTRACT(EPOCH FROM fetch_failing_since)::bigint FROM feeds WHERE id = $1::text::uuid",
            &[&failing],
        )
        .await
        .expect("read failure record");
    let reason: Option<String> = row.get(0);
    let since: Option<i64> = row.get(1);
    let reason = reason.expect("a guard rejection must be recorded");
    assert!(!reason.is_empty());
    assert!(reason.chars().count() <= 200);
    let since = since.expect("a guard rejection must stamp when it started");
    assert!(since > 0);

    let row = db
        .query_one(
            "SELECT last_fetch_error, EXTRACT(EPOCH FROM fetch_failing_since)::bigint FROM feeds WHERE id = $1::text::uuid",
            &[&fresh],
        )
        .await
        .expect("read never-fetched record");
    let reason: Option<String> = row.get(0);
    let since: Option<i64> = row.get(1);
    assert_eq!(reason, None);
    assert_eq!(since, None);
}

#[tokio::test]
async fn list_feeds_exposes_the_failure_columns() {
    // Catches GET /api/feeds dropping the new columns: a failure recorded
    // directly in the DB must show up in the feed JSON (#245).
    let db = fresh_db().await;
    let id = seed_feed(&db, "https://a.example/feed").await;
    db.execute(
        "UPDATE feeds SET last_fetch_error = $1, fetch_failing_since = now() - make_interval(days => 2) WHERE id = $2::text::uuid",
        &[&"HTTP 404", &id],
    )
    .await
    .expect("seed failure record");

    let (status, body) = get_json("/api/feeds").await;
    assert_eq!(status, 200);
    let feeds = body.as_array().expect("array of feeds");
    assert_eq!(feeds.len(), 1);
    assert_eq!(feeds[0]["id"], id.as_str());
    assert_eq!(feeds[0]["last_fetch_error"], "HTTP 404");
    let since = feeds[0]["fetch_failing_since"]
        .as_i64()
        .expect("failing-since as epoch seconds");
    assert!(since > 0);
}

#[tokio::test]
async fn cleaner_deletes_only_read_articles_past_retention() {
    let db = fresh_db().await;
    let feed = seed_feed(&db, "https://a.example/feed").await;
    // 15 days is past the e2e retention (10) but within the component's default
    // (30), so this only goes away if run.sh's --variable retention_days is honored.
    let old_read = seed_article(&db, &feed, "old-read", 15).await;
    seed_article(&db, &feed, "old-unread", 15).await;
    let recent_read = seed_article(&db, &feed, "recent-read", 5).await;
    mark_read_in_db(&db, &old_read).await;
    mark_read_in_db(&db, &recent_read).await;

    let resp = reqwest::Client::new()
        .post(format!("{}/clean", env("E2E_CLEANER_URL")))
        .send()
        .await
        .expect("POST /clean");
    assert_eq!(resp.status().as_u16(), 200);

    let (_, left) = get_json("/api/articles").await;
    assert_eq!(titles(&left), ["old-unread", "recent-read"]);
}
