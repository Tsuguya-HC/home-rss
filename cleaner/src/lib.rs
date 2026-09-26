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

    // READ COMMITTED evaluates the whole DELETE (including the NOT EXISTS on
    // favorites) against the snapshot taken when the statement starts, so a
    // favorite committed mid-DELETE stays invisible and the article is deleted
    // out from under a 204 response (#152). SERIALIZABLE turns that
    // interleaving into a 40001 instead, and the retry re-runs the DELETE
    // after the favorite commits, so NOT EXISTS sees it. Only 40001 is
    // retried; anything else is a real failure.
    const MAX_ATTEMPTS: u32 = 10;
    let mut attempt = 0;
    let deleted = loop {
        attempt += 1;
        conn.execute("START TRANSACTION ISOLATION LEVEL SERIALIZABLE", vec![])
            .await?;
        let outcome = conn
            .execute(
                "DELETE FROM articles \
                 WHERE id IN ( \
                   SELECT a.id FROM articles a \
                   JOIN read_status rs ON a.id = rs.article_id \
                   WHERE a.fetched_at < NOW() - $1::text::interval \
                   AND NOT EXISTS (SELECT 1 FROM favorites f WHERE f.article_id = a.id) \
                 )",
                vec![ParameterValue::Str(format!("{retention_days} days"))],
            )
            .await;
        let deleted = match outcome {
            Ok(n) => n,
            Err(e) if is_serialization_failure(&e) && attempt < MAX_ATTEMPTS => {
                conn.execute("ROLLBACK", vec![]).await.ok();
                continue;
            }
            Err(e) => {
                conn.execute("ROLLBACK", vec![]).await.ok();
                return Err(e.into());
            }
        };
        match conn.execute("COMMIT", vec![]).await {
            Ok(_) => break deleted,
            Err(e) if is_serialization_failure(&e) && attempt < MAX_ATTEMPTS => {
                conn.execute("ROLLBACK", vec![]).await.ok();
                continue;
            }
            Err(e) => {
                conn.execute("ROLLBACK", vec![]).await.ok();
                return Err(e.into());
            }
        }
    };

    let msg = format!("deleted {deleted} read article(s) older than {retention_days} day(s)");
    println!("home-rss-cleaner: {msg}");
    Ok(msg)
}

fn is_serialization_failure(err: &spin_sdk::pg::Error) -> bool {
    match err {
        spin_sdk::pg::Error::PgError(spin_sdk::pg::PgError::QueryFailed(
            spin_sdk::pg::QueryError::DbError(db),
        )) => db.code == "40001",
        _ => false,
    }
}
