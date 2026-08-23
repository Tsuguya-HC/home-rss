use anyhow::Result;
use home_rss_shared::db;
use home_rss_shared::http::{Resp, text};
use spin_sdk::http::{Request, StatusCode};
use spin_sdk::http_service;
use spin_sdk::pg::ParameterValue;

#[http_service]
async fn handle_clean(_req: Request) -> Resp {
    match run().await {
        Ok(msg) => text(StatusCode::OK, msg),
        Err(e) => {
            eprintln!("home-rss-cleaner: {e:#}");
            text(StatusCode::INTERNAL_SERVER_ERROR, format!("error: {e:#}"))
        }
    }
}

async fn run() -> Result<String> {
    let retention_days: i64 = spin_sdk::variables::get("retention_days")
        .await
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(30);

    let conn = db::connect().await?;

    let deleted = conn
        .execute(
            "DELETE FROM articles \
             WHERE id IN ( \
               SELECT a.id FROM articles a \
               JOIN read_status rs ON a.id = rs.article_id \
               WHERE a.fetched_at < NOW() - $1::interval \
             )",
            vec![ParameterValue::Str(format!("{retention_days} days"))],
        )
        .await?;

    let msg = format!("deleted {deleted} read article(s) older than {retention_days} day(s)");
    println!("home-rss-cleaner: {msg}");
    Ok(msg)
}
