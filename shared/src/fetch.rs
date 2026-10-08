use crate::feed::{ParsedFeed, parse_feed_bytes};
use crate::models::Feed;
use crate::ssrf::reject_internal_feed_url;
use anyhow::{Context, Result};
use spin_sdk::http::body::IncomingBodyExt;
use spin_sdk::http::{EmptyBody, Request, Response, StatusCode, send};
use spin_sdk::pg::{Connection, DbValue, Decode, ParameterValue};
use std::time::Duration;

/// POST /api/feeds の即時取得 (#106) と定期取得 (fetcher) で共有する
/// 「取得して保存する」処理。SSRF ガード（判定自体は `shared::ssrf`、#106 U4）・
/// 条件付き GET のヘッダ付与・304 の扱い・パース・articles INSERT・feeds
/// UPDATE (etag/last_modified 含む) を一箇所にまとめ、両経路の間で扱いが
/// 乖離しないようにする (#106 R2)。
#[derive(Debug)]
pub enum FetchAndStoreOutcome {
    /// 304 Not Modified。DB には触れていない。
    NotModified,
    /// 取得・保存に成功した。更新後の feed 行 (UPDATE ... RETURNING の結果) を返す。
    /// Box 化しているのは `Feed` の拡張 (#245) で enum 全体のサイズが膨らみ
    /// `large_enum_variant` に当たるため。`FetchFailed` 等の小さい variant を
    /// 返すたびに大きい `Feed` ぶんの領域をスタックに確保しないための措置。
    Stored(Box<Feed>),
    /// URL がスキーム/ポート/内部ホストのガードで弾かれた、到達不能・想定外
    /// ステータス・タイムアウトなど、取得自体の失敗。
    FetchFailed(anyhow::Error),
    /// 本文がフィードとしてパース不能。
    Unparseable(anyhow::Error),
    /// 取得には成功したが、articles/feeds への書き込みで失敗した。
    StoreFailed(anyhow::Error),
}

/// 取得だけを行い DB に触れない共有処理。`fetch_and_store` のうち外部 HTTP
/// とパースだけを切り出したもの。add_feed はこちらをトランザクションの外で
/// 呼び、保存だけを `in_transaction` の中に入れる。INSERT〜COMMIT の間ずっと
/// 行ロックを保持すると、fetcher・DELETE・同時追加を外部フェッチの時間だけ
/// ブロックする (#148 の差し戻し)。正規化済み URL を受け取る: ガード自体は
/// `fetch_and_store` の入口（#106 R2。add_feed は別に 400 を返す）が担う。
#[derive(Debug)]
pub enum FetchOnlyOutcome {
    /// 304 Not Modified。DB には触れていない。
    NotModified {
        etag: Option<String>,
        last_modified: Option<String>,
    },
    /// 取得・パースに成功した。`store_fetched` で保存する材料。
    Fetched {
        parsed: ParsedFeed,
        etag: Option<String>,
        last_modified: Option<String>,
    },
    /// 到達不能・想定外ステータス・タイムアウトなど、取得自体の失敗。
    FetchFailed(anyhow::Error),
    /// 本文がフィードとしてパース不能。
    Unparseable(anyhow::Error),
}

