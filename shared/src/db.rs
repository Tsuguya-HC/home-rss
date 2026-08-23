use anyhow::Result;
use spin_sdk::pg::{Certificate, Connection, OpenOptions};

pub async fn connect() -> Result<Connection> {
    let address = spin_sdk::variables::get("db_url").await?;
    let ca_root = spin_sdk::variables::get("db_ca_root")
        .await
        .ok()
        .filter(|s| !s.trim().is_empty());

    // The URL comes straight out of the CNPG-generated secret, which carries no
    // sslmode. Supplying a CA and then connecting in the clear is incoherent, so
    // whenever one is present the mode is forced rather than merely defaulted.
    // Spin rejects `verify-ca`/`verify-full` outright, and `require` already
    // performs full verification, so `require` is the strictest value available.
    let address = match &ca_root {
        Some(_) => force_sslmode_require(&address),
        None => address,
    };

    let options = OpenOptions {
        ca_root: ca_root.map(Certificate::Text),
    };
    let conn = Connection::open_with_options(&address, options).await?;
    Ok(conn)
}

fn force_sslmode_require(url: &str) -> String {
    let (base, query) = match url.split_once('?') {
        Some((base, query)) => (base, query),
        None => return format!("{url}?sslmode=require"),
    };

    let kept: Vec<&str> = query
        .split('&')
        .filter(|p| !p.is_empty() && !p.split_once('=').is_some_and(|(k, _)| k == "sslmode"))
        .collect();

    if kept.is_empty() {
        format!("{base}?sslmode=require")
    } else {
        format!("{base}?{}&sslmode=require", kept.join("&"))
    }
}

#[cfg(test)]
mod tests {
    use super::force_sslmode_require;

    #[test]
    fn appends_when_no_query() {
        assert_eq!(
            force_sslmode_require("postgres://u:p@h:5432/db"),
            "postgres://u:p@h:5432/db?sslmode=require"
        );
    }

    #[test]
    fn replaces_a_weaker_mode() {
        assert_eq!(
            force_sslmode_require("postgres://u:p@h:5432/db?sslmode=disable"),
            "postgres://u:p@h:5432/db?sslmode=require"
        );
    }

    #[test]
    fn keeps_unrelated_options() {
        assert_eq!(
            force_sslmode_require("postgres://u:p@h:5432/db?connect_timeout=5&sslmode=prefer"),
            "postgres://u:p@h:5432/db?connect_timeout=5&sslmode=require"
        );
    }
}
