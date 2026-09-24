//! #145 のサムネイル抽出を、実装の比較で落ちた入力で固定する。公開 API だけを通す。

use home_rss_shared::feed::parse_feed_bytes;

fn rss(item: &str) -> String {
    format!(
        r#"<?xml version="1.0" encoding="UTF-8"?>
<rss version="2.0" xmlns:media="http://search.yahoo.com/mrss/">
<channel><title>t</title><link>https://example.com/</link><description>d</description>
<item><title>item</title><link>https://example.com/post/1</link>
{item}
</item></channel></rss>"#
    )
}

fn body(html: &str) -> String {
    rss(&format!("<description><![CDATA[{html}]]></description>"))
}

fn image(xml: &str) -> Option<String> {
    let feed = parse_feed_bytes(xml.as_bytes()).unwrap();
    assert_eq!(feed.entries.len(), 1);
    feed.entries.into_iter().next().unwrap().image_url
}

#[test]
fn audio_enclosure_without_type_is_not_an_image() {
    let with_length = rss(r#"<enclosure url="https://example.com/a.mp3" length="123"/>"#);
    let bare = rss(r#"<enclosure url="https://example.com/a.mp3"/>"#);
    assert_eq!(image(&with_length), None);
    assert_eq!(image(&bare), None);
}

#[test]
fn data_url_pixel_before_real_image_is_skipped() {
    let xml = body(
        r#"<img src="data:image/gif;base64,R0lGODlhAQABAAAAACw="><img src="https://example.com/real.jpg">"#,
    );
    assert_eq!(image(&xml).as_deref(), Some("https://example.com/real.jpg"));
}

#[test]
fn invalid_top_candidate_falls_through() {
    let to_enclosure = rss(
        r#"<media:thumbnail url="javascript:alert(1)"/><enclosure url="https://example.com/e.jpg" type="image/jpeg" length="1"/>"#,
    );
    let to_body = rss(
        r#"<media:thumbnail url="ftp://example.com/t.jpg"/><description><![CDATA[<img src="https://example.com/i.jpg">]]></description>"#,
    );
    assert_eq!(
        image(&to_enclosure).as_deref(),
        Some("https://example.com/e.jpg")
    );
    assert_eq!(
        image(&to_body).as_deref(),
        Some("https://example.com/i.jpg")
    );
}

#[test]
fn multibyte_text_before_image() {
    let xml =
        body(r#"<p>こんにちは世界。日本語の本文です。</p><img src="https://example.com/ja.jpg">"#);
    assert_eq!(image(&xml).as_deref(), Some("https://example.com/ja.jpg"));
}

#[test]
fn image_inside_html_comment_is_ignored() {
    let xml = body(
        r#"<!-- <img src="https://example.com/c.jpg"> --><img src="https://example.com/r.jpg">"#,
    );
    assert_eq!(image(&xml).as_deref(), Some("https://example.com/r.jpg"));
}

#[test]
fn media_content_without_type_is_an_image() {
    let xml = rss(r#"<media:content url="https://example.com/mc.jpg"/>"#);
    assert_eq!(image(&xml).as_deref(), Some("https://example.com/mc.jpg"));
}

#[test]
fn video_media_content_is_not_an_image() {
    let xml = rss(r#"<media:content url="https://example.com/v.mp4" type="video/mp4"/>"#);
    assert_eq!(image(&xml), None);
}

#[test]
fn thumbnail_wins_over_every_other_candidate() {
    let xml = rss(r#"<media:thumbnail url="https://example.com/t.jpg"/>
<media:content url="https://example.com/mc.jpg" type="image/jpeg"/>
<enclosure url="https://example.com/e.jpg" type="image/jpeg" length="1"/>
<description><![CDATA[<img src="https://example.com/i.jpg">]]></description>"#);
    assert_eq!(image(&xml).as_deref(), Some("https://example.com/t.jpg"));
}

#[test]
fn atom_image_enclosure() {
    let xml = r#"<?xml version="1.0" encoding="utf-8"?>
<feed xmlns="http://www.w3.org/2005/Atom"><title>t</title><id>urn:x</id><updated>2026-09-01T00:00:00Z</updated>
<entry><title>e</title><id>urn:e</id><updated>2026-09-01T00:00:00Z</updated>
<link rel="alternate" href="https://example.com/post/1"/>
<link rel="enclosure" type="image/png" href="https://example.com/at.png"/>
</entry></feed>"#;
    assert_eq!(image(xml).as_deref(), Some("https://example.com/at.png"));
}

#[test]
fn entry_without_any_image() {
    assert_eq!(image(&body("<p>text only</p>")), None);
}
