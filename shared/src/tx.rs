use anyhow::Result;
use spin_sdk::pg::Connection;

/// `in_transaction` の中の作業の結末。値を呼び出し元に返すのはどちらも
/// 同じで、書き込みを残すかどうかだけが違う。
pub enum TxOutcome<T> {
    /// COMMIT して値を返す。
    Commit(T),
    /// ROLLBACK して値を返す。
    Rollback(T),
}

/// 1 本のコネクション上で BEGIN〜COMMIT/ROLLBACK を回す (#148)。
/// `work` が `Commit` を返したら COMMIT、`Rollback`・`Err` を返したら
/// ROLLBACK する。他の書き処理が使い回すための共有ヘルパー。
pub async fn in_transaction<T>(
    conn: &Connection,
    work: impl std::future::Future<Output = Result<TxOutcome<T>>>,
) -> Result<T> {
    conn.execute("BEGIN", vec![]).await?;
    match work.await {
        Ok(TxOutcome::Commit(value)) => {
            conn.execute("COMMIT", vec![]).await?;
            Ok(value)
        }
        Ok(TxOutcome::Rollback(value)) => {
            // ROLLBACK 自体の失敗で元の結果を置き換えない。捨てるはずの
            // 書き込みが残る可能性はログに残し、呼び出し元には元の値を返す。
            if let Err(e) = conn.execute("ROLLBACK", vec![]).await {
                eprintln!("home-rss-shared tx: ROLLBACK failed: {e:#}");
            }
            Ok(value)
        }
        Err(e) => {
            if let Err(rb) = conn.execute("ROLLBACK", vec![]).await {
                eprintln!("home-rss-shared tx: ROLLBACK failed: {rb:#}");
            }
            Err(e)
        }
    }
}
