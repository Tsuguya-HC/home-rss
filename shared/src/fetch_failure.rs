//! #245: fetcher の定期取得の 1 回分を「記録する / 消す / 変えない」に振り分ける純粋関数。
//! Spin の中にしか無い値 (pg の `Connection`、受け取った `Response`) を引数に取らないので単体テストできる。
//! SQL の組み立てと実行は fetcher 側 (`process_feed` の独立の `UPDATE feeds`) に残る。

/// 定期取得の 1 回分の結果。`fetch_and_store` の `FetchAndStoreOutcome` に対応するが、
/// `anyhow::Error` の中身ではなく種類だけを見る。種類で分けないと、文字列の
/// 書き換えで分岐が壊れてもテストが気づけない。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ScheduledFetchOutcome {
    FetchFailed(FetchFailureKind),
    Unparseable,
    Stored,
    NotModified,
    StoreFailed,
}

/// `FetchFailed` の内訳。送信・受信のエラー、URL ガードでの拒否、リクエストの
/// 組み立て失敗、200 / 304 以外の応答 (3xx を含む) を区別する。メッセージ付きの
/// 種類は短い詳細をそのまま運び、記録時に 200 文字で切る。
///
/// `fetch_and_store` は `FetchFailed(anyhow::Error)` の文字列しか返さないので、
/// fetcher は `scheduled_outcome_of` で文字列から種類に戻す。文字列の書き換えで
/// 分類が壊れたら `scheduled_outcome_of` の単体テストが落ちる。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FetchFailureKind {
    GuardRejected(String),
    RequestBuild(String),
    Send(String),
    UnexpectedStatus(u16),
    BodyRead(String),
}

/// 振り分けの結果。`Record` は失敗の列を書く、`Clear` は成功で NULL に戻す、
/// `Keep` は `StoreFailed` と未取得のときの「触れない」。`Record` の理由は
/// 200 文字で切った状態で運ぶ (#245)。変換（分類・切り詰め）と判定を同じ
/// 関数にまとめる: 呼び出し側で変換を忘れると長い文が DB に残るので、
/// 振り分けの結果としては切った後の文字列だけを出す。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FailureAction {
    Record { reason: String },
    Clear,
    Keep,
}

pub fn decide_failure_action(outcome: &ScheduledFetchOutcome, ever_fetched: bool) -> FailureAction {
    match outcome {
        ScheduledFetchOutcome::FetchFailed(kind) if ever_fetched => FailureAction::Record {
            reason: truncate_reason(&kind.reason()),
        },
        ScheduledFetchOutcome::Unparseable if ever_fetched => FailureAction::Record {
            reason: truncate_reason("フィードとして解釈できない"),
        },
        ScheduledFetchOutcome::Stored | ScheduledFetchOutcome::NotModified => FailureAction::Clear,
        _ => FailureAction::Keep,
    }
}

/// 失敗理由を 200 文字で切る。文字数であってバイト数ではない (日本語の理由で
/// バイト切りすると境界で文字化けする)。
pub fn truncate_reason(reason: &str) -> String {
    reason.chars().take(200).collect()
}

impl FetchFailureKind {
    fn reason(&self) -> String {
        match self {
            FetchFailureKind::GuardRejected(detail) => {
                format!("URL ガードで拒否: {detail}")
            }
            FetchFailureKind::RequestBuild(detail) => {
                format!("リクエストの組み立て失敗: {detail}")
            }
            FetchFailureKind::Send(detail) => format!("送信エラー: {detail}"),
            FetchFailureKind::UnexpectedStatus(status) => format!("HTTP {status}"),
            FetchFailureKind::BodyRead(detail) => format!("受信エラー: {detail}"),
        }
    }
}

/// `fetch_and_store` が返した `FetchFailed` のエラーメッセージを種類に戻す。
/// `anyhow::Error` の中身は型で運べないので、組み立て側 (`fetch.rs`) の文面の
/// 接頭辞で分類する。組み立て側の文面を変えたらこの分類も変える必要が
/// あることは、ここと下の単体テストが使う文字列で固定する。
pub fn classify_fetch_error(message: &str) -> FetchFailureKind {
    if let Some(detail) = message.strip_prefix("SSRF guard rejected feed") {
        let detail = detail.trim_start_matches([':', ' ']).to_owned();
        FetchFailureKind::GuardRejected(detail)
    } else if let Some(detail) = message.strip_prefix("failed to build request") {
        FetchFailureKind::RequestBuild(detail.to_owned())
    } else if message.starts_with("request timed out") {
        FetchFailureKind::Send(message.to_owned())
    } else if let Some(detail) = message.strip_prefix("request failed") {
        FetchFailureKind::Send(detail.to_owned())
    } else if message.starts_with("HTTP ") {
        match message
            .split_whitespace()
            .nth(1)
            .and_then(|s| s.parse::<u16>().ok())
        {
            Some(status) => FetchFailureKind::UnexpectedStatus(status),
            None => FetchFailureKind::Send(message.to_owned()),
        }
    } else if let Some(detail) = message.strip_prefix("failed to read body") {
        FetchFailureKind::BodyRead(detail.to_owned())
    } else if message.starts_with("timed out reading body") {
        FetchFailureKind::BodyRead(message.to_owned())
    } else {
        FetchFailureKind::Send(message.to_owned())
    }
}
