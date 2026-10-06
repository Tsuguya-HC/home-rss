//! #245: `decode_feed_row` が失敗列を含む 10 列を位置どおりに読むことを固定する。
//! `spin_sdk::pg::Row` に public な作り方がないため `Vec<DbValue>` で渡す。
//! 新 2 列の取り違え（順序・型）を捕まえる。

use home_rss_shared::fetch::decode_feed_row;
use spin_sdk::pg::DbValue;

#[test]
fn decodes_the_failure_columns_at_their_positions() {
    let row = vec![
        DbValue::Str("feed-id".to_owned()),
        DbValue::Str("https://example.com/feed".to_owned()),
        DbValue::Str("Example".to_owned()),
        DbValue::DbNull,
        DbValue::Str("etag-1".to_owned()),
        DbValue::DbNull,
        DbValue::Int64(1757894400),
        DbValue::Int64(1757808000),
        DbValue::Str("HTTP 404".to_owned()),
        DbValue::Int64(1757894400),
    ];
    let feed = decode_feed_row(&row).unwrap();
    assert_eq!(feed.id, "feed-id");
    assert_eq!(feed.url, "https://example.com/feed");
    assert_eq!(feed.title.as_deref(), Some("Example"));
    assert_eq!(feed.site_url, None);
    assert_eq!(feed.etag.as_deref(), Some("etag-1"));
    assert_eq!(feed.last_modified, None);
    assert_eq!(feed.last_fetched_at, Some(1757894400));
    assert_eq!(feed.created_at, Some(1757808000));
    assert_eq!(feed.last_fetch_error.as_deref(), Some("HTTP 404"));
    assert_eq!(feed.fetch_failing_since, Some(1757894400));
}

#[test]
fn decodes_null_failure_columns_as_no_failure() {
    let row = vec![
        DbValue::Str("feed-id".to_owned()),
        DbValue::Str("https://example.com/feed".to_owned()),
        DbValue::DbNull,
        DbValue::DbNull,
        DbValue::DbNull,
        DbValue::DbNull,
        DbValue::Int64(1757894400),
        DbValue::Int64(1757808000),
        DbValue::DbNull,
        DbValue::DbNull,
    ];
    let feed = decode_feed_row(&row).unwrap();
    assert_eq!(feed.last_fetch_error, None);
    assert_eq!(feed.fetch_failing_since, None);
}
