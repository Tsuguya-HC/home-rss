use anyhow::Result;
use spin_sdk::pg::{Certificate, Connection, OpenOptions};

pub async fn connect() -> Result<Connection> {
    let address = spin_sdk::variables::get("db_url").await?;
    let ca_root = spin_sdk::variables::get("db_ca_root")
        .await
        .ok()
        .filter(|s| !s.trim().is_empty())
        .map(Certificate::Text);

    let conn = Connection::open_with_options(&address, OpenOptions { ca_root }).await?;
    Ok(conn)
}
