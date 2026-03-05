use anyhow::Result;
use home_rss_shared::db;
use spin_sdk::http::{IntoResponse, Request, Response};
use spin_sdk::http_component;
use spin_sdk::pg4::ParameterValue;

#[http_component]
fn handle_clean(_req: Request) -> Result<impl IntoResponse> {
    match run() {
        Ok(msg) => Ok(Response::new(200, msg)),
        Err(e) => {
            eprintln!("home-rss-cleaner: {e:#}");
            Ok(Response::new(500, format!("error: {e:#}")))
        }
    }
}

fn run() -> Result<String> {
    let retention_days: i64 = spin_sdk::variables::get("retention_days")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(30);

    let conn = db::connect()?;

    let deleted = conn.execute(
        "DELETE FROM articles \
         WHERE id IN ( \
           SELECT a.id FROM articles a \
           JOIN read_status rs ON a.id = rs.article_id \
           WHERE a.fetched_at < NOW() - $1::interval \
         )",
        &[ParameterValue::Str(format!("{retention_days} days"))],
    )?;

    let msg = format!("deleted {deleted} read article(s) older than {retention_days} day(s)");
    println!("home-rss-cleaner: {msg}");
    Ok(msg)
}
