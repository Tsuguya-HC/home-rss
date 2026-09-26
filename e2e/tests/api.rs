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

async fn mark_favorite_in_db(db: &Client, article_id: &str) {
    db.execute(
        "INSERT INTO favorites (article_id) VALUES ($1::text::uuid)",
        &[&article_id],
    )
    .await
    .expect("insert favorites");
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
async fn marking_favorite_is_idempotent_both_ways() {
    let db = fresh_db().await;
    let feed = seed_feed(&db, "https://a.example/feed").await;
    let article = seed_article(&db, &feed, "one", 1).await;

    assert_eq!(post(&format!("/api/articles/{article}/favorite"), None).await, 204);
    assert_eq!(post(&format!("/api/articles/{article}/favorite"), None).await, 204);
    assert_eq!(count(&db, "SELECT COUNT(*) FROM favorites").await, 1);

    let (_, listed) = get_json("/api/articles").await;
    let items = listed.as_array().expect("array of articles");
    assert_eq!(items.len(), 1);
    // The mark is visible in the list payload (and the detail payload serves the same row).
    assert_eq!(items[0]["is_favorite"], true);

    assert_eq!(delete(&format!("/api/articles/{article}/favorite")).await, 204);
    assert_eq!(delete(&format!("/api/articles/{article}/favorite")).await, 204);
    assert_eq!(count(&db, "SELECT COUNT(*) FROM favorites").await, 0);

    let (_, listed) = get_json("/api/articles").await;
    let items = listed.as_array().expect("array of articles");
    assert_eq!(items[0]["is_favorite"], false);
}

#[tokio::test]
async fn marking_favorite_deleted_mid_request_returns_not_found() {
    let db = fresh_db().await;
    let feed = seed_feed(&db, "https://a.example/feed").await;
    let article = seed_article(&db, &feed, "one", 1).await;

    // Block the server's INSERT INTO favorites with a trigger that waits on
    // an advisory lock we hold, so the delete lands between its existence
    // check and its INSERT without any timing guesswork. This test never
    // inserts into favorites itself, so the trigger needs no exclusion.
    db.batch_execute(
        "DROP TRIGGER IF EXISTS e2e_hold_favorite_insert ON favorites; \
         CREATE OR REPLACE FUNCTION e2e_hold_favorite_insert() RETURNS trigger \
         LANGUAGE plpgsql AS $$BEGIN \
           PERFORM pg_advisory_xact_lock(424242, 1); \
           RETURN NEW; END$$; \
         CREATE TRIGGER e2e_hold_favorite_insert BEFORE INSERT ON favorites \
         FOR EACH ROW EXECUTE FUNCTION e2e_hold_favorite_insert(); \
         SELECT pg_advisory_lock(424242, 1)",
    )
    .await
    .expect("install blocking trigger and take lock");

    let path = format!("/api/articles/{article}/favorite");
    let favorite = tokio::spawn(async move { post(&path, None).await });
    // Wait until the POST is inside the trigger (blocked on our lock); a
    // fixed sleep would let it finish before the delete or fire too early.
    // The polling query mentions favorites too, so our own backend is excluded.
    for _ in 0..2000 {
        let n: i64 = db
            .query_one(
                "SELECT COUNT(*) FROM pg_stat_activity \
                 WHERE pid <> pg_backend_pid() AND state = 'active' \
                 AND query LIKE '%INSERT INTO favorites%'",
                &[],
            )
            .await
            .expect("poll pg_stat_activity")
            .get(0);
        if n > 0 {
            break;
        }
        tokio::task::yield_now().await;
    }
    // Delete the row the existence check saw, so the unblocked INSERT hits
    // the favorites.article_id foreign key. That must surface as 404, not 500.
    db.execute(
        "DELETE FROM articles WHERE id = $1::text::uuid",
        &[&article],
    )
    .await
    .expect("delete article mid-request");
    db.batch_execute(
        "SELECT pg_advisory_unlock(424242, 1); \
         DROP TRIGGER e2e_hold_favorite_insert ON favorites; \
         DROP FUNCTION e2e_hold_favorite_insert()",
    )
    .await
    .expect("release lock and drop trigger");

    assert_eq!(favorite.await.expect("favorite response"), 404);
}

#[tokio::test]
async fn marking_a_missing_article_as_favorite_returns_not_found() {
    let db = fresh_db().await;
    let feed = seed_feed(&db, "https://a.example/feed").await;
    let article = seed_article(&db, &feed, "one", 1).await;

    assert_eq!(post(&format!("/api/articles/{article}/favorite"), None).await, 204);

    let missing = "00000000-0000-0000-0000-000000000000";
    // A missing article must surface as 404.
    assert_eq!(post(&format!("/api/articles/{missing}/favorite"), None).await, 404);
    assert_eq!(count(&db, "SELECT COUNT(*) FROM favorites").await, 1);
}

#[tokio::test]
async fn article_list_filters_favorites_combined_with_feed_and_unread() {
    let db = fresh_db().await;
    let a = seed_feed(&db, "https://a.example/feed").await;
    let b = seed_feed(&db, "https://b.example/feed").await;
    let a_fav_unread = seed_article(&db, &a, "a-fav-unread", 1).await;
    let a_fav_read = seed_article(&db, &a, "a-fav-read", 1).await;
    seed_article(&db, &a, "a-plain-unread", 1).await;
    let b_fav_read = seed_article(&db, &b, "b-fav-read", 1).await;
    mark_favorite_in_db(&db, &a_fav_unread).await;
    mark_favorite_in_db(&db, &a_fav_read).await;
    mark_favorite_in_db(&db, &b_fav_read).await;
    mark_read_in_db(&db, &a_fav_read).await;
    mark_read_in_db(&db, &b_fav_read).await;

    let (_, favs) = get_json("/api/articles?favorite=true").await;
    assert_eq!(titles(&favs), ["a-fav-read", "a-fav-unread", "b-fav-read"]);

    let (_, favs_of_a) = get_json(&format!("/api/articles?feed_id={a}&favorite=true")).await;
    assert_eq!(titles(&favs_of_a), ["a-fav-read", "a-fav-unread"]);

    let (_, unread_favs) = get_json("/api/articles?unread=true&favorite=true").await;
    assert_eq!(titles(&unread_favs), ["a-fav-unread"]);

    let (_, unread_favs_of_a) =
        get_json(&format!("/api/articles?feed_id={a}&unread=true&favorite=true")).await;
    assert_eq!(titles(&unread_favs_of_a), ["a-fav-unread"]);
}

#[tokio::test]
async fn deleting_a_feed_removes_favorites_with_its_articles() {
    let db = fresh_db().await;
    let feed = seed_feed(&db, "https://a.example/feed").await;
    let article = seed_article(&db, &feed, "one", 1).await;
    mark_favorite_in_db(&db, &article).await;

    assert_eq!(delete(&format!("/api/feeds/{feed}")).await, 204);
    assert_eq!(count(&db, "SELECT COUNT(*) FROM articles").await, 0);
    assert_eq!(count(&db, "SELECT COUNT(*) FROM favorites").await, 0);
}

#[tokio::test]
async fn cleaner_keeps_favorites_read_or_not_and_unmarking_reenlists() {
    let db = fresh_db().await;
    let feed = seed_feed(&db, "https://a.example/feed").await;
    let fav_read = seed_article(&db, &feed, "fav-read", 15).await;
    let fav_unread = seed_article(&db, &feed, "fav-unread", 15).await;
    seed_article(&db, &feed, "plain-unread", 15).await;
    mark_read_in_db(&db, &fav_read).await;
    mark_favorite_in_db(&db, &fav_read).await;
    mark_favorite_in_db(&db, &fav_unread).await;

    let cleaner = || {
        reqwest::Client::new()
            .post(format!("{}/clean", env("E2E_CLEANER_URL")))
            .send()
    };
    assert_eq!(cleaner().await.expect("POST /clean").status().as_u16(), 200);

    // Favorites survive the cleaner whether read or not; unmarking makes
    // the article eligible again on the next run.
    let (_, left) = get_json("/api/articles").await;
    assert_eq!(titles(&left), ["fav-read", "fav-unread", "plain-unread"]);

    assert_eq!(delete(&format!("/api/articles/{fav_read}/favorite")).await, 204);
    assert_eq!(cleaner().await.expect("POST /clean").status().as_u16(), 200);

    let (_, left) = get_json("/api/articles").await;
    assert_eq!(titles(&left), ["fav-unread", "plain-unread"]);
}

#[tokio::test]
async fn cleaner_running_while_an_article_is_favorited_keeps_that_article() {
    // 採用 #1 が捕まえる変異: cleaner の DELETE は文開始時のスナップショットで
    // NOT EXISTS (favorites) を評価する。直前の仕分けがこの Pod で実行して確かめた
    // 通り、文が動き始めた後に付けたお気に入りは DELETE には見えず、204 を返した
    // 直後に記事ごと消える。articles への BEFORE DELETE 文トリガで DELETE 本体を
    // 数秒止め、その隙に POST /favorite を成功させてから再開させるので、競合の
    // 窓はタイミングの推測ではなくトリガが作る。このテストは今落ちる（204 の後で
    // 両テーブルとも空になる）。
    let db = fresh_db().await;
    let feed = seed_feed(&db, "https://a.example/feed").await;
    let article = seed_article(&db, &feed, "raced", 15).await;
    mark_read_in_db(&db, &article).await;

    // 文のスナップショットはトリガより先に確定する（この Pod で psql から実測）。
    // pg_sleep は行の顔ぶれを変えないので、止めるだけで結果は変えないはずだが、
    // 今の実装は止めている間の INSERT を見ずに消す。トリガは TRUNCATE では
    // 消えないので、作る前にも消してから作り（前の失敗の残骸を拾わない）、
    // 最後にも必ず外す。
    db.batch_execute(
        "DROP TRIGGER IF EXISTS e2e_hold_article_delete ON articles; \
         DROP FUNCTION IF EXISTS e2e_hold_article_delete(); \
         CREATE FUNCTION e2e_hold_article_delete() RETURNS trigger \
         LANGUAGE plpgsql AS $$BEGIN PERFORM pg_sleep(4); RETURN NULL; END$$; \
         CREATE TRIGGER e2e_hold_article_delete BEFORE DELETE ON articles \
         FOR EACH STATEMENT EXECUTE FUNCTION e2e_hold_article_delete()",
    )
    .await
    .expect("install delete-holding trigger");

    let clean = tokio::spawn({
        let url = format!("{}/clean", env("E2E_CLEANER_URL"));
        async move {
            reqwest::Client::new()
                .post(url)
                .send()
                .await
                .expect("POST /clean")
                .status()
                .as_u16()
        }
    });
    // DELETE がトリガの中で止まっている（＝文が始まりスナップショットが確定した
    // 後）のを pg_stat_activity で確認してから POST する。固定の sleep では
    // DELETE がまだ始まっていない／もう終わっている競合が残る。自分自身は除き、
    // この問い合わせ自体は articles に触れない。
    let mut saw_delete = false;
    for _ in 0..2000 {
        let n: i64 = db
            .query_one(
                "SELECT COUNT(*) FROM pg_stat_activity \
                 WHERE pid <> pg_backend_pid() AND state = 'active' \
                 AND query LIKE '%DELETE FROM articles%'",
                &[],
            )
            .await
            .expect("poll pg_stat_activity")
            .get(0);
        if n > 0 {
            saw_delete = true;
            break;
        }
        tokio::task::yield_now().await;
    }
    // DELETE がトリガに入る前に終わる競合では、競合を再現できていないので
    // 誤って通さず落とす。
    assert!(saw_delete, "cleaner DELETE never entered the delete trigger");
    // お気に入り登録は DELETE の実行中（スナップショット確定後）に成功する。
    assert_eq!(
        post(&format!("/api/articles/{article}/favorite"), None).await,
        204
    );
    assert_eq!(clean.await.expect("cleaner response"), 200);
    db.batch_execute(
        "DROP TRIGGER e2e_hold_article_delete ON articles; \
         DROP FUNCTION e2e_hold_article_delete()",
    )
    .await
    .expect("drop delete-holding trigger");
    // 直った実装はこの競合でも記事を消さない。今の実装は消すので落ちる。
    assert_eq!(count(&db, "SELECT COUNT(*) FROM articles").await, 1);
    assert_eq!(count(&db, "SELECT COUNT(*) FROM favorites").await, 1);
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