/// `url` は `reject_internal_feed_url` が返した正規化後の値 (#106 U7)。
/// `timeout` に `None` を渡すと打ち切らない（定期取得の既存動作をそのまま保つ）。
/// `Some(duration)` を渡すと send と本文読み取りのそれぞれに duration を課す
/// （追加直後取得のハング対策 #106。応答を返さないホストへの send は WASI の
/// outbound HTTP にタイムアウト API が無いため、放置すると無期限に待ち続ける。
/// 値は server/src/lib.rs の `FETCH_TIMEOUT` を参照）。
pub async fn fetch_only(
    url: &url::Url,
    etag: Option<&str>,
    last_modified: Option<&str>,
    timeout: Option<Duration>,
) -> FetchOnlyOutcome {
    let url = url.as_str();
    let mut builder = Request::get(url).header("user-agent", "home-rss-fetcher/0.1");
    if let Some(etag) = etag {
        builder = builder.header("if-none-match", etag);
    }
    if let Some(lm) = last_modified {
        builder = builder.header("if-modified-since", lm);
    }
    let req = match builder.body(EmptyBody::new()) {
        Ok(r) => r,
        Err(e) => {
            return FetchOnlyOutcome::FetchFailed(anyhow::anyhow!(
                "failed to build request for {url}: {e:#}"
            ));
        }
    };

    let resp: Response = match await_with_optional_timeout(timeout, send(req)).await {
        Some(Ok(r)) => r,
        Some(Err(e)) => {
            return FetchOnlyOutcome::FetchFailed(anyhow::anyhow!(
                "request failed for {url}: {e:#}"
            ));
        }
        None => {
            return FetchOnlyOutcome::FetchFailed(anyhow::anyhow!("request timed out for {url}"));
        }
    };

    match classify_status(resp.status()) {
        // last_fetched_at は本文を取得して保存した時刻 (#106 U5)。304 は
        // store_fetched() を呼ばずにここで早期リターンするので更新されない
        // （旧実装からの既存動作であり、この diff での退行ではない。issue
        // #106 (c) の要求により定期取得の挙動は変えない）。この列を
        // 「取得が滞っているフィード」の検出にそのまま使うと、304 が
        // 続いている（＝正常に最新のまま）フィードを誤検知するので注意。
        StatusOutcome::NotModified => {
            return FetchOnlyOutcome::NotModified {
                etag: header_string(&resp, "etag"),
                last_modified: header_string(&resp, "last-modified"),
            };
        }
        StatusOutcome::Unexpected(status) => {
            return FetchOnlyOutcome::FetchFailed(anyhow::anyhow!("HTTP {status} fetching {url}"));
        }
        StatusOutcome::Ok => {}
    }

    let new_etag = header_string(&resp, "etag");
    let new_last_modified = header_string(&resp, "last-modified");

    let body = match await_with_optional_timeout(timeout, resp.into_body().bytes()).await {
        Some(Ok(b)) => b,
        Some(Err(e)) => {
            return FetchOnlyOutcome::FetchFailed(anyhow::anyhow!(
                "failed to read body for {url}: {e:#}"
            ));
        }
        None => {
            return FetchOnlyOutcome::FetchFailed(anyhow::anyhow!(
                "timed out reading body for {url}"
            ));
        }
    };

    match parse_feed_bytes(body.as_ref()) {
        Ok(parsed) => FetchOnlyOutcome::Fetched {
            parsed,
            etag: new_etag,
            last_modified: new_last_modified,
        },
        Err(e) => FetchOnlyOutcome::Unparseable(anyhow::anyhow!(
            "feed at {url} could not be parsed: {e:#}"
        )),
    }
}

/// `fetch_only` が持ち帰ったパース結果の保存。記事 INSERT と `feeds` UPDATE
/// を行う。呼び出し側のトランザクションの中で呼ぶことを想定する。
/// 304（`FetchOnlyOutcome::NotModified`）は保存するものが無いのでここには
/// 来ない。呼び出し側で分岐すること。
pub async fn store_fetched(
    conn: &Connection,
    feed_id: &str,
    parsed: &ParsedFeed,
    etag: Option<&str>,
    last_modified: Option<&str>,
) -> Result<Feed> {
    store(conn, feed_id, parsed, etag, last_modified).await
}

