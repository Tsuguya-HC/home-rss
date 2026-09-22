use anyhow::Result;
use feed_rs::parser;

/// POST /api/feeds の即時取得と定期取得で共有する純粋なパース (#106)。
/// HTTP 取得・DB 書き込みは含まず、本文バイトから記事レコードの材料だけを取り出す。
/// パース不能は Err として返し、呼び出し側がユーザーへの通知に使う。
#[derive(Debug, PartialEq)]
pub struct ParsedEntry {
    pub url: String,
    pub title: String,
    pub content: Option<String>,
    pub author: Option<String>,
    /// RFC 3339。entry.published が無ければ entry.updated を使う。
    pub published_at: Option<String>,
    pub image_url: Option<String>,
}

#[derive(Debug, PartialEq)]
pub struct ParsedFeed {
    pub title: Option<String>,
    pub site_url: Option<String>,
    pub entries: Vec<ParsedEntry>,
}

pub fn parse_feed_bytes(body: &[u8]) -> Result<ParsedFeed> {
    let feed = parser::parse(body)?;
    Ok(ParsedFeed {
        title: feed.title.as_ref().map(|t| t.content.clone()),
        site_url: feed.links.first().map(|l| l.href.clone()),
        entries: feed
            .entries
            .iter()
            .filter_map(|entry| {
                // #109: rel="alternate"（記事 URL）を rel="self"（フィード URL）より優先する。
                // feed-rs は文書順を保持し、rel 省略時は "alternate" を補うため、
                // alternate が無ければ enclosure でも self でもない先頭を使い、
                // それも無ければ従来通り先頭を使う。
                //
                // #145: Atom の <link rel="enclosure"> は entry.links に
                // rel="enclosure" として入る（feed-rs の parser/atom の実ソースで
                // 確認）。RSS の <enclosure> は links には入らず、feed-rs が
                // media.content にも入れる（parser/rss2 実ソース）。alternate が無い
                // フィードで enclosure が先頭にあると、従来の first() が画像や
                // 音声の URL を記事 URL として掴む。enclosure でも self でもない
                // 先頭を優先し、どちらかしか無いエントリだけ従来通り先頭を使う
                // （音声フィード等でエントリ自体を落とさないため）。
                // 中段で self も除外する理由: self しか選べない形では全 entry の
                // URL が同一になり、UNIQUE(feed_id, url) + ON CONFLICT DO NOTHING
                // で 2 件目以降が黙って捨てられる。enclosure を選ぶ方が件数が保たれる。
                let entry_url = entry
                    .links
                    .iter()
                    .find(|l| l.rel.as_deref() == Some("alternate"))
                    .or_else(|| {
                        entry
                            .links
                            .iter()
                            .find(|l| !matches!(l.rel.as_deref(), Some("enclosure") | Some("self")))
                    })
                    .or_else(|| {
                        entry
                            .links
                            .iter()
                            .find(|l| l.rel.as_deref() == Some("enclosure"))
                    })
                    .or(entry.links.first())?
                    .href
                    .clone();
                let entry_title = entry
                    .title
                    .as_ref()
                    .map(|t| t.content.clone())
                    .unwrap_or_else(|| "(no title)".to_owned());
                let content = entry
                    .content
                    .as_ref()
                    .and_then(|c| c.body.clone())
                    .or_else(|| entry.summary.as_ref().map(|s| s.content.clone()));
                let author = entry.authors.first().map(|a| a.name.clone());
                let published_at = entry.published.or(entry.updated).map(|dt| dt.to_rfc3339());
                // image_url は content より後に決める。本文 <img> フォールバックが
                // 上の content を見るため (#145)。
                let image_url = extract_image_url(entry, &entry_url, content.as_deref());
                Some(ParsedEntry {
                    url: entry_url,
                    title: entry_title,
                    content,
                    author,
                    published_at,
                    image_url,
                })
            })
            .collect(),
    })
}

/// 記事のサムネイル候補を優先順位順に拾う (#145)。
///
/// 優先順位の根拠:
/// 1. `media:thumbnail` — Media RSS で「代表画像」として明示されたもの。
///    フィード発行者が選んだサムネイルなので最も意図に近い。
/// 2. `media:content` の画像 — 同じ Media RSS だが本文相当のメディアも混ざる
///    ため thumbnail の後。RSS の `<enclosure>` は feed-rs が `media.content`
///    に入れる（parser/rss2 に `"enclosure" is treated as if it was a MediaRSS
///    MediaContent element` とある）ため、ここで扱われる。`content_type` が
///    `image/` で始まるものだけ採用し、動画・音声をサムネイルにしない。
///    `content_type` 省略時は URL パスの拡張子で画像か判定する（enclosure と
///    本来の media:content は同じ `MediaContent` 型に潰れて構造的に区別でき
///    ないため。`image/` 以外が明示されていたら捨てる）。
/// 3. `enclosure` (Atom の `<link rel="enclosure">` のみ。RSS 由来は
///    `entry.links` に入らないのでここを通るのは Atom だけ) —
///    こちらは音声・動画フィードで画像以外の用途が主流なので media より後。
///    `media_type` が `image/` で始まるものだけ採用する。type 省略は採用しない
///    （enclosure の主用途は音声であり、無条件採用は誤爆が多い）。
/// 4. 本文/概要 HTML の最初の `<img>` — 明示的な画像指定が無い場合の最終手段。
///
/// 各候補は `normalize_image_url` で絶対化・スキーム検証を通し、最初に通った
/// ものを返す。全部落ちたら `None`（画像が無い記事）。
fn extract_image_url(
    entry: &feed_rs::model::Entry,
    entry_url: &str,
    content: Option<&str>,
) -> Option<String> {
    // 1. media:thumbnail (MediaObject.thumbnails: Vec<MediaThumbnail>、
    // 実ソースでは MediaThumbnail { image: Image, time: Option<...> } で
    // URL は thumb.image.uri: String)。
    for media in &entry.media {
        for thumb in &media.thumbnails {
            if let Some(url) = normalize_image_url(&thumb.image.uri, entry_url) {
                return Some(url);
            }
        }
    }
    // 2. media:content (MediaContent { url: Option<Url>, content_type:
    // Option<MediaTypeBuf>, ... } — String ではないので実ソースで確認)。
    // 画像のみ。RSS <enclosure> は feed-rs が media.content にも入れるが
    // entry.links には入らない（type 付きでも入らないことを実測で確認）。
    // enclosure と本来の media:content は同じ MediaContent 型に潰れ、
    // medium 属性も feed-rs が落とすため構造的に区別できない。
    // そこで content_type 省略時は URL パスの拡張子で画像か判定する。
    for media in &entry.media {
        for item in &media.content {
            let Some(raw) = item.url.as_ref().map(|u| u.as_str()) else {
                continue;
            };
            // MIME は case-insensitive のため小文字で比べる（B-1）。
            // RSS <enclosure> は feed-rs が media.content にも入れるため、
            // こちら側の大文字対策も要る（`type="IMAGE/JPEG"` の enclosure が
            // 取りこぼされることを実測で確認）。
            let is_image = match item.content_type.as_ref() {
                Some(t) => t
                    .to_string()
                    .trim()
                    .to_ascii_lowercase()
                    .starts_with("image/"),
                // type 省略は拡張子で判定する。拡張子の無い URL（CDN の
                // `?id=123` 形式等）は取りこぼすが、音声 URL をサムネイルに
                // してしまうより、画像無しで step4 の本文 <img> に落とす方が
                // 害が小さい。
                None => url_path_has_image_extension(raw),
            };
            if !is_image {
                continue;
            }
            if let Some(url) = normalize_image_url(raw, entry_url) {
                return Some(url);
            }
        }
    }
    // 3. enclosure (entry.links の rel="enclosure"、MIME は Link.media_type)。
    // RSS <enclosure> は links に入らないのでここを通るのは Atom だけ。
    // RSS 由来の画像は step2 で既に拾われている。
    for link in &entry.links {
        if link.rel.as_deref() != Some("enclosure") {
            continue;
        }
        // MIME は RFC 上 case-insensitive のため小文字で比べる
        //（`type="IMAGE/JPEG"` は合法だが feed-rs は生文字列を保持する）。
        let is_image = match link.media_type.as_deref() {
            Some(t) => t.trim().to_ascii_lowercase().starts_with("image/"),
            None => false,
        };
        if !is_image {
            continue;
        }
        if let Some(url) = normalize_image_url(&link.href, entry_url) {
            return Some(url);
        }
    }
    // 4. 本文/概要 HTML の <img>。先頭の src が normalize で落ちても
    // 後続の正規画像を拾うため、通った最初のものを採用する。
    content.and_then(|html| {
        img_srcs(html)
            .into_iter()
            .find_map(|raw| normalize_image_url(&raw, entry_url))
    })
}

