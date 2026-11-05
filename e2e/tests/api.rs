//! End-to-end scenarios: the server and the cleaner run under `spin up` against a
//! real PostgreSQL, and each test talks to them over HTTP while seeding and
//! checking rows directly in the database. Run through `e2e/run.sh`.
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

async fn post_fetch() -> u16 {
    reqwest::Client::new()
        .post(format!("{}/fetch", env("E2E_FETCHER_URL")))
        .send()
        .await
        .expect("POST /fetch")
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
        .query_one("SELECT id::text, url, title FROM feeds", &[])
        .await
        .expect("read feed");
    let kept_id: String = row.get(0);
    let kept_url: String = row.get(1);
    let kept_title: Option<String> = row.get(2);
    assert_eq!(kept_id, id);
    assert_eq!(kept_url, url);
    assert_eq!(kept_title.as_deref(), Some("Original Title"));
    assert_eq!(count(&db, "SELECT COUNT(*) FROM articles").await, 1);
}

#[tokio::test]
async fn fetcher_records_a_guard_rejection_as_a_fetch_failure() {
    // #245: fetcher の定期取得で URL ガードに弾かれたフィードは、失敗として
    // feeds に記録される。https://localhost/feed は SSRF ガードに弾かれるので
    // 外部へは出ない。未実装では last_fetch_error 列自体が無く落ちる。
    let db = fresh_db().await;
    let feed = seed_feed(&db, "https://localhost/feed").await;
    // 成功歴のある行でも記録されることの確認用に、一度取得したことにする
    // （成功歴の無い行の記録は `fetcher_records_a_never_fetched_feed_on_its_first_fetch`
    // が別に固定する）。
    db.execute(
        "UPDATE feeds SET last_fetched_at = now() WHERE id = $1::text::uuid",
        &[&feed],
    )
    .await
    .expect("mark feed as fetched");

    assert_eq!(post_fetch().await, 200);

    let row = db
        .query_one(
            "SELECT last_fetch_error, fetch_failing_since IS NOT NULL AS failing, \
             fetch_failing_since::text FROM feeds",
            &[],
        )
        .await
        .expect("read failure record");
    let reason: Option<String> = row.get(0);
    let failing: bool = row.get(1);
    let since: Option<String> = row.get(2);
    let reason = reason.expect("a guard rejection must be recorded");
    assert!(!reason.is_empty());
    assert!(failing);
    // 失敗が続いても fetch_failing_since は上書きされない。もう1回叩いて同じ値のままにする。
    assert_eq!(post_fetch().await, 200);
    let again: Option<String> = db
        .query_one("SELECT fetch_failing_since::text FROM feeds", &[])
        .await
        .expect("read failure record again")
        .get(0);
    assert_eq!(again, since);
}

#[tokio::test]
async fn fetcher_records_a_never_fetched_feed_on_its_first_fetch() {
    // #245 item4: OPML 直後など、一度も成功したことがない行でも、取得の
    // 試みが失敗すればその 1 回目から失敗列が書かれることを固定する。
    // 試み自体がまだ無い行（次の cron tick 前）は NULL のまま。
    let db = fresh_db().await;
    seed_feed(&db, "https://localhost/feed").await;

    let before: Option<String> = db
        .query_one("SELECT last_fetch_error FROM feeds", &[])
        .await
        .expect("read failure record before any fetch")
        .get(0);
    assert_eq!(before, None);

    assert_eq!(post_fetch().await, 200);

    let row = db
        .query_one(
            "SELECT last_fetch_error, fetch_failing_since IS NOT NULL AS failing FROM feeds",
            &[],
        )
        .await
        .expect("read failure record");
    let reason: Option<String> = row.get(0);
    let failing: bool = row.get(1);
    let reason = reason.expect("a failing first fetch must be recorded");
    assert!(!reason.is_empty());
    assert!(failing);
}

#[tokio::test]
async fn fetcher_eventually_marks_a_never_fetched_feed_that_keeps_failing() {
    // #245 item1 (HIGH): OPML 直後など、一度も取得に成功したことがない
    // フィード（last_fetched_at が NULL のまま）が恒常的に失敗し続けたら、
    // last_fetch_error / fetch_failing_since が書かれる。初回から記録する
    // 直しでは 1 回でも書かれる。
    let db = fresh_db().await;
    seed_feed(&db, "https://localhost/feed").await;

    // まだ 1 回も process_feed が呼ばれていない行には印が付かない。
    let before: Option<String> = db
        .query_one("SELECT last_fetch_error FROM feeds", &[])
        .await
        .expect("read failure record before any fetch")
        .get(0);
    assert_eq!(before, None);

    // https://localhost/feed は SSRF ガードに弾かれるので外部へは出ない。
    // 閾値を決めないため多めに繰り返す。
    for _ in 0..10 {
        assert_eq!(post_fetch().await, 200);
    }

    let row = db
        .query_one(
            "SELECT last_fetch_error, fetch_failing_since IS NOT NULL AS failing FROM feeds",
            &[],
        )
        .await
        .expect("read failure record");
    let reason: Option<String> = row.get(0);
    let failing: bool = row.get(1);
    let reason = reason.expect("a permanently failing feed must eventually be recorded");
    assert!(!reason.is_empty());
    assert!(failing);
}

#[tokio::test]
async fn feed_list_exposes_the_fetch_failure_record() {
    // #245: DB に直接入れた失敗の記録が GET /api/feeds に出る。未実装では
    // 列が無く INSERT で落ちる。
    let db = fresh_db().await;
    let feed = seed_feed(&db, "https://a.example/feed").await;
    db.execute(
        "UPDATE feeds SET last_fetch_error = $1, fetch_failing_since = now() \
         WHERE id = $2::text::uuid",
        &[&"HTTP 404", &feed],
    )
    .await
    .expect("seed failure record");

    let (status, body) = get_json("/api/feeds").await;
    assert_eq!(status, 200);
    let feeds = body.as_array().expect("array of feeds");
    assert_eq!(feeds.len(), 1);
    assert_eq!(feeds[0]["last_fetch_error"], "HTTP 404");
    assert!(feeds[0]["fetch_failing_since"].is_number());
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