/// `timeout` に `None` を渡すと打ち切らない（定期取得の既存動作をそのまま保つ）。
/// `Some(duration)` を渡すと send と本文読み取りのそれぞれに duration を課す
/// （追加直後取得のハング対策 #106。応答を返さないホストへの send は WASI の
/// outbound HTTP にタイムアウト API が無いため、放置すると無期限に待ち続ける。
/// 値は server/src/lib.rs の `FETCH_TIMEOUT` を参照）。
pub async fn fetch_and_store(
    conn: &Connection,
    feed_id: &str,
    url: &str,
    etag: Option<&str>,
    last_modified: Option<&str>,
    timeout: Option<Duration>,
) -> FetchAndStoreOutcome {
    // add_feed は事前にも同じ検査を呼んで 400 をすぐ返すが、ここでも検査するのは
    // DB に既に残っている行（定期取得・OPML インポート由来）が、検査導入前に
    // 登録された/後から内部ホストに変わった URL のまま永遠に叩かれ続けるのを
    // 防ぐため (#106 R2): リスクの本体は「追加時の1回」ではなく「定期取得が
    // 無検証で叩き続けること」。
    //
    // 検証済みの正規化後 URL (`url` を Url へシャドーする) を fetch に使う
    // (#106 U7): 生の入力文字列のままだと、Url::parse は通すが http::Uri
    // （実際に fetch に使う側）は拒否する先頭空白・末尾改行等がガードを
    // 素通りしたまま feeds 行だけ作られ、以後ここが永遠に失敗し続ける。
    let normalized = match reject_internal_feed_url(url) {
        Ok(normalized) => normalized,
        Err(reason) => {
            // "SSRF guard rejected" は Loki で検索できる固定文字列 (#106 U3)。
            // このメッセージはネットワークエラーや 404 と同じ eprintln! 経由で
            // ログに出るので、埋もれずに見つけられるようにここで固定しておく。
            return FetchAndStoreOutcome::FetchFailed(anyhow::anyhow!(
                "SSRF guard rejected feed {feed_id}: {url} ({reason})"
            ));
        }
    };

    match fetch_only(&normalized, etag, last_modified, timeout).await {
        FetchOnlyOutcome::NotModified { .. } => FetchAndStoreOutcome::NotModified,
        FetchOnlyOutcome::FetchFailed(e) => FetchAndStoreOutcome::FetchFailed(e),
        FetchOnlyOutcome::Unparseable(e) => FetchAndStoreOutcome::Unparseable(e),
        FetchOnlyOutcome::Fetched {
            parsed,
            etag,
            last_modified,
        } => match store(
            conn,
            feed_id,
            &parsed,
            etag.as_deref(),
            last_modified.as_deref(),
        )
        .await
        {
            Ok(feed) => FetchAndStoreOutcome::Stored(Box::new(feed)),
            Err(e) => FetchAndStoreOutcome::StoreFailed(e),
        },
    }
}

async fn store(
    conn: &Connection,
    feed_id: &str,
    parsed: &ParsedFeed,
    etag: Option<&str>,
    last_modified: Option<&str>,
) -> Result<Feed> {
    // N 件の個別 INSERT ではなく UNNEST で 1 回のラウンドトリップにまとめる
    // (#106 R9)。実測: 50 件で個別 INSERT 約42ms → UNNEST 一括 約3.2ms。
    //
    // 注意 (#106 U6): 個別 INSERT ループ（失敗前のエントリは残る）から単一文
    // に変えたことで、部分失敗時の挙動が変わっている。ON CONFLICT DO NOTHING
    // の対象外のエラー（制約違反等）が起きると、このバッチの記事が全滅しうる。
    // バッチ内に同一 URL の重複があっても ON CONFLICT DO NOTHING で正しく
    // 排除されることは実測済み (PostgreSQL 17)。
    if !parsed.entries.is_empty() {
        let urls: Vec<Option<String>> =
            parsed.entries.iter().map(|e| Some(e.url.clone())).collect();
        let titles: Vec<Option<String>> = parsed
            .entries
            .iter()
            .map(|e| Some(e.title.clone()))
            .collect();
        let contents: Vec<Option<String>> =
            parsed.entries.iter().map(|e| e.content.clone()).collect();
        let authors: Vec<Option<String>> =
            parsed.entries.iter().map(|e| e.author.clone()).collect();
        let published_ats: Vec<Option<String>> = parsed
            .entries
            .iter()
            .map(|e| e.published_at.clone())
            .collect();
        let image_urls: Vec<Option<String>> =
            parsed.entries.iter().map(|e| e.image_url.clone()).collect();

        // UNNEST は各配列を独立に展開するので、長さが揃っていないと短い方が
        // NULL 埋めされずに行数がズレる。現状はすべて同じ entries から生成
        // しているので一致するが、将来 filter が入ると静かに壊れる (#106 U6)。
        debug_assert_eq!(urls.len(), titles.len());
        debug_assert_eq!(urls.len(), contents.len());
        debug_assert_eq!(urls.len(), authors.len());
        debug_assert_eq!(urls.len(), published_ats.len());
        debug_assert_eq!(urls.len(), image_urls.len());

        conn.execute(
            "INSERT INTO articles (feed_id, url, title, content, author, published_at, image_url) \
             SELECT $1, u.url, u.title, u.content, u.author, u.published_at::timestamptz, u.image_url \
             FROM UNNEST($2::text[], $3::text[], $4::text[], $5::text[], $6::text[], $7::text[]) \
             AS u(url, title, content, author, published_at, image_url) \
             ON CONFLICT DO NOTHING",
            vec![
                ParameterValue::Uuid(feed_id.to_owned()),
                urls.into(),
                titles.into(),
                contents.into(),
                authors.into(),
                published_ats.into(),
                image_urls.into(),
            ],
        )
        .await
        .with_context(|| format!("failed to insert articles for feed {feed_id}"))?;
    }

    let rows = conn
        .query(
            "UPDATE feeds SET title = $1, site_url = $2, etag = $3, last_modified = $4, \
             last_fetched_at = NOW() WHERE id = $5 \
             RETURNING id::text, url, title, site_url, etag, last_modified, \
             EXTRACT(EPOCH FROM last_fetched_at)::bigint, \
             EXTRACT(EPOCH FROM created_at)::bigint, last_fetch_error, \
             EXTRACT(EPOCH FROM fetch_failing_since)::bigint",
            vec![
                parsed.title.clone().into(),
                parsed.site_url.clone().into(),
                etag.map(str::to_owned).into(),
                last_modified.map(str::to_owned).into(),
                ParameterValue::Uuid(feed_id.to_owned()),
            ],
        )
        .await
        .with_context(|| format!("failed to update feed {feed_id}"))?
        .collect()
        .await
        .with_context(|| format!("failed to collect updated feed {feed_id}"))?;

    match rows.first() {
        Some(row) => {
            decode_feed_row(row).with_context(|| format!("failed to decode updated feed {feed_id}"))
        }
        None => anyhow::bail!("UPDATE feeds RETURNING returned no rows for feed {feed_id}"),
    }
}

