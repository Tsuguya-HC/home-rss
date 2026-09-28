use anyhow::{Context, Result};
use spin_sdk::pg::{Certificate, Connection, OpenOptions, ParameterValue};
use std::future::Future;

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

/// `BEGIN`/`COMMIT` を素の SQL 文として `execute` する。`spin_sdk::pg` に
/// 専用のトランザクション型は無く (v6.0.0 実測)、`Connection` が単一の
/// セッションを指すため、同じ接続で BEGIN した後に流す文は同じ
/// トランザクションに入る。
async fn begin(conn: &Connection) -> Result<()> {
    conn.execute("BEGIN", Vec::<ParameterValue>::new())
        .await
        .context("failed to BEGIN transaction")?;
    Ok(())
}

async fn commit(conn: &Connection) -> Result<()> {
    conn.execute("COMMIT", Vec::<ParameterValue>::new())
        .await
        .context("failed to COMMIT transaction")?;
    Ok(())
}

async fn rollback(conn: &Connection) -> Result<()> {
    conn.execute("ROLLBACK", Vec::<ParameterValue>::new())
        .await
        .context("failed to ROLLBACK transaction")?;
    Ok(())
}

/// 単一接続上のトランザクションで `f` を実行する。`f` が `Ok(v)` を返したら
/// COMMIT して `Ok(v)`、`Err` を返したら ROLLBACK してそのエラーをそのまま
/// 返す。ROLLBACK の成否によらず元のエラーは失わない（ROLLBACK の失敗は
/// eprintln に留め、呼び出し元が受け取る応答は `f` の失敗のままにする）。
/// `f` は `&Connection` を受け取るクロージャで、既存の `conn: &Connection`
/// を取る書き込み関数をそのまま呼べる。共有ライブラリ側に置くのは、この
/// リポジトリの書き込みはどれもここを通すため (#148)。
pub async fn in_transaction<'a, T, E, F, Fut>(conn: &'a Connection, f: F) -> Result<T, E>
where
    F: FnOnce(&'a Connection) -> Fut,
    Fut: Future<Output = Result<T, E>> + 'a,
    E: From<anyhow::Error> + std::fmt::Display,
{
    begin(conn).await?;
    match f(conn).await {
        Ok(v) => {
            commit(conn).await?;
            Ok(v)
        }
        Err(e) => {
            if let Err(rb) = rollback(conn).await {
                eprintln!("home-rss-shared: rollback failed: {rb:#}");
            }
            Err(e)
        }
    }
}

fn force_sslmode_require(url: &str) -> String {
    let (base, query) = match url.split_once('?') {
        Some((base, query)) => (base, query),
        None => return format!("{url}?sslmode=require"),
    };

    let kept: Vec<&str> = query
        .split('&')
        .filter(|p| !p.is_empty() && p.split_once('=').is_none_or(|(k, _)| k != "sslmode"))
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
