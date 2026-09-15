use anyhow::Result;
use url::{Host, Url};

/// SSRF 対策 (#106)。`allowed_outbound_hosts` はスキームを https:443 に限定するが
/// ホストは無制限なので、検証なしに fetch すると内部ホスト探索のオラクルになる。
/// 取れる範囲でスキーム・ポートと明らかな内部宛リテラルを弾く。`add_feed`
/// （事前チェックで 400 を返す）と `fetch_and_store` の入口（定期取得・OPML
/// インポート経由の行も含めて弾く、#106 R2）の両方から呼ぶ。
///
/// 限界: Spin/wasm には DNS 解決して IP を検査する手段が無いため、DNS
/// リバインディングと、内部ホストを指す外部 DNS 名（例: 社内向けの public DNS
/// レコード）は防げない。ネットワーク側 (home-cluster の CiliumNetworkPolicy) が
/// 最終的な境界だが、その CNP の `toEntities: [world]` はクラスタ外全般を指す
/// ため、インターネットだけでなく家庭 LAN 上の非クラスタ機器（NAS/UniFi/ルータ
/// の 443 等）にも到達しうる（クラスタ内部サービスは identity ベースの
/// default-deny で到達不可）。
///
/// IDN/Punycode によるブロックリスト回避は起こらない: `url` crate は UTS46 の
/// マッピングを経て正規化してから `Host::Domain` を返すため、`localhost` /
/// `*.local` 等を装った同値の文字列はここに来る前に正規化済み（実測確認済み）。
///
/// 戻り値は `Url::parse` が正規化した後の `Url`（生の入力文字列ではない）
/// (#106 U7)。検証したのは正規化後の値なのに、呼び出し側が生の入力文字列を
/// 保存・fetch すると、先頭空白や末尾改行のような `Url::parse` は通すが
/// `http::Uri`（実際に fetch に使う側）は拒否する入力がガードを素通りして
/// `feeds` 行だけ作られ、以後 `fetch_and_store` が永遠に失敗し続ける。呼び出し
/// 側は必ずこの戻り値（`.as_str()` / `Display`）を保存・fetch に使うこと。
pub fn reject_internal_feed_url(url: &str) -> Result<Url, &'static str> {
    let parsed = Url::parse(url).map_err(|_| "feed URL is not a valid URL")?;

    if parsed.scheme() != "https" {
        return Err("feed URL must use https");
    }
    // allowed_outbound_hosts は https:443 しか許可しないので、他のポートは
    // outbound で必ず弾かれて 502 になる。理由が分かる形でここで先に弾く
    // (#106 R12)。
    if parsed.port_or_known_default() != Some(443) {
        return Err("feed URL must use port 443");
    }

    match parsed.host() {
        Some(Host::Domain(domain)) => {
            // 末尾ドット付き FQDN ("localhost.") は DNS 上は同じ名前に解決される
            // が文字列としては一致しないので、比較前に取り除く (#106 R1)。
            let domain = domain.trim_end_matches('.').to_ascii_lowercase();
            if domain.is_empty() {
                return Err("feed URL must have a host");
            }
            if domain == "localhost"
                || domain.ends_with(".localhost")
                || domain.ends_with(".internal")
                || domain.ends_with(".local")
            // "*.cluster.local" もここに含まれる
            {
                return Err("feed URL must not point to an internal host");
            }
        }
        Some(Host::Ipv4(v4)) if is_private_or_loopback_v4(&v4) => {
            return Err("feed URL must not point to an internal host");
        }
        Some(Host::Ipv6(v6)) if is_private_or_loopback_v6(&v6) => {
            return Err("feed URL must not point to an internal host");
        }
        Some(_) => {}
        None => return Err("feed URL must have a host"),
    }

    Ok(parsed)
}

fn is_private_or_loopback_v4(v4: &std::net::Ipv4Addr) -> bool {
    v4.is_loopback() || v4.is_private() || v4.is_link_local() || v4.is_unspecified()
}

fn is_private_or_loopback_v6(v6: &std::net::Ipv6Addr) -> bool {
    v6.is_loopback()
        || v6.is_unspecified()
        || (v6.segments()[0] & 0xfe00) == 0xfc00 // ULA fc00::/7
        || (v6.segments()[0] & 0xffc0) == 0xfe80 // link-local fe80::/10
        || embedded_ipv4(v6).is_some_and(|v4| is_private_or_loopback_v4(&v4))
}

/// IPv6 に埋め込まれた IPv4 を取り出す。`::ffff:a.b.c.d`（IPv4-mapped）と、廃止
/// された `::a.b.c.d`（IPv4-compatible）の両方を拾う。素の `Ipv6Addr` の
/// is_loopback/is_unspecified だけを見ていると、`::ffff:169.254.169.254` の
/// ような表記でリンクローカル判定をすり抜ける (#106 H)。
fn embedded_ipv4(v6: &std::net::Ipv6Addr) -> Option<std::net::Ipv4Addr> {
    v6.to_ipv4_mapped().or_else(|| {
        let segs = v6.segments();
        if segs[0..6] == [0, 0, 0, 0, 0, 0] && (segs[6] != 0 || segs[7] != 0) {
            Some(std::net::Ipv4Addr::new(
                (segs[6] >> 8) as u8,
                (segs[6] & 0xff) as u8,
                (segs[7] >> 8) as u8,
                (segs[7] & 0xff) as u8,
            ))
        } else {
            None
        }
    })
}