/// `SELECT id::text, url, title, site_url, etag, last_modified, \
///  EXTRACT(EPOCH FROM last_fetched_at)::bigint, EXTRACT(EPOCH FROM created_at)::bigint, \
///  last_fetch_error, EXTRACT(EPOCH FROM fetch_failing_since)::bigint`
/// の列順に対応する行デコード。取得+保存の共有処理とここ (server の一覧/追加系
/// クエリ) の両方から使う (#106)。永続化に依存しない DTO である
/// `shared::models` を汚さないよう、ここ (feed feature 配下) に置く (#106 R6)。
/// `&Row` ではなく `Index` で受けるのは、`Row` に公開コンストラクタが無く
/// 単体テストで構築できないため (`Vec<DbValue>` を渡して全列のデコードを
/// 固定する。#245)。呼び出し側の `&Row` はそのまま渡せる。
pub fn decode_feed_row(row: &impl std::ops::Index<usize, Output = DbValue>) -> Result<Feed> {
    Ok(Feed {
        id: String::decode(&row[0])?,
        url: String::decode(&row[1])?,
        title: Option::<String>::decode(&row[2])?,
        site_url: Option::<String>::decode(&row[3])?,
        etag: Option::<String>::decode(&row[4])?,
        last_modified: Option::<String>::decode(&row[5])?,
        last_fetched_at: Option::<i64>::decode(&row[6])?,
        created_at: Option::<i64>::decode(&row[7])?,
        last_fetch_error: Option::<String>::decode(&row[8])?,
        fetch_failing_since: Option::<i64>::decode(&row[9])?,
    })
}

/// レスポンスステータスの分類。取得成功/未更新/失敗のいずれかに写す純粋関数。
#[derive(Debug, PartialEq, Eq)]
pub enum StatusOutcome {
    Ok,
    NotModified,
    Unexpected(StatusCode),
}

pub fn classify_status(status: StatusCode) -> StatusOutcome {
    match status {
        StatusCode::OK => StatusOutcome::Ok,
        StatusCode::NOT_MODIFIED => StatusOutcome::NotModified,
        other => StatusOutcome::Unexpected(other),
    }
}

