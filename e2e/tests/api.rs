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

async fn mark_favorite_in_db(db: &Client, article_id: &str) {
    db.execute(
        "INSERT INTO favorites (article_id) VALUES ($1::text::uuid)",
        &[&article_id],
    )
    .await
    .expect("insert favorites");
}

async fn run_cleaner() {
    let resp = reqwest::Client::new()
        .post(format!("{}/clean", env("E2E_CLEANER_URL")))
        .send()
        .await
        .expect("POST /clean");
    assert_eq!(resp.status().as_u16(), 200);
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

    run_cleaner().await;

    let (_, left) = get_json("/api/articles").await;
    assert_eq!(titles(&left), ["old-unread", "recent-read"]);
}

#[tokio::test]
async fn marking_favorite_is_idempotent_and_unmarking_removes_it() {
    let db = fresh_db().await;
    let feed = seed_feed(&db, "https://a.example/feed").await;
    let article = seed_article(&db, &feed, "fav", 1).await;

    // ON CONFLICT が無いと 2 回目の mark が主キー違反で落ちる。
    assert_eq!(post(&format!("/api/articles/{article}/favorite"), None).await, 204);
    assert_eq!(post(&format!("/api/articles/{article}/favorite"), None).await, 204);
    assert_eq!(count(&db, "SELECT COUNT(*) FROM favorites").await, 1);

    let (_, only_favs) = get_json("/api/articles?favorite=true").await;
    assert_eq!(titles(&only_favs), ["fav"]);

    // 印が無い削除で 404 を返す書き方だと 2 回目の unmark で落ちる。
    assert_eq!(delete(&format!("/api/articles/{article}/favorite")).await, 204);
    assert_eq!(delete(&format!("/api/articles/{article}/favorite")).await, 204);
    assert_eq!(count(&db, "SELECT COUNT(*) FROM favorites").await, 0);

    let (_, no_favs) = get_json("/api/articles?favorite=true").await;
    assert_eq!(titles(&no_favs), Vec::<&str>::new());

    // INSERT ... ON CONFLICT DO NOTHING は存在しない article_id の FK 違反を
    // 吸収しないので、存在確認を挟まない実装ではここが 500 になる (#152)。
    let missing = "00000000-0000-0000-0000-000000000000";
    assert_eq!(post(&format!("/api/articles/{missing}/favorite"), None).await, 404);
}

#[tokio::test]
async fn favorites_only_filter_combines_with_feed_and_unread() {
    let db = fresh_db().await;
    let a = seed_feed(&db, "https://a.example/feed").await;
    let b = seed_feed(&db, "https://b.example/feed").await;
    let a_fav_read = seed_article(&db, &a, "a-fav-read", 1).await;
    let a_fav_unread = seed_article(&db, &a, "a-fav-unread", 1).await;
    seed_article(&db, &a, "a-plain-unread", 1).await;
    let b_fav_unread = seed_article(&db, &b, "b-fav-unread", 1).await;
    mark_read_in_db(&db, &a_fav_read).await;
    mark_favorite_in_db(&db, &a_fav_read).await;
    mark_favorite_in_db(&db, &a_fav_unread).await;
    mark_favorite_in_db(&db, &b_fav_unread).await;

    let (_, favs) = get_json("/api/articles?favorite=true").await;
    assert_eq!(titles(&favs), ["a-fav-read", "a-fav-unread", "b-fav-unread"]);

    let (_, favs_of_a) = get_json(&format!("/api/articles?feed_id={a}&favorite=true")).await;
    assert_eq!(titles(&favs_of_a), ["a-fav-read", "a-fav-unread"]);

    // favorite の絞り込みが unread と独立でない（unread 側で除外を忘れた）
    // 実装だとここが 3 件になる。
    let (_, fav_unread) = get_json("/api/articles?favorite=true&unread=true").await;
    assert_eq!(titles(&fav_unread), ["a-fav-unread", "b-fav-unread"]);

    let (_, fav_unread_of_a) =
        get_json(&format!("/api/articles?feed_id={a}&favorite=true&unread=true")).await;
    assert_eq!(titles(&fav_unread_of_a), ["a-fav-unread"]);
}