/// `content_type` 省略の media:content 候補が画像 URL かをパス拡張子で判定する。
/// クエリ・フラグメントを除いたパス部分だけを見て、大文字小文字を無視する。
/// `svg` は含めない（スクリプトを含みうるため、type 省略という曖昧な状況で
/// `<img>` に流し込む先として選ばない）。
fn url_path_has_image_extension(raw: &str) -> bool {
    // 相対 URL の可能性もあるので、Url::parse に失敗したら `?` / `#` の手前を
    // 素朴に見るフォールバックにする。
    let path = match url::Url::parse(raw) {
        Ok(url) => url.path().to_owned(),
        Err(_) => {
            let trunc = raw.find(['?', '#']).map(|i| &raw[..i]).unwrap_or(raw);
            trunc.to_owned()
        }
    };
    // ファイル名部分（最後の `/` より後）に `.` が無ければ拡張子無し。
    let filename = path.rsplit('/').next().unwrap_or("");
    let ext = filename.rsplit('.').next().unwrap_or("");
    if !filename.contains('.') {
        return false;
    }
    matches!(
        ext.to_ascii_lowercase().as_str(),
        "jpg" | "jpeg" | "png" | "gif" | "webp" | "avif"
    )
}

/// 画像 URL の検証・正規化 (#145)。記事 URL をベースに相対パスを絶対化し、
/// `http` / `https` のみ通す（`data:` / `javascript:` 等の非 HTTP スキームは
/// 一覧表示も保存もしない）。異常に長い URL（2048 文字超）も捨てる。
/// いずれかに引っかかったら `None`。
fn normalize_image_url(raw: &str, entry_url: &str) -> Option<String> {
    const MAX_IMAGE_URL_LEN: usize = 2048;
    let raw = raw.trim();
    if raw.is_empty() {
        return None;
    }
    // `#frag` は Url::join がベース URL + フラグメントに解決してしまい、
    // 記事 URL 自身（HTML ページ）を image_url として保存してしまうので弾く。
    if raw.starts_with('#') {
        return None;
    }
    // ベースが壊れていることは通常無い（entry_url 自体がフィード由来の URL）が、
    // 万一パース不能でも絶対 URL の候補まで道連れにしない。
    let joined = match url::Url::parse(entry_url) {
        Ok(base) => base.join(raw).ok()?,
        Err(_) => url::Url::parse(raw).ok()?,
    };
    match joined.scheme() {
        "http" | "https" => {}
        _ => return None,
    }
    let url = joined.to_string();
    if url.len() > MAX_IMAGE_URL_LEN {
        return None;
    }
    Some(url)
}

/// HTML 中の `<img src="...">` の src 値を出現順に列挙する (#145)。
/// 新しい依存を足さず自前スキャンする理由: やりたいのは src の抜き出しだけで、
/// HTML パーサを持ち込むほどのことは無い。対応ケースはテストで固定する:
/// ダブル/シングルクォート、引用符なし、属性順の違い、`<IMG` 等の大文字小文字、
/// 自己閉じ、`<image>` 等の紛らわしいタグ名の除外、`data-src` 等の部分一致の除外。
/// `<!-- ... -->` 内のタグはコメントアウト済みとして読み飛ばす。
/// 既知の割り切り: タグ終端は最初に現れた `>` で決めるため、引用符内に `>` を
/// 含む属性（`<img alt="a>b" src="x.jpg">` 等）ではタグが途中で切れて src を
/// 見逃す。簡易スキャナの複雑度に見合わないので直さない。
fn img_srcs(html: &str) -> Vec<String> {
    let bytes = html.as_bytes();
    let mut out = Vec::new();
    let mut i = 0;
    while i + 4 <= bytes.len() {
        // HTML コメントは読み飛ばす。`-->` が無ければそれ以降にタグは
        // 成立し得ないので打ち切る。
        if bytes[i..].starts_with(b"<!--") {
            let rest = &bytes[i + 4..];
            match rest.windows(3).position(|w| w == b"-->") {
                Some(rel) => {
                    i = i + 4 + rel + 3;
                    continue;
                }
                None => break,
            }
        }
        // `<img` の大文字小文字を問わない検出。ただし `<image>` のようにタグ名が
        // 続くものは別タグなので、直後が空白・`/`・`>` のときだけ採用する。
        if bytes[i] == b'<'
            && (bytes[i + 1] == b'i' || bytes[i + 1] == b'I')
            && (bytes[i + 2] == b'm' || bytes[i + 2] == b'M')
            && (bytes[i + 3] == b'g' || bytes[i + 3] == b'G')
            && (i + 4 >= bytes.len()
                // img_tag_src 側の空白スキップは is_ascii_whitespace() で
                // \x0C を含むため、ここも揃えて <img\x0Csrc=...> を認識する。
                || matches!(bytes[i + 4], b' ' | b'\t' | b'\n' | b'\x0C' | b'\r' | b'/' | b'>'))
        {
            let Some(rel) = html[i..].find('>') else {
                break;
            };
            let tag_end = i + rel;
            if let Some(src) = img_tag_src(&html[i..tag_end]) {
                out.push(src);
            }
            i = tag_end + 1;
        } else {
            i += 1;
        }
    }
    out
}