/// 定期取得の 1 回分の結果を、失敗の記録への振る舞いに振り分ける (#245)。
/// `ever_fetched` はそのフィードが過去に 1 度でも取得済みか
/// (`last_fetched_at` が NULL でないか)。OPML 直後などの未取得フィードには
/// 印を付けない。`Record` が運ぶ文字列は直近の失敗理由で 200 文字で切る。
#[derive(Debug, PartialEq, Eq)]
pub enum FetchFailureAction {
    Record(String),
    Clear,
    Keep,
}

pub fn classify_fetch_failure(
    outcome: &FetchAndStoreOutcome,
    ever_fetched: bool,
) -> FetchFailureAction {
    match outcome {
        FetchAndStoreOutcome::Stored(_) | FetchAndStoreOutcome::NotModified => {
            FetchFailureAction::Clear
        }
        FetchAndStoreOutcome::StoreFailed(_) => FetchFailureAction::Keep,
        FetchAndStoreOutcome::FetchFailed(e) | FetchAndStoreOutcome::Unparseable(e) => {
            if ever_fetched {
                FetchFailureAction::Record(truncate_reason(&format!("{e:#}")))
            } else {
                FetchFailureAction::Keep
            }
        }
    }
}

fn truncate_reason(reason: &str) -> String {
    reason.chars().take(200).collect()
}

/// `timeout` が `None` なら `select!` を経由せず素直に `fut` を待つ（「タイムアウト
/// なし」の意図とコードを一致させる、#106 R14）。`Some(d)` のときだけ
/// `with_timeout` でレースする。
async fn await_with_optional_timeout<T>(
    timeout: Option<Duration>,
    fut: impl std::future::Future<Output = T>,
) -> Option<T> {
    match timeout {
        Some(d) => with_timeout(spin_sdk::time::sleep(d), fut).await,
        None => Some(fut.await),
    }
}

/// `fut` が `timeout` より先に解決すれば `Some`、`timeout` が先に解決すれば `None`。
/// spin_sdk の実 I/O から切り離された自己完結な関数なので、任意の Future を渡して
/// 単体テストできる。
pub async fn with_timeout<T>(
    timeout: impl std::future::Future<Output = ()>,
    fut: impl std::future::Future<Output = T>,
) -> Option<T> {
    use futures_util::FutureExt;
    futures_util::select! {
        result = fut.fuse() => Some(result),
        _ = timeout.fuse() => None,
    }
}

fn header_string(resp: &Response, name: &str) -> Option<String> {
    resp.headers()
        .get(name)
        .and_then(|v| v.to_str().ok())
        .map(str::to_owned)
}

#[cfg(test)]
mod tests {
    use super::{StatusOutcome, await_with_optional_timeout, classify_status, with_timeout};
    use futures_util::FutureExt;
    use spin_sdk::http::StatusCode;

    #[test]
    fn classifies_ok_not_modified_and_unexpected() {
        assert_eq!(classify_status(StatusCode::OK), StatusOutcome::Ok);
        assert_eq!(
            classify_status(StatusCode::NOT_MODIFIED),
            StatusOutcome::NotModified
        );
        assert_eq!(
            classify_status(StatusCode::NOT_FOUND),
            StatusOutcome::Unexpected(StatusCode::NOT_FOUND)
        );
    }

    #[test]
    fn returns_some_when_future_resolves_before_timeout() {
        let result = with_timeout(std::future::pending::<()>(), async { 42 }).now_or_never();
        assert_eq!(result, Some(Some(42)));
    }

    #[test]
    fn returns_none_when_timeout_resolves_first() {
        let result = with_timeout(async {}, std::future::pending::<i32>()).now_or_never();
        assert_eq!(result, Some(None));
    }

    #[test]
    fn none_timeout_returns_the_future_result_without_racing() {
        // timeout: None（定期取得の既定）は select! を経由せず fut をそのまま
        // 待つはずで、fut の結果が Some に包まれて返る (#106 U1)。ここが
        // 崩れると定期取得が常に FetchFailed になる。
        let result = await_with_optional_timeout(None, async { 42 }).now_or_never();
        assert_eq!(result, Some(Some(42)));
    }
}
