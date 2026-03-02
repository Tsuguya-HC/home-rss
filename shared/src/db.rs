use anyhow::Result;
use spin_sdk::pg4::Connection;

pub fn connect() -> Result<Connection> {
    let address = spin_sdk::variables::get("db_url")?;
    let connection = Connection::open(&address)?;
    Ok(connection)
}