/// 単一の `<img ...` タグ文字列（`>` を含まない）から `src` 属性値を抜く。
/// 属性名はトークンとして切り出して完全一致で比べるため、`data-src` に誤爆しない。
fn img_tag_src(tag: &str) -> Option<String> {
    let bytes = tag.as_bytes();
    // `<img` の 4 文字を飛ばす。
    let mut i = 4;
    while i < bytes.len() {
        // 空白と自己閉じの `/` を読み飛ばす。
        while i < bytes.len() && (bytes[i].is_ascii_whitespace() || bytes[i] == b'/') {
            i += 1;
        }
        if i >= bytes.len() {
            break;
        }
        // 属性名の切り出し。
        let name_start = i;
        while i < bytes.len()
            && (bytes[i].is_ascii_alphanumeric() || matches!(bytes[i], b':' | b'_' | b'-' | b'.'))
        {
            i += 1;
        }
        if name_start == i {
            // 属性名にならない文字（`>` 直前など）は 1 文字進めて継続。
            i += 1;
            continue;
        }
        let name = &tag[name_start..i];
        while i < bytes.len() && bytes[i].is_ascii_whitespace() {
            i += 1;
        }
        if i >= bytes.len() || bytes[i] != b'=' {
            // 値無し属性（`ismap` 等）。次へ。
            continue;
        }
        i += 1;
        while i < bytes.len() && bytes[i].is_ascii_whitespace() {
            i += 1;
        }
        if i >= bytes.len() {
            break;
        }
        let value = if bytes[i] == b'"' || bytes[i] == b'\'' {
            let quote = bytes[i];
            i += 1;
            let start = i;
            while i < bytes.len() && bytes[i] != quote {
                i += 1;
            }
            let value = &tag[start..i];
            i = (i + 1).min(bytes.len());
            value.to_owned()
        } else {
            // 引用符なし。空白またはタグ終端まで。
            let start = i;
            while i < bytes.len() && !bytes[i].is_ascii_whitespace() {
                i += 1;
            }
            tag[start..i].to_owned()
        };
        if name.eq_ignore_ascii_case("src") && !value.trim().is_empty() {
            return Some(value);
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::parse_feed_bytes;

    const RSS: &[u8] = br#"<?xml version="1.0"?>
<rss version="2.0"><channel>
<title>Example Feed</title>
<link>https://example.com/</link>
<item>
<title>Hello</title>
<link>https://example.com/hello</link>
<description>world</description>
<pubDate>Mon, 15 Sep 2026 00:00:00 GMT</pubDate>
</item>
</channel></rss>"#;

    #[test]
    fn parses_feed_title_site_url_and_entry() {
        let feed = parse_feed_bytes(RSS).unwrap();
        assert_eq!(feed.title.as_deref(), Some("Example Feed"));
        assert_eq!(feed.site_url.as_deref(), Some("https://example.com/"));
        assert_eq!(feed.entries.len(), 1);
        assert_eq!(feed.entries[0].url, "https://example.com/hello");
        assert_eq!(feed.entries[0].title, "Hello");
        assert_eq!(feed.entries[0].content.as_deref(), Some("world"));
        assert_eq!(
            feed.entries[0].published_at.as_deref(),
            Some("2026-09-15T00:00:00+00:00")
        );
    }

    #[test]
    fn entry_without_link_is_skipped() {
        let xml = br#"<?xml version="1.0"?>
<rss version="2.0"><channel>
<title>T</title>
<item><title>No link</title></item>
<item><title>Has link</title><link>https://example.com/x</link></item>
</channel></rss>"#;
        let feed = parse_feed_bytes(xml).unwrap();
        assert_eq!(feed.entries.len(), 1);
        assert_eq!(feed.entries[0].url, "https://example.com/x");
    }

    #[test]
    fn entry_without_title_gets_default() {
        let xml = br#"<?xml version="1.0"?>
<rss version="2.0"><channel>
<title>T</title>
<item><link>https://example.com/x</link></item>
</channel></rss>"#;
        let feed = parse_feed_bytes(xml).unwrap();
        assert_eq!(feed.entries.len(), 1);
        assert_eq!(feed.entries[0].title, "(no title)");
    }

    #[test]
    fn prefers_alternate_link_over_self_link() {
        // #109: feeds that list a <link rel="self"> (feed URL) before the
        // <link rel="alternate"> (article URL) must store the article URL,
        // so the reader can link to the original page. feed-rs preserves
        // document order, so taking links.first() stores the self URL.
        let xml = br#"<?xml version="1.0" encoding="utf-8"?>
<feed xmlns="http://www.w3.org/2005/Atom">
<title>Example</title>
<entry>
<title>An article</title>
<link rel="self" href="https://example.com/feed.xml"/>
<link rel="alternate" href="https://example.com/article"/>
</entry>
</feed>"#;
        let feed = parse_feed_bytes(xml).unwrap();
        assert_eq!(feed.entries.len(), 1);
        assert_eq!(feed.entries[0].url, "https://example.com/article");
    }

    #[test]
    fn unparseable_body_is_an_error() {
        assert!(parse_feed_bytes(b"this is not a feed").is_err());
    }

    #[test]
    fn enclosure_is_not_mistaken_for_article_url() {
        // #145: Atom の <link rel="enclosure"> が先頭にあっても、記事 URL と
        // して掴むのは enclosure ではなく alternate の方でなければならない
        // (#109 の意図を保つ)。enclosure は画像抽出には使う。
        let xml = br#"<?xml version="1.0" encoding="utf-8"?>
<feed xmlns="http://www.w3.org/2005/Atom">
<title>Example</title>
<entry>
<title>An article</title>
<link rel="enclosure" href="https://example.com/audio.mp3" type="audio/mpeg"/>
<link rel="alternate" href="https://example.com/article"/>
</entry>
</feed>"#;
        let feed = parse_feed_bytes(xml).unwrap();
        assert_eq!(feed.entries.len(), 1);
        assert_eq!(feed.entries[0].url, "https://example.com/article");
    }

    #[test]
    fn enclosure_wins_when_only_enclosure_and_self_remain() {
        // #145: alternate が無く enclosure と self しか無い場合、self を選ぶと
        // 全 entry が同一 URL になり ON CONFLICT DO NOTHING で潰れるため、
        // enclosure を選ぶ（件数を保つ）。
        let xml = br#"<?xml version="1.0" encoding="utf-8"?>
<feed xmlns="http://www.w3.org/2005/Atom">
<title>Example</title>
<entry>
<title>An article</title>
<link rel="enclosure" href="https://example.com/thumb.jpg" type="image/jpeg"/>
<link rel="self" href="https://example.com/feed.xml"/>
</entry>
</feed>"#;
        let feed = parse_feed_bytes(xml).unwrap();
        assert_eq!(feed.entries.len(), 1);
        assert_eq!(feed.entries[0].url, "https://example.com/thumb.jpg");
    }

    #[test]
    fn self_only_entries_keep_distinct_enclosure_urls() {
        // alternate が無く各 entry が同名 self + 固有 enclosure の場合、
        // enclosure を選ぶことで entry が区別される（self だと全件同一 URL）。
        let xml = br#"<?xml version="1.0" encoding="utf-8"?>
<feed xmlns="http://www.w3.org/2005/Atom">
<title>Example</title>
<entry>
<title>E1</title>
<link rel="enclosure" href="https://example.com/e1.mp3" type="audio/mpeg"/>
<link rel="self" href="https://example.com/feed.xml"/>
</entry>
<entry>
<title>E2</title>
<link rel="enclosure" href="https://example.com/e2.mp3" type="audio/mpeg"/>
<link rel="self" href="https://example.com/feed.xml"/>
</entry>
</feed>"#;
        let feed = parse_feed_bytes(xml).unwrap();
        assert_eq!(feed.entries.len(), 2);
        assert_eq!(feed.entries[0].url, "https://example.com/e1.mp3");
        assert_eq!(feed.entries[1].url, "https://example.com/e2.mp3");
    }

    #[test]
    fn extracts_image_from_rss_enclosure() {
        let xml = br#"<?xml version="1.0"?>
<rss version="2.0"><channel>
<title>T</title>
<link>https://example.com/</link>
<item>
<title>E</title>
<link>https://example.com/e</link>
<enclosure url="https://example.com/img.jpg" length="1234" type="image/jpeg"/>
</item>
</channel></rss>"#;
        let feed = parse_feed_bytes(xml).unwrap();
        assert_eq!(feed.entries.len(), 1);
        assert_eq!(
            feed.entries[0].image_url.as_deref(),
            Some("https://example.com/img.jpg")
        );
    }

    #[test]
    fn extracts_image_from_media_thumbnail() {
        let xml = br#"<?xml version="1.0"?>
<rss version="2.0" xmlns:media="http://search.yahoo.com/mrss/"><channel>
<title>T</title>
<link>https://example.com/</link>
<item>
<title>E</title>
<link>https://example.com/e</link>
<media:thumbnail url="https://example.com/thumb.jpg" width="100" height="100"/>
</item>
</channel></rss>"#;
        let feed = parse_feed_bytes(xml).unwrap();
        assert_eq!(feed.entries.len(), 1);
        assert_eq!(
            feed.entries[0].image_url.as_deref(),
            Some("https://example.com/thumb.jpg")
        );
    }

    #[test]
    fn extracts_image_from_media_content() {
        let xml = br#"<?xml version="1.0"?>
<rss version="2.0" xmlns:media="http://search.yahoo.com/mrss/"><channel>
<title>T</title>
<link>https://example.com/</link>
<item>
<title>E</title>
<link>https://example.com/e</link>
<media:content url="https://example.com/photo.jpg" type="image/jpeg" medium="image"/>
</item>
</channel></rss>"#;
        let feed = parse_feed_bytes(xml).unwrap();
        assert_eq!(feed.entries.len(), 1);
        assert_eq!(
            feed.entries[0].image_url.as_deref(),
            Some("https://example.com/photo.jpg")
        );
    }

    #[test]
    fn extracts_image_from_atom_enclosure() {
        let xml = br#"<?xml version="1.0" encoding="utf-8"?>
<feed xmlns="http://www.w3.org/2005/Atom">
<title>T</title>
<entry>
<title>E</title>
<link rel="alternate" href="https://example.com/e"/>
<link rel="enclosure" href="https://example.com/img.png" type="image/png" length="100"/>
</entry>
</feed>"#;
        let feed = parse_feed_bytes(xml).unwrap();
        assert_eq!(feed.entries.len(), 1);
        assert_eq!(feed.entries[0].url, "https://example.com/e");
        assert_eq!(
            feed.entries[0].image_url.as_deref(),
            Some("https://example.com/img.png")
        );
    }

    #[test]
    fn extracts_first_img_from_body_when_no_media_or_enclosure() {
        let xml = br#"<?xml version="1.0"?>
<rss version="2.0"><channel>
<title>T</title>
<link>https://example.com/</link>
<item>
<title>E</title>
<link>https://example.com/e</link>
<description>&lt;p&gt;hi&lt;/p&gt;&lt;img src="https://example.com/a.jpg"&gt;&lt;img src="https://example.com/b.jpg"&gt;</description>
</item>
</channel></rss>"#;
        let feed = parse_feed_bytes(xml).unwrap();
        assert_eq!(feed.entries.len(), 1);
        assert_eq!(
            feed.entries[0].image_url.as_deref(),
            Some("https://example.com/a.jpg")
        );
    }

    #[test]
    fn entry_without_image_has_none() {
        let feed = parse_feed_bytes(RSS).unwrap();
        assert_eq!(feed.entries.len(), 1);
        assert_eq!(feed.entries[0].image_url, None);
    }

    #[test]
    fn non_image_enclosure_is_ignored() {
        let xml = br#"<?xml version="1.0"?>
<rss version="2.0"><channel>
<title>T</title>
<link>https://example.com/</link>
<item>
<title>E</title>
<link>https://example.com/e</link>
<enclosure url="https://example.com/audio.mp3" length="1234" type="audio/mpeg"/>
</item>
</channel></rss>"#;
        let feed = parse_feed_bytes(xml).unwrap();
        assert_eq!(feed.entries.len(), 1);
        assert_eq!(feed.entries[0].image_url, None);
    }

    #[test]
    fn dangerous_schemes_are_rejected() {
        // data: / javascript: は絶対化しても http(s) にならないので捨てられる。
        let xml = br#"<?xml version="1.0"?>
<rss version="2.0"><channel>
<title>T</title>
<link>https://example.com/</link>
<item>
<title>E1</title>
<link>https://example.com/e1</link>
<description>&lt;img src="data:image/png;base64,AAAA"&gt;</description>
</item>
<item>
<title>E2</title>
<link>https://example.com/e2</link>
<description>&lt;img src="javascript:alert(1)"&gt;</description>
</item>
</channel></rss>"#;
        let feed = parse_feed_bytes(xml).unwrap();
        assert_eq!(feed.entries.len(), 2);
        assert_eq!(feed.entries[0].image_url, None);
        assert_eq!(feed.entries[1].image_url, None);
    }

    #[test]
    fn relative_img_src_is_resolved_against_entry_url() {
        let xml = br#"<?xml version="1.0"?>
<rss version="2.0"><channel>
<title>T</title>
<link>https://example.com/</link>
<item>
<title>E</title>
<link>https://example.com/posts/e</link>
<description>&lt;img src="/a.png"&gt;</description>
</item>
</channel></rss>"#;
        let feed = parse_feed_bytes(xml).unwrap();
        assert_eq!(feed.entries.len(), 1);
        assert_eq!(
            feed.entries[0].image_url.as_deref(),
            Some("https://example.com/a.png")
        );
    }

    #[test]
    fn img_scanner_handles_quote_variants_case_and_attr_order() {
        // ダブル/シングルクォート、引用符なし、属性順、タグの大文字小文字、
        // 自己閉じ、data-src の部分一致除外を直接検査する。
        fn first(html: &str) -> Option<String> {
            super::img_srcs(html).into_iter().next()
        }
        assert_eq!(
            first(r#"<IMG ALT="x" SRC='https://example.com/u.jpg'/>"#).as_deref(),
            Some("https://example.com/u.jpg")
        );
        assert_eq!(
            first(r#"<img src=https://example.com/plain.jpg>"#).as_deref(),
            Some("https://example.com/plain.jpg")
        );
        assert_eq!(
            first(r#"<img data-src="https://example.com/no.jpg">"#),
            None
        );
        assert_eq!(first(r#"<image src="https://example.com/no.jpg">"#), None);
    }

    #[test]
    fn thumbnail_wins_when_all_candidates_present() {
        // A-1: 4 候補すべてを含む entry では media:thumbnail が勝つ。
        // 順序を入れ替える変異を殺すための固定。
        let xml = br#"<?xml version="1.0"?>
<rss version="2.0" xmlns:media="http://search.yahoo.com/mrss/"><channel>
<title>T</title>
<link>https://example.com/</link>
<item>
<title>E</title>
<link>https://example.com/e</link>
<enclosure url="https://example.com/enclosure.jpg" length="1234" type="image/jpeg"/>
<media:thumbnail url="https://example.com/thumb.jpg" width="100" height="100"/>
<media:content url="https://example.com/content.jpg" type="image/jpeg" medium="image"/>
<description>&lt;img src="https://example.com/body.jpg"&gt;</description>
</item>
</channel></rss>"#;
        let feed = parse_feed_bytes(xml).unwrap();
        assert_eq!(feed.entries.len(), 1);
        assert_eq!(
            feed.entries[0].image_url.as_deref(),
            Some("https://example.com/thumb.jpg")
        );
    }

    // RSS <enclosure> は feed-rs が media.content にも入れるため、下の pairwise
    // 用テストは media:content を持つ Atom 寄りの形ではなく、media と enclosure
    // をともに持てる形で書く必要がある。RSS の media:content は type 省略でも
    // enclosure 由来の content が media_type 情報を持つため、純粋な pairwise
    // 比較は feed 形式の制約を受ける。
    #[test]
    fn media_content_wins_over_enclosure() {
        // A-1 pairwise: thumbnail が無い場合は media:content が enclosure に勝つ。
        // RSS では enclosure が media.content にも入るため、明示の media:content
        // が文書順で先にある形で確認する。
        let xml = br#"<?xml version="1.0"?>
<rss version="2.0" xmlns:media="http://search.yahoo.com/mrss/"><channel>
<title>T</title>
<link>https://example.com/</link>
<item>
<title>E</title>
<link>https://example.com/e</link>
<media:content url="https://example.com/content.jpg" type="image/jpeg" medium="image"/>
<enclosure url="https://example.com/enclosure.jpg" length="1234" type="image/jpeg"/>
</item>
</channel></rss>"#;
        let feed = parse_feed_bytes(xml).unwrap();
        assert_eq!(feed.entries.len(), 1);
        assert_eq!(
            feed.entries[0].image_url.as_deref(),
            Some("https://example.com/content.jpg")
        );
    }

    #[test]
    fn enclosure_wins_over_body_img() {
        // A-1 pairwise: media が無い場合は enclosure が本文 img に勝つ。
        let xml = br#"<?xml version="1.0"?>
<rss version="2.0"><channel>
<title>T</title>
<link>https://example.com/</link>
<item>
<title>E</title>
<link>https://example.com/e</link>
<enclosure url="https://example.com/enclosure.jpg" length="1234" type="image/jpeg"/>
<description>&lt;img src="https://example.com/body.jpg"&gt;</description>
</item>
</channel></rss>"#;
        let feed = parse_feed_bytes(xml).unwrap();
        assert_eq!(feed.entries.len(), 1);
        assert_eq!(
            feed.entries[0].image_url.as_deref(),
            Some("https://example.com/enclosure.jpg")
        );
    }

    #[test]
    fn media_content_without_type_is_treated_as_image() {
        // A-2: media:content の type 省略は画像として採用する。
        let xml = br#"<?xml version="1.0"?>
<rss version="2.0" xmlns:media="http://search.yahoo.com/mrss/"><channel>
<title>T</title>
<link>https://example.com/</link>
<item>
<title>E</title>
<link>https://example.com/e</link>
<media:content url="https://example.com/photo.jpg" medium="image"/>
</item>
</channel></rss>"#;
        let feed = parse_feed_bytes(xml).unwrap();
        assert_eq!(feed.entries.len(), 1);
        assert_eq!(
            feed.entries[0].image_url.as_deref(),
            Some("https://example.com/photo.jpg")
        );
    }

    #[test]
    fn media_content_with_video_type_is_ignored() {
        // A-2: media:content の明示的非画像 (video/*) は捨てる。
        let xml = br#"<?xml version="1.0"?>
<rss version="2.0" xmlns:media="http://search.yahoo.com/mrss/"><channel>
<title>T</title>
<link>https://example.com/</link>
<item>
<title>E</title>
<link>https://example.com/e</link>
<media:content url="https://example.com/movie.mp4" type="video/mp4" medium="video"/>
</item>
</channel></rss>"#;
        let feed = parse_feed_bytes(xml).unwrap();
        assert_eq!(feed.entries.len(), 1);
        assert_eq!(feed.entries[0].image_url, None);
    }

    #[test]
    fn enclosure_without_type_is_ignored() {
        // A-2: enclosure の type 省略は不採用（主用途が音声のため）。
        // RSS <enclosure> は feed-rs が media.content にも入れるが step2 の
        // 拡張子判定で .mp3 が落ち、step3（links）にも入らないため捨てられる。
        // Atom 形でも step3 の判定で同様に捨てられることを両方固定する（D-1）。
        let rss = br#"<?xml version="1.0"?>
<rss version="2.0"><channel>
<title>T</title>
<link>https://example.com/</link>
<item>
<title>E</title>
<link>https://example.com/e</link>
<enclosure url="https://example.com/audio.mp3" length="1234"/>
</item>
</channel></rss>"#;
        let feed = parse_feed_bytes(rss).unwrap();
        assert_eq!(feed.entries.len(), 1);
        assert_eq!(feed.entries[0].image_url, None);
        let atom = br#"<?xml version="1.0" encoding="utf-8"?>
<feed xmlns="http://www.w3.org/2005/Atom">
<title>T</title>
<entry>
<title>E</title>
<link rel="alternate" href="https://example.com/e"/>
<link rel="enclosure" href="https://example.com/img.jpg" length="1234"/>
</entry>
</feed>"#;
        let feed = parse_feed_bytes(atom).unwrap();
        assert_eq!(feed.entries.len(), 1);
        assert_eq!(feed.entries[0].image_url, None);
    }

    #[test]
    fn media_content_with_uppercase_image_type_is_adopted() {
        // D-2: 純粋な media:content の `type="IMAGE/JPEG"` は画像として採用する。
        // step2 の小文字化を削る変異を殺す（RSS enclosure 由来だと step3 でも
        // 拾われて Some のままになるため、enclosure 由来ではない形で固定する）。
        let xml = br#"<?xml version="1.0"?>
<rss version="2.0" xmlns:media="http://search.yahoo.com/mrss/"><channel>
<title>T</title>
<link>https://example.com/</link>
<item>
<title>E</title>
<link>https://example.com/e</link>
<media:content url="https://example.com/photo.jpg" type="IMAGE/JPEG" medium="image"/>
</item>
</channel></rss>"#;
        let feed = parse_feed_bytes(xml).unwrap();
        assert_eq!(feed.entries.len(), 1);
        assert_eq!(
            feed.entries[0].image_url.as_deref(),
            Some("https://example.com/photo.jpg")
        );
    }

    #[test]
    fn invalid_thumbnail_falls_through_to_enclosure() {
        // D-3: 無効な上位候補（data: の thumbnail）を飛ばして下位の enclosure を拾う。
        let xml = br#"<?xml version="1.0"?>
<rss version="2.0" xmlns:media="http://search.yahoo.com/mrss/"><channel>
<title>T</title>
<link>https://example.com/</link>
<item>
<title>E</title>
<link>https://example.com/e</link>
<enclosure url="https://example.com/enclosure.jpg" length="1234" type="image/jpeg"/>
<media:thumbnail url="data:image/png;base64,AAAA" width="100" height="100"/>
</item>
</channel></rss>"#;
        let feed = parse_feed_bytes(xml).unwrap();
        assert_eq!(feed.entries.len(), 1);
        assert_eq!(
            feed.entries[0].image_url.as_deref(),
            Some("https://example.com/enclosure.jpg")
        );
    }

    #[test]
    fn img_without_src_is_skipped_for_next_img() {
        // D-4: src の無い最初の <img> を飛ばして 2 枚目を拾う。
        assert_eq!(
            super::img_srcs(r#"<img alt="x"><img src="https://example.com/a.jpg">"#)
                .first()
                .map(String::as_str),
            Some("https://example.com/a.jpg")
        );
    }

    #[test]
    fn enclosure_media_type_is_case_insensitive() {
        // B-1: MIME は case-insensitive なので `type="IMAGE/JPEG"` も画像扱い。
        // RSS <enclosure> は media.content にも入るため、step 3 (links) の
        // 大文字対策を直接固定するには Atom <link rel="enclosure"> の形を使う。
        let xml = br#"<?xml version="1.0" encoding="utf-8"?>
<feed xmlns="http://www.w3.org/2005/Atom">
<title>T</title>
<entry>
<title>E</title>
<link rel="alternate" href="https://example.com/e"/>
<link rel="enclosure" href="https://example.com/img.jpg" type="IMAGE/JPEG" length="1234"/>
</entry>
</feed>"#;
        let feed = parse_feed_bytes(xml).unwrap();
        assert_eq!(feed.entries.len(), 1);
        assert_eq!(
            feed.entries[0].image_url.as_deref(),
            Some("https://example.com/img.jpg")
        );
    }

    #[test]
    fn overlong_image_url_is_rejected_at_boundary() {
        // A-3: 2048 文字超は捨て、ちょうど 2048 は採用する。
        fn xml_with_img(src: &str) -> Vec<u8> {
            format!(
                "<?xml version=\"1.0\"?>\n<rss version=\"2.0\"><channel>\n<title>T</title>\n<link>https://example.com/</link>\n<item>\n<title>E</title>\n<link>https://example.com/e</link>\n<description>&lt;img src=\"{src}\"&gt;</description>\n</item>\n</channel></rss>"
            )
            .into_bytes()
        }
        // "https://example.com/" (20 文字) + "a" * 2028 = ちょうど 2048。
        let ok_src = format!("https://example.com/{}", "a".repeat(2028));
        assert_eq!(ok_src.len(), 2048);
        let feed = parse_feed_bytes(&xml_with_img(&ok_src)).unwrap();
        assert_eq!(feed.entries[0].image_url.as_deref(), Some(ok_src.as_str()));
        // 1 文字超えると捨てる。
        let long_src = format!("https://example.com/{}", "a".repeat(2029));
        let feed = parse_feed_bytes(&xml_with_img(&long_src)).unwrap();
        assert_eq!(feed.entries[0].image_url, None);
    }

    #[test]
    fn multibyte_body_img_is_extracted() {
        // A-4: マルチバイト文字を含む本文でもパニックせず img を抜ける
        //（和文フィードが主用途のため parse_feed_bytes 経由で回帰ガード）。
        let xml = "<?xml version=\"1.0\"?>\n<rss version=\"2.0\"><channel>\n<title>T</title>\n<link>https://example.com/</link>\n<item>\n<title>E</title>\n<link>https://example.com/e</link>\n<description>&lt;p&gt;こんにちは世界&lt;/p&gt;&lt;img src=\"https://example.com/a.jpg\"&gt;</description>\n</item>\n</channel></rss>";
        let feed = parse_feed_bytes(xml.as_bytes()).unwrap();
        assert_eq!(feed.entries.len(), 1);
        assert_eq!(
            feed.entries[0].image_url.as_deref(),
            Some("https://example.com/a.jpg")
        );
    }

    #[test]
    fn fragment_only_img_src_is_rejected() {
        // B-5: `<img src="#">` は Url::join で記事 URL 自身になるため弾く。
        let xml = br##"<?xml version="1.0"?>
<rss version="2.0"><channel>
<title>T</title>
<link>https://example.com/</link>
<item>
<title>E</title>
<link>https://example.com/e</link>
<description>&lt;img src="#"&gt;</description>
</item>
</channel></rss>"##;
        let feed = parse_feed_bytes(xml).unwrap();
        assert_eq!(feed.entries.len(), 1);
        assert_eq!(feed.entries[0].image_url, None);
    }

    #[test]
    fn enclosure_without_type_or_length_is_ignored() {
        // D-1 再修正: `<enclosure url=".../a.mp3"/>`（type も length も省略）は
        // 旧ヒューリスティックでは step2 で拾ってしまっていた。拡張子判定では
        // .mp3 が落ち、links にも入らないため None。
        let xml = br#"<?xml version="1.0"?>
<rss version="2.0"><channel>
<title>T</title>
<link>https://example.com/</link>
<item>
<title>E</title>
<link>https://example.com/e</link>
<enclosure url="https://example.com/a.mp3"/>
</item>
</channel></rss>"#;
        let feed = parse_feed_bytes(xml).unwrap();
        assert_eq!(feed.entries.len(), 1);
        assert_eq!(feed.entries[0].image_url, None);
    }

    #[test]
    fn media_content_without_type_but_with_filesize_is_adopted() {
        // D-1 再修正: `<media:content url=".../b.jpg" fileSize="999"/>`（type
        // 省略・fileSize あり）は旧ヒューリスティックでは enclosure 由来と
        // 誤認して捨てていた。拡張子判定では .jpg で採用される。
        let xml = br#"<?xml version="1.0"?>
<rss version="2.0" xmlns:media="http://search.yahoo.com/mrss/"><channel>
<title>T</title>
<link>https://example.com/</link>
<item>
<title>E</title>
<link>https://example.com/e</link>
<media:content url="https://example.com/b.jpg" fileSize="999"/>
</item>
</channel></rss>"#;
        let feed = parse_feed_bytes(xml).unwrap();
        assert_eq!(feed.entries.len(), 1);
        assert_eq!(
            feed.entries[0].image_url.as_deref(),
            Some("https://example.com/b.jpg")
        );
    }

    #[test]
    fn typeless_image_url_with_query_is_adopted() {
        // クエリ付き画像 URL（type 省略）はパス部分の拡張子で採用される。
        let xml = br#"<?xml version="1.0"?>
<rss version="2.0" xmlns:media="http://search.yahoo.com/mrss/"><channel>
<title>T</title>
<link>https://example.com/</link>
<item>
<title>E</title>
<link>https://example.com/e</link>
<media:content url="https://example.com/b.jpg?w=100"/>
</item>
</channel></rss>"#;
        let feed = parse_feed_bytes(xml).unwrap();
        assert_eq!(feed.entries.len(), 1);
        assert_eq!(
            feed.entries[0].image_url.as_deref(),
            Some("https://example.com/b.jpg?w=100")
        );
    }

    #[test]
    fn typeless_uppercase_extension_is_adopted() {
        // 拡張子が大文字でも採用される（type 省略）。
        let xml = br#"<?xml version="1.0"?>
<rss version="2.0" xmlns:media="http://search.yahoo.com/mrss/"><channel>
<title>T</title>
<link>https://example.com/</link>
<item>
<title>E</title>
<link>https://example.com/e</link>
<media:content url="https://example.com/B.JPG"/>
</item>
</channel></rss>"#;
        let feed = parse_feed_bytes(xml).unwrap();
        assert_eq!(feed.entries.len(), 1);
        assert_eq!(
            feed.entries[0].image_url.as_deref(),
            Some("https://example.com/B.JPG")
        );
    }

    #[test]
    fn typeless_extensionless_url_is_not_adopted() {
        // 拡張子の無い URL（type 省略）は step2 で採用されない。
        let xml = br#"<?xml version="1.0"?>
<rss version="2.0" xmlns:media="http://search.yahoo.com/mrss/"><channel>
<title>T</title>
<link>https://example.com/</link>
<item>
<title>E</title>
<link>https://example.com/e</link>
<media:content url="https://example.com/image"/>
</item>
</channel></rss>"#;
        let feed = parse_feed_bytes(xml).unwrap();
        assert_eq!(feed.entries.len(), 1);
        assert_eq!(feed.entries[0].image_url, None);
    }

    #[test]
    fn img_with_form_feed_separator_is_recognized() {
        // B-3: `<img\x0Csrc=...>` を img と認識する（img_tag_src 側の
        // is_ascii_whitespace との整合）。
        assert_eq!(
            super::img_srcs("<img\x0Csrc=\"https://example.com/a.jpg\">")
                .first()
                .map(String::as_str),
            Some("https://example.com/a.jpg")
        );
    }

    #[test]
    fn enclosure_wins_when_self_comes_first() {
        // A-2: alternate が無く [self, enclosure] の順でも enclosure を選ぶ
        // （順序非依存。self だと全 entry が同一 URL に潰れる）。
        let xml = br#"<?xml version="1.0" encoding="utf-8"?>
<feed xmlns="http://www.w3.org/2005/Atom">
<title>Example</title>
<entry>
<title>An article</title>
<link rel="self" href="https://example.com/feed.xml"/>
<link rel="enclosure" href="https://example.com/e1.mp3" type="audio/mpeg"/>
</entry>
</feed>"#;
        let feed = parse_feed_bytes(xml).unwrap();
        assert_eq!(feed.entries.len(), 1);
        assert_eq!(feed.entries[0].url, "https://example.com/e1.mp3");
    }

    #[test]
    fn media_content_type_with_surrounding_whitespace_is_adopted() {
        // A-3: `type="image/jpeg "`（末尾スペース）は trim して画像扱い。
        let xml = br#"<?xml version="1.0"?>
<rss version="2.0" xmlns:media="http://search.yahoo.com/mrss/"><channel>
<title>T</title>
<link>https://example.com/</link>
<item>
<title>E</title>
<link>https://example.com/e</link>
<media:content url="https://example.com/photo.jpg" type="image/jpeg " medium="image"/>
</item>
</channel></rss>"#;
        let feed = parse_feed_bytes(xml).unwrap();
        assert_eq!(feed.entries.len(), 1);
        assert_eq!(
            feed.entries[0].image_url.as_deref(),
            Some("https://example.com/photo.jpg")
        );
    }

    #[test]
    fn atom_enclosure_type_with_surrounding_whitespace_is_adopted() {
        // A-3: Atom enclosure の `type=" image/png "`（前後スペース）も画像扱い。
        let xml = br#"<?xml version="1.0" encoding="utf-8"?>
<feed xmlns="http://www.w3.org/2005/Atom">
<title>T</title>
<entry>
<title>E</title>
<link rel="alternate" href="https://example.com/e"/>
<link rel="enclosure" href="https://example.com/img.png" type=" image/png " length="100"/>
</entry>
</feed>"#;
        let feed = parse_feed_bytes(xml).unwrap();
        assert_eq!(feed.entries.len(), 1);
        assert_eq!(
            feed.entries[0].image_url.as_deref(),
            Some("https://example.com/img.png")
        );
    }

    #[test]
    fn body_img_falls_through_invalid_first_src() {
        // A-1: 本文先頭の src が normalize で落ちても（data: の計測ピクセル等）、
        // 後続の正規画像を拾う。
        let xml = br#"<?xml version="1.0"?>
<rss version="2.0"><channel>
<title>T</title>
<link>https://example.com/</link>
<item>
<title>E</title>
<link>https://example.com/e</link>
<description>&lt;img src="data:image/png;base64,AAAA"&gt;&lt;img src="https://example.com/real.jpg"&gt;</description>
</item>
</channel></rss>"#;
        let feed = parse_feed_bytes(xml).unwrap();
        assert_eq!(feed.entries.len(), 1);
        assert_eq!(
            feed.entries[0].image_url.as_deref(),
            Some("https://example.com/real.jpg")
        );
    }

    #[test]
    fn img_in_html_comment_is_ignored() {
        // A-4: コメント内の <img> は採用せず、後続の実 img を拾う。
        assert_eq!(
            super::img_srcs(
                r#"<!-- <img src="https://tracker.example/pixel.gif"> --><img src="https://example.com/real.jpg">"#
            )
            .first()
            .map(String::as_str),
            Some("https://example.com/real.jpg")
        );
        assert!(
            super::img_srcs(r#"<!-- <img src="https://tracker.example/pixel.gif"> -->"#).is_empty()
        );
        // 終端の無いコメント以降にタグは成立し得ないので空。
        assert!(
            super::img_srcs(r#"<!-- <img src="https://tracker.example/pixel.gif">"#).is_empty()
        );
    }

    #[test]
    fn media_content_wins_over_body_img() {
        // B-1: step2（media:content）と step4（本文 img）の優先順位を固定。
        // thumbnail / enclosure 無しで両方ある場合、media:content が勝つ。
        let xml = br#"<?xml version="1.0"?>
<rss version="2.0" xmlns:media="http://search.yahoo.com/mrss/"><channel>
<title>T</title>
<link>https://example.com/</link>
<item>
<title>E</title>
<link>https://example.com/e</link>
<media:content url="https://example.com/content.jpg" type="image/jpeg" medium="image"/>
<description>&lt;img src="https://example.com/body.jpg"&gt;</description>
</item>
</channel></rss>"#;
        let feed = parse_feed_bytes(xml).unwrap();
        assert_eq!(feed.entries.len(), 1);
        assert_eq!(
            feed.entries[0].image_url.as_deref(),
            Some("https://example.com/content.jpg")
        );
    }

    #[test]
    fn typeless_media_content_extension_coverage() {
        // B-2: typeless 経路の拡張子リスト（jpeg/png/gif/webp/avif）を固定し、
        // svg 除外（セキュリティ意図）を回帰ガードする。
        for (name, url) in [
            ("jpeg", "https://example.com/a.jpeg"),
            ("png", "https://example.com/a.png"),
            ("gif", "https://example.com/a.gif"),
            ("webp", "https://example.com/a.webp"),
            ("avif", "https://example.com/a.avif"),
        ] {
            let xml = format!(
                "<?xml version=\"1.0\"?>\n<rss version=\"2.0\" xmlns:media=\"http://search.yahoo.com/mrss/\"><channel>\n<title>T</title>\n<link>https://example.com/</link>\n<item>\n<title>E</title>\n<link>https://example.com/e</link>\n<media:content url=\"{url}\"/>\n</item>\n</channel></rss>"
            );
            let feed = parse_feed_bytes(xml.as_bytes()).unwrap();
            assert_eq!(feed.entries.len(), 1, "{name}");
            assert_eq!(feed.entries[0].image_url.as_deref(), Some(url), "{name}");
        }
        let svg = br#"<?xml version="1.0"?>
<rss version="2.0" xmlns:media="http://search.yahoo.com/mrss/"><channel>
<title>T</title>
<link>https://example.com/</link>
<item>
<title>E</title>
<link>https://example.com/e</link>
<media:content url="https://example.com/photo.svg"/>
</item>
</channel></rss>"#;
        let feed = parse_feed_bytes(svg).unwrap();
        assert_eq!(feed.entries.len(), 1);
        assert_eq!(feed.entries[0].image_url, None);
    }

    #[test]
    fn typeless_filename_equal_to_extension_name_is_not_adopted() {
        // B-3: ドット無しでファイル名が拡張子リストと同じ文字列
        // （`/jpg`）は filename.contains('.') ガードで None。
        let xml = br#"<?xml version="1.0"?>
<rss version="2.0" xmlns:media="http://search.yahoo.com/mrss/"><channel>
<title>T</title>
<link>https://example.com/</link>
<item>
<title>E</title>
<link>https://example.com/e</link>
<media:content url="https://example.com/jpg"/>
</item>
</channel></rss>"#;
        let feed = parse_feed_bytes(xml).unwrap();
        assert_eq!(feed.entries.len(), 1);
        assert_eq!(feed.entries[0].image_url, None);
    }

    #[test]
    fn media_content_without_url_does_not_panic() {
        // B-4: url 省略の media:content はスキップしてパニックせず None。
        let xml = br#"<?xml version="1.0"?>
<rss version="2.0" xmlns:media="http://search.yahoo.com/mrss/"><channel>
<title>T</title>
<link>https://example.com/</link>
<item>
<title>E</title>
<link>https://example.com/e</link>
<media:content type="image/jpeg"/>
</item>
</channel></rss>"#;
        let feed = parse_feed_bytes(xml).unwrap();
        assert_eq!(feed.entries.len(), 1);
        assert_eq!(feed.entries[0].image_url, None);
    }

    #[test]
    fn single_link_entries_are_kept() {
        // B-5: alternate 無しで link が単一（self のみ／enclosure のみ）でも
        // エントリを落とさず、その URL を使う。
        let xml = br#"<?xml version="1.0" encoding="utf-8"?>
<feed xmlns="http://www.w3.org/2005/Atom">
<title>T</title>
<entry>
<title>Self only</title>
<link rel="self" href="https://example.com/feed.xml"/>
</entry>
<entry>
<title>Enclosure only</title>
<link rel="enclosure" href="https://example.com/e.mp3" type="audio/mpeg"/>
</entry>
</feed>"#;
        let feed = parse_feed_bytes(xml).unwrap();
        assert_eq!(feed.entries.len(), 2);
        assert_eq!(feed.entries[0].url, "https://example.com/feed.xml");
        assert_eq!(feed.entries[1].url, "https://example.com/e.mp3");
    }

    #[test]
    fn url_path_has_image_extension_handles_relative_urls() {
        // B-6: 公開経路から到達不能な相対 URL フォールバックを直接固定する。
        assert!(super::url_path_has_image_extension("a.jpg"));
        assert!(super::url_path_has_image_extension("a.jpg?x=1"));
        assert!(super::url_path_has_image_extension("a.jpg#f"));
        assert!(super::url_path_has_image_extension("dir/b.png"));
        assert!(!super::url_path_has_image_extension("a.svg"));
        assert!(!super::url_path_has_image_extension("noext"));
    }
}
