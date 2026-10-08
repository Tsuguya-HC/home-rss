//! `decode_feed_row` の失敗記録の列 (`last_fetch_error` /
//! `fetch_failing_since`) のデコードを固定する (#245)。`spin_sdk::pg::Row`
//! に公開コンストラクタが無いため `Vec<DbValue>` を渡す (`Index<usize,
//! Output = DbValue>` の受け口のおかげで呼び出し側の `&Row` はそのまま)。

use home_rss_shared::fetch::decode_feed_row;
use spin_sdk::pg::DbValue;

fn failure_row() -> Vec<DbValue> {
    vec![
        DbValue::Str("00000000-0000-0000-0000-000000000000".to_owned()),
        DbValue::Str("https://example.com/feed".to_owned()),
        DbValue::Str("Example".to_owned()),
        DbValue::DbNull,
        DbValue::DbNull,
        DbValue::DbNull,
        DbValue::Int64(1_700_000_000),
        DbValue::Int64(1_699_000_000),
        DbValue::Str("HTTP 404 fetching https://example.com/feed".to_owned()),
        DbValue::Int64(1_700_000_100),
    ]
}

#[test]
fn failure_record_columns_decode_in_column_order() {
    let feed = decode_feed_row(&failure_row()).expect("the row must decode");
    assert_eq!(
        feed.last_fetch_error.as_deref(),
        Some("HTTP 404 fetching https://example.com/feed"),
        "the ninth column must land in last_fetch_error"
    );
    assert_eq!(
        feed.fetch_failing_since,
        Some(1_700_000_100),
        "the tenth column must land in fetch_failing_since"
    );
}

#[test]
fn null_failure_record_columns_decode_as_none() {
    let mut row = failure_row();
    row[8] = DbValue::DbNull;
    row[9] = DbValue::DbNull;
    let feed = decode_feed_row(&row).expect("the row must decode");
    assert_eq!(
        feed.last_fetch_error, None,
        "a NULL ninth column must stay None"
    );
    assert_eq!(
        feed.fetch_failing_since, None,
        "a NULL tenth column must stay None"
    );
}
