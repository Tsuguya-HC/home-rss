//! End-to-end scenarios: the server and the cleaner run under `spin up` against a
//! real PostgreSQL, and each test talks to them over HTTP while seeding and
//! checking rows directly in the database. Run through `e2e/run.sh`.
//!
//! Every test starts from empty tables, so they must not run concurrently
//! (`run.sh` passes `--test-threads=1`).

use serde_json::Value;
use tokio_postgres::{Client, NoTls};

fn env(key: &str) -> String {
    std::env::var(key).unwrap_or_else(|_| panic!("{key} is not set; run the suite through e2e/run.sh"))
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
    db.query_one("INSERT INTO feeds (url) VALUES ($1) RETURNING id::text", &[&url])
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
        &[&feed_id, &format!("https://example.com/{slug}"), &slug, &age_days],
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
        req = req.header("content-type", "application/json").body(body.to_owned());
    }
    req.send().await.expect("POST").status().as_u16()
}

async fn delete(path: &str) -> u16 {
    reqwest::Client::new().delete(server(path)).send().await.expect("DELETE").status().as_u16()
}

async fn mark_favorite(article_id: &str) -> u16 {
    post(&format!("/api/articles/{article_id}/favorite"), None).await
}

async fn unmark_favorite(article_id: &str) -> u16 {
    delete(&format!("/api/articles/{article_id}/favorite")).await
}

async fn run_cleaner() -> u16 {
    reqwest::Client::new()
        .post(format!("{}/clean", env("E2E_CLEANER_URL")))
        .send()
        .await
        .expect("POST /clean")
        .status()
        .as_u16()
}

fn is_favorite(article: &Value) -> bool {
    article["is_favorite"].as_bool().expect("article has is_favorite")
}

fn article_by_id<'a>(articles: &'a Value, id: &str) -> &'a Value {
    articles
        .as_array()
        .expect("array of articles")
        .iter()
        .find(|a| a["id"] == id)
        .expect("article is listed")
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

    assert_eq!(post(&format!("/api/articles/{first}/read"), None).await, 204);
    assert_eq!(post(&format!("/api/articles/{first}/read"), None).await, 204);
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

// POST marks and DELETE unmarks; repeating either way keeps the mark stable.
// Catches a missing favorite route (404 where 204 is expected) and a mark
// that is not reflected back in the article list.
#[tokio::test]
async fn marking_and_unmarking_favorite_is_idempotent_both_ways() {
    let db = fresh_db().await;
    let feed = seed_feed(&db, "https://a.example/feed").await;
    let article = seed_article(&db, &feed, "one", 1).await;

    assert_eq!(mark_favorite(&article).await, 204);
    assert_eq!(mark_favorite(&article).await, 204);
    let (_, listed) = get_json("/api/articles").await;
    assert!(
        is_favorite(article_by_id(&listed, &article)),
        "marked article reports is_favorite=true",
    );

    assert_eq!(unmark_favorite(&article).await, 204);
    assert_eq!(unmark_favorite(&article).await, 204);
    let (_, relisted) = get_json("/api/articles").await;
    assert!(
        !is_favorite(article_by_id(&relisted, &article)),
        "unmarked article reports is_favorite=false",
    );
}

// The favorites-only filter combines with the feed and unread filters:
// feed x unread x favorites must all narrow the same list.
#[tokio::test]
async fn favorites_only_filter_combines_with_feed_and_unread() {
    let db = fresh_db().await;
    let a = seed_feed(&db, "https://a.example/feed").await;
    let b = seed_feed(&db, "https://b.example/feed").await;
    let a_fav_read = seed_article(&db, &a, "a-fav-read", 1).await;
    let a_fav_unread = seed_article(&db, &a, "a-fav-unread", 1).await;
    let b_fav_unread = seed_article(&db, &b, "b-fav-unread", 1).await;
    seed_article(&db, &a, "a-plain-unread", 1).await;
    for id in [&a_fav_read, &a_fav_unread, &b_fav_unread] {
        assert_eq!(mark_favorite(id).await, 204);
    }
    mark_read_in_db(&db, &a_fav_read).await;

    let (_, favs_only) = get_json("/api/articles?favorites_only=true").await;
    assert_eq!(titles(&favs_only), ["a-fav-read", "a-fav-unread", "b-fav-unread"]);

    let (_, favs_of_a) = get_json(&format!("/api/articles?feed_id={a}&favorites_only=true")).await;
    assert_eq!(titles(&favs_of_a), ["a-fav-read", "a-fav-unread"]);

    let (_, unread_favs) = get_json("/api/articles?unread=true&favorites_only=true").await;
    assert_eq!(titles(&unread_favs), ["a-fav-unread", "b-fav-unread"]);

    let (_, unread_favs_of_a) =
        get_json(&format!("/api/articles?feed_id={a}&unread=true&favorites_only=true")).await;
    assert_eq!(titles(&unread_favs_of_a), ["a-fav-unread"]);
}

// Marking an article that no longer exists answers 404, not 500, on both the
// mark and unmark paths; a real mark next to it proves the route itself works.
#[tokio::test]
async fn marking_a_missing_article_returns_not_found() {
    let db = fresh_db().await;
    let feed = seed_feed(&db, "https://a.example/feed").await;
    let real = seed_article(&db, &feed, "real", 1).await;
    let missing = "00000000-0000-0000-0000-000000000000";

    assert_eq!(mark_favorite(missing).await, 404);
    assert_eq!(unmark_favorite(missing).await, 404);
    assert_eq!(mark_favorite(&real).await, 204);
    assert_eq!(unmark_favorite(&real).await, 204);
}

// Deleting a feed is explicit, so it removes the favorited article; the
// article list no longer shows it.
#[tokio::test]
async fn deleting_a_feed_removes_favorited_articles() {
    let db = fresh_db().await;
    let feed = seed_feed(&db, "https://a.example/feed").await;
    let article = seed_article(&db, &feed, "fav", 1).await;
    assert_eq!(mark_favorite(&article).await, 204);

    assert_eq!(delete(&format!("/api/feeds/{feed}")).await, 204);
    let (_, left) = get_json("/api/articles").await;
    assert!(
        left.as_array().expect("array of articles").is_empty(),
        "favorited article goes away with its feed",
    );
}

// A favorite survives the cleaner whether read or not; unmarking makes it
// eligible again on the next run.
#[tokio::test]
async fn cleaner_keeps_favorites_until_unmarked() {
    let db = fresh_db().await;
    let feed = seed_feed(&db, "https://a.example/feed").await;
    let fav_read = seed_article(&db, &feed, "fav-read", 15).await;
    let fav_unread = seed_article(&db, &feed, "fav-unread", 15).await;
    assert_eq!(mark_favorite(&fav_read).await, 204);
    assert_eq!(mark_favorite(&fav_unread).await, 204);
    mark_read_in_db(&db, &fav_read).await;

    assert_eq!(run_cleaner().await, 200);
    let (_, left) = get_json("/api/articles").await;
    assert_eq!(titles(&left), ["fav-read", "fav-unread"]);

    assert_eq!(unmark_favorite(&fav_read).await, 204);
    assert_eq!(run_cleaner().await, 200);
    let (_, after_unmark) = get_json("/api/articles").await;
    assert_eq!(titles(&after_unmark), ["fav-unread"]);
}
