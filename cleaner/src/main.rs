use anyhow::Result;
use home_rss_shared::db;
use spin_sdk::pg4::ParameterValue;

fn main() {
    if let Err(e) = run() {
        eprintln!("home-rss-cleaner: {e:#}");
        std::process::exit(1);
    }
}

fn run() -> Result<()> {
    let retention_days: i64 = spin_sdk::variables::get("retention_days")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(30);

    let conn = db::connect()?;

    // Delete read articles older than retention_days
    let deleted = conn.execute(
        "DELETE FROM articles \
         WHERE id IN ( \
           SELECT a.id FROM articles a \
           JOIN read_status rs ON a.id = rs.article_id \
           WHERE a.fetched_at < NOW() - $1::interval \
         )",
        &[ParameterValue::Str(format!("{retention_days} days"))],
    )?;

    println!(
        "home-rss-cleaner: deleted {deleted} read article(s) older than {retention_days} day(s)"
    );
    Ok(())
}