#[cfg(test)]
mod tests {
    use super::reject_internal_feed_url;

    #[test]
    fn accepts_https_url_with_public_host() {
        assert!(reject_internal_feed_url("https://example.com/feed").is_ok());
        assert!(reject_internal_feed_url("https://example.com:443/feed").is_ok());
    }

    #[test]
    fn accepts_scheme_regardless_of_case() {
        // RFC 3986 上スキームは大小無視。url crate がパース時に小文字化する
        // ので自然に解決する (#106 R11)。
        assert!(reject_internal_feed_url("HTTPS://example.com/feed").is_ok());
    }

    #[test]
    fn rejects_non_https_scheme() {
        assert!(reject_internal_feed_url("http://example.com/feed").is_err());
    }

    #[test]
    fn rejects_non_443_port() {
        assert!(reject_internal_feed_url("https://example.com:8443/feed").is_err());
    }

    #[test]
    fn rejects_localhost() {
        assert!(reject_internal_feed_url("https://localhost/feed").is_err());
        assert!(reject_internal_feed_url("https://foo.localhost/feed").is_err());
    }

    #[test]
    fn rejects_trailing_dot_fqdn_bypass() {
        // "localhost." は DNS 上 "localhost" と同じ名前に解決されるが、
        // 末尾ドットを剥がさないと文字列比較で素通りしていた (#106 R1)。
        assert!(reject_internal_feed_url("https://localhost./feed").is_err());
        assert!(reject_internal_feed_url("https://foo.internal./feed").is_err());
        assert!(reject_internal_feed_url("https://foo.cluster.local./feed").is_err());
    }

    #[test]
    fn rejects_loopback_ip_literal() {
        assert!(reject_internal_feed_url("https://127.0.0.1/feed").is_err());
        assert!(reject_internal_feed_url("https://[::1]/feed").is_err());
    }

    #[test]
    fn rejects_unspecified_ip_literal() {
        assert!(reject_internal_feed_url("https://0.0.0.0/feed").is_err());
    }

    #[test]
    fn rejects_private_and_link_local_ip_literals() {
        assert!(reject_internal_feed_url("https://10.0.0.5/feed").is_err());
        assert!(reject_internal_feed_url("https://192.168.1.1/feed").is_err());
        assert!(reject_internal_feed_url("https://172.16.0.1/feed").is_err());
        assert!(reject_internal_feed_url("https://169.254.169.254/feed").is_err());
    }

    #[test]
    fn rejects_ipv6_ula_and_link_local() {
        assert!(reject_internal_feed_url("https://[fc00::1]/feed").is_err());
        assert!(reject_internal_feed_url("https://[fe80::1]/feed").is_err());
    }

    #[test]
    fn rejects_ipv4_mapped_and_compat_ipv6_literals() {
        // ::ffff:a.b.c.d (mapped) と ::a.b.c.d (廃止された compat 表記) の
        // どちらも埋め込みの IPv4 を展開して判定する (#106 H)。
        assert!(reject_internal_feed_url("https://[::ffff:127.0.0.1]/feed").is_err());
        assert!(reject_internal_feed_url("https://[::ffff:169.254.169.254]/feed").is_err());
        assert!(reject_internal_feed_url("https://[::10.0.0.5]/feed").is_err());
    }

    #[test]
    fn rejects_userinfo_bypass() {
        // ホストではなく userinfo に内部っぽい文字列を混ぜても、実際に接続する
        // ホストはループバック側。url crate の host_str/host は userinfo を
        // 含まないので、手書きパースの頃のような取りこぼしが構造的に無い
        // (#106 R3)。
        assert!(reject_internal_feed_url("https://evil.com@127.0.0.1/feed").is_err());
    }

    #[test]
    fn rejects_internal_dns_suffixes() {
        assert!(reject_internal_feed_url("https://foo.cluster.local/feed").is_err());
        assert!(reject_internal_feed_url("https://foo.internal/feed").is_err());
        assert!(reject_internal_feed_url("https://foo.local/feed").is_err());
    }

    #[test]
    fn normalizes_returned_url_scheme_and_host_case() {
        // 検証結果 (Url) をそのまま保存・fetch に使う前提 (#106 U7):
        // スキーム・ホストは正規化後の小文字表記になる。生の入力文字列
        // ("HTTPS://EXAMPLE.COM/feed") を素通しすると、DB の UNIQUE(url) が
        // 素のテキスト比較のため大小違いだけで二重登録できてしまう。
        let normalized = reject_internal_feed_url("HTTPS://EXAMPLE.COM/feed").unwrap();
        assert_eq!(normalized.as_str(), "https://example.com/feed");
    }

    #[test]
    fn trims_leading_and_trailing_whitespace_from_returned_url() {
        // Url::parse は先頭の空白や末尾の改行を落として解析するが、素の
        // 入力文字列をそのまま使うと http::Uri 側（実際に fetch に使う側）は
        // これを invalid character として拒否する。返ってきた Url を使えば
        // 検証層と実行層のズレが起きない (#106 U7)。
        let normalized = reject_internal_feed_url(" https://example.com/feed\n").unwrap();
        assert_eq!(normalized.as_str(), "https://example.com/feed");
    }

    #[test]
    fn percent_encodes_unsafe_characters_in_returned_url() {
        let normalized = reject_internal_feed_url("https://example.com/a b").unwrap();
        assert_eq!(normalized.as_str(), "https://example.com/a%20b");
    }
}