#[tokio::test]
async fn cleaner_keeps_favorites_and_unmarking_makes_them_eligible_again() {
    let db = fresh_db().await;
    let feed = seed_feed(&db, "https://a.example/feed").await;
    // 15 days is past the e2e retention (10), matching the existing cleaner test.
    let fav_read = seed_article(&db, &feed, "fav-read", 15).await;
    let fav_unread = seed_article(&db, &feed, "fav-unread", 15).await;
    let plain_read = seed_article(&db, &feed, "plain-read", 15).await;
    mark_read_in_db(&db, &fav_read).await;
    mark_read_in_db(&db, &plain_read).await;
    mark_favorite_in_db(&db, &fav_read).await;
    mark_favorite_in_db(&db, &fav_unread).await;

    // favorites 除外の無い削除 SQL では fav-read が消えて old 相当だけ残る。
    run_cleaner().await;
    let (_, left) = get_json("/api/articles").await;
    assert_eq!(titles(&left), ["fav-read", "fav-unread"]);

    // unmark 後も除外条件が残る書き方だと 2 回目でも消えない。
    assert_eq!(delete(&format!("/api/articles/{fav_read}/favorite")).await, 204);
    run_cleaner().await;
    let (_, left_again) = get_json("/api/articles").await;
    assert_eq!(titles(&left_again), ["fav-unread"]);
}

#[tokio::test]
async fn deleting_a_feed_removes_favorites_with_its_articles() {
    let db = fresh_db().await;
    let feed = seed_feed(&db, "https://a.example/feed").await;
    let article = seed_article(&db, &feed, "one", 1).await;
    mark_favorite_in_db(&db, &article).await;

    // ON DELETE CASCADE が無いとここで favorites 行が孤児として残る。
    assert_eq!(delete(&format!("/api/feeds/{feed}")).await, 204);
    assert_eq!(count(&db, "SELECT COUNT(*) FROM articles").await, 0);
    assert_eq!(count(&db, "SELECT COUNT(*) FROM favorites").await, 0);
}

#[tokio::test]
async fn marking_favorite_with_malformed_uuid_is_a_server_error() {
    let _db = fresh_db().await;
    // 形式不正の UUID は記事の有無の問題ではなく DB 層での変換失敗なので、
    // mark_read / delete_feed と同じく 500 になるべき。今の mark_favorite は
    // article_exists の Err を 404 に丸めているのでここが 404 になって落ちる。
    assert_eq!(post("/api/articles/abc/favorite", None).await, 500);
}

#[tokio::test]
async fn marking_favorite_racing_feed_deletion_is_not_a_server_error() {
    let db = fresh_db().await;
    // 存在確認を通過した直後に記事が消えると INSERT が FK 違反で 500 になる
    // 実装では、競合が起きた周回でここが 500 になって落ちる (#152)。
    for i in 0..30 {
        let feed = seed_feed(&db, &format!("https://race.example/{i}/feed")).await;
        let article = seed_article(&db, &feed, &format!("race-{i}"), 1).await;
        let fav_url = server(&format!("/api/articles/{article}/favorite"));
        let del_url = server(&format!("/api/feeds/{feed}"));
        // favorite を先に投げ、削除をすぐ後ろに重ねる。競合しなかった周回は
        // 204（favorite が先）か 404（削除が先）になり、次の周回で試す。
        let (fav, del) = tokio::join!(
            reqwest::Client::new().post(fav_url).send(),
            reqwest::Client::new().delete(del_url).send(),
        );
        assert_eq!(del.expect("DELETE feed").status().as_u16(), 204);
        assert_ne!(
            fav.expect("POST favorite").status().as_u16(),
            500,
            "article deleted mid-request must be 404, not 500"
        );
    }
}

#[tokio::test]
async fn article_list_reports_which_articles_are_favorites() {
    let db = fresh_db().await;
    let feed = seed_feed(&db, "https://a.example/feed").await;
    let fav = seed_article(&db, &feed, "fav", 1).await;
    seed_article(&db, &feed, "plain", 1).await;
    mark_favorite_in_db(&db, &fav).await;

    // EXISTS 式を忘れて常に false を返す実装だと、印の表示もお気に入りのみの
    // 絞り込み後の表示もずれる。
    let (_, body) = get_json("/api/articles").await;
    let flags: Vec<(&str, bool)> = body
        .as_array()
        .expect("array of articles")
        .iter()
        .map(|a| {
            (
                a["title"].as_str().expect("title"),
                a["is_favorite"].as_bool().expect("is_favorite"),
            )
        })
        .collect();
    assert!(flags.contains(&("fav", true)), "{flags:?}");
    assert!(flags.contains(&("plain", false)), "{flags:?}");
}
