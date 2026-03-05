use anyhow::Result;
use chrono::DateTime;
use serde::Deserialize;
use spin_sdk::http::{send, IntoResponse, Method, Request, Response};
use spin_sdk::http_component;

// --- Data models matching server REST API JSON responses ---

#[derive(Debug, Deserialize)]
struct Feed {
    id: String,
    url: String,
    title: Option<String>,
}

#[derive(Debug, Deserialize)]
struct Article {
    id: String,
    url: String,
    title: String,
    content: Option<String>,
    author: Option<String>,
    published_at: Option<i64>,
}

#[derive(Debug, Deserialize)]
struct Stats {
    feeds: i64,
    unread: i64,
}

// --- Entry point ---

#[http_component]
async fn handle(req: Request) -> Result<impl IntoResponse> {
    let full_uri = req.uri().to_string();
    let path = full_uri.split('?').next().unwrap_or(&full_uri).to_string();
    let method = req.method().to_string();

    match (method.as_str(), path.as_str()) {
        ("GET", "/") => index_page().await,
        ("GET", "/partial/feeds") => partial_feeds().await,
        ("GET", "/partial/articles") => partial_articles(&full_uri).await,
        ("GET", p) if p.starts_with("/partial/article/") => {
            let id = &p["/partial/article/".len()..];
            partial_article(id).await
        }
        ("GET", "/partial/stats") => partial_stats().await,
        ("POST", "/feeds") => add_feed(&req).await,
        ("DELETE", p) if p.starts_with("/feeds/") => {
            let id = &p["/feeds/".len()..];
            delete_feed(id).await
        }
        ("POST", "/articles/read-all") => mark_all_read().await,
        ("POST", p) if p.starts_with("/articles/") && p.ends_with("/read") => {
            let segments: Vec<&str> = p.split('/').collect();
            let id = segments.get(2).copied().unwrap_or("");
            mark_read(id).await
        }
        ("POST", "/import/opml") => import_opml(&req).await,
        _ => Ok(Response::builder().status(404).body("Not Found").build()),
    }
}

// --- Helpers ---

fn api_base() -> String {
    spin_sdk::variables::get("api_url").unwrap_or_else(|_| String::new())
}

fn esc(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
        .replace('\'', "&#39;")
}

fn fmt_date(epoch: Option<i64>) -> String {
    epoch
        .and_then(|ts| DateTime::from_timestamp(ts, 0))
        .map(|dt| dt.format("%b %d, %Y").to_string())
        .unwrap_or_default()
}

fn parse_query(uri: &str) -> std::collections::HashMap<String, String> {
    let query = uri.splitn(2, '?').nth(1).unwrap_or("");
    query
        .split('&')
        .filter_map(|pair| {
            let mut kv = pair.splitn(2, '=');
            let k = kv.next()?.to_string();
            let v = kv.next().unwrap_or("").to_string();
            if k.is_empty() {
                None
            } else {
                Some((k, v))
            }
        })
        .collect()
}

async fn api_get(path: &str) -> Result<Vec<u8>> {
    let url = format!("{}{}", api_base(), path);
    let req = Request::new(Method::Get, url);
    let resp: Response = send(req).await?;
    Ok(resp.into_body())
}

async fn api_post_raw(path: &str, body: Vec<u8>, content_type: &str) -> Result<Response> {
    let url = format!("{}{}", api_base(), path);
    let mut builder = Request::builder();
    builder
        .method(Method::Post)
        .uri(url)
        .header("content-type", content_type)
        .body(body);
    Ok(send(builder.build()).await?)
}

async fn api_post_empty(path: &str) -> Result<Response> {
    let url = format!("{}{}", api_base(), path);
    let mut builder = Request::builder();
    builder.method(Method::Post).uri(url);
    Ok(send(builder.build()).await?)
}

async fn api_delete(path: &str) -> Result<Response> {
    let url = format!("{}{}", api_base(), path);
    let mut builder = Request::builder();
    builder.method(Method::Delete).uri(url);
    Ok(send(builder.build()).await?)
}

fn html_ok(body: impl Into<String>) -> Result<Response> {
    Ok(Response::builder()
        .status(200)
        .header("content-type", "text/html; charset=utf-8")
        .body(body.into())
        .build())
}

fn html_err(msg: &str) -> Result<Response> {
    Ok(Response::builder()
        .status(200)
        .header("content-type", "text/html; charset=utf-8")
        .body(format!("<div class=\"alert-error\">{}</div>", esc(msg)))
        .build())
}

fn url_decode(s: &str) -> String {
    let mut result = String::with_capacity(s.len());
    let bytes = s.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'+' {
            result.push(' ');
            i += 1;
        } else if bytes[i] == b'%' && i + 2 < bytes.len() {
            if let (Some(h), Some(l)) = (hex_digit(bytes[i + 1]), hex_digit(bytes[i + 2])) {
                result.push(char::from(h * 16 + l));
                i += 3;
            } else {
                result.push('%');
                i += 1;
            }
        } else {
            result.push(bytes[i] as char);
            i += 1;
        }
    }
    result
}

fn hex_digit(b: u8) -> Option<u8> {
    match b {
        b'0'..=b'9' => Some(b - b'0'),
        b'a'..=b'f' => Some(b - b'a' + 10),
        b'A'..=b'F' => Some(b - b'A' + 10),
        _ => None,
    }
}

fn sanitize_html(input: &str) -> String {
    let mut result = String::with_capacity(input.len());
    let mut i = 0;
    let bytes = input.as_bytes();
    while i < bytes.len() {
        if bytes[i] == b'<' {
            let tag_start = i + 1;
            let tag_end = match input[tag_start..].find('>') {
                Some(p) => tag_start + p,
                None => {
                    result.push_str(&esc(&input[i..]));
                    break;
                }
            };
            let tag_content = &input[tag_start..tag_end];
            let tag_name = tag_content
                .split_whitespace()
                .next()
                .unwrap_or("")
                .trim_start_matches('/')
                .to_ascii_lowercase();
            match tag_name.as_str() {
                "script" | "style" | "iframe" | "object" | "embed" | "form" | "input"
                | "textarea" | "button" | "select" => {
                    if !tag_content.starts_with('/') {
                        let close = format!("</{tag_name}>");
                        let after = &input[tag_end + 1..];
                        if let Some(end) = after.to_ascii_lowercase().find(&close) {
                            i = tag_end + 1 + end + close.len();
                        } else {
                            i = tag_end + 1;
                        }
                    } else {
                        i = tag_end + 1;
                    }
                }
                _ => {
                    let cleaned = remove_event_attrs(tag_content);
                    result.push('<');
                    result.push_str(&cleaned);
                    result.push('>');
                    i = tag_end + 1;
                }
            }
        } else {
            result.push(bytes[i] as char);
            i += 1;
        }
    }
    result
}

fn remove_event_attrs(tag: &str) -> String {
    let mut result = String::with_capacity(tag.len());
    let mut i = 0;
    let bytes = tag.as_bytes();
    while i < bytes.len() {
        if (i == 0 || bytes[i - 1].is_ascii_whitespace()) && i + 2 < bytes.len() {
            let rest = &tag[i..];
            let lower = rest.to_ascii_lowercase();
            if lower.starts_with("on") && rest[2..].starts_with(|c: char| c.is_ascii_alphabetic())
            {
                if let Some(eq) = rest.find('=') {
                    let after_eq = &rest[eq + 1..];
                    let skip = if after_eq.starts_with('"') {
                        after_eq[1..].find('"').map(|p| eq + 1 + 1 + p + 1)
                    } else if after_eq.starts_with('\'') {
                        after_eq[1..].find('\'').map(|p| eq + 1 + 1 + p + 1)
                    } else {
                        after_eq
                            .find(|c: char| c.is_ascii_whitespace() || c == '>' || c == '/')
                            .or(Some(after_eq.len()))
                            .map(|p| eq + 1 + p)
                    };
                    if let Some(skip_len) = skip {
                        i += skip_len;
                        continue;
                    }
                }
            }
            if lower.starts_with("href")
                && rest
                    .get(4..)
                    .and_then(|s| s.find('=').map(|p| &s[..=p]))
                    .map(|s| s.trim().starts_with('='))
                    .unwrap_or(false)
            {
                if let Some(eq) = rest.find('=') {
                    let after_eq = rest[eq + 1..].trim_start();
                    let val = if after_eq.starts_with('"') {
                        after_eq[1..].find('"').map(|p| &after_eq[1..1 + p])
                    } else if after_eq.starts_with('\'') {
                        after_eq[1..].find('\'').map(|p| &after_eq[1..1 + p])
                    } else {
                        after_eq
                            .find(|c: char| c.is_ascii_whitespace() || c == '>')
                            .map(|p| &after_eq[..p])
                    };
                    if let Some(v) = val {
                        let trimmed = v.trim().to_ascii_lowercase();
                        if trimmed.starts_with("javascript:") || trimmed.starts_with("data:") {
                            if let Some(skip) = rest.find(|c: char| c.is_ascii_whitespace()) {
                                i += skip;
                                continue;
                            }
                        }
                    }
                }
            }
        }
        result.push(bytes[i] as char);
        i += 1;
    }
    result
}

fn parse_form_body(body: &[u8]) -> std::collections::HashMap<String, String> {
    body.split(|&b| b == b'&')
        .filter_map(|pair| {
            let s = std::str::from_utf8(pair).ok()?;
            let mut kv = s.splitn(2, '=');
            let k = url_decode(kv.next()?);
            let v = url_decode(kv.next().unwrap_or(""));
            Some((k, v))
        })
        .collect()
}

// --- HTML rendering ---

fn render_feed_nav(feeds: &[Feed]) -> String {
    // Use ## raw strings since HTML attributes contain "#id" patterns
    let mut html = String::from(r##"<nav id="feed-nav">
  <div class="feed-section-title">Feeds</div>
  <div class="feed-item"
       hx-get="/partial/articles?unread=true"
       hx-target="#article-list"
       hx-swap="innerHTML">All Unread</div>
  <div class="feed-item"
       hx-get="/partial/articles"
       hx-target="#article-list"
       hx-swap="innerHTML">All Articles</div>"##);

    for feed in feeds {
        let name = feed
            .title
            .as_deref()
            .filter(|s| !s.is_empty())
            .unwrap_or(&feed.url);
        let escaped_name = esc(name);
        let escaped_id = esc(&feed.id);
        html.push_str(&format!(
            r##"
  <div class="feed-item">
    <span class="feed-name"
          hx-get="/partial/articles?feed_id={id}"
          hx-target="#article-list"
          hx-swap="innerHTML">{name}</span>
    <button class="btn-icon"
            hx-delete="/feeds/{id}"
            hx-target="#feed-nav"
            hx-swap="outerHTML"
            hx-confirm="Delete this feed?"
            title="Delete feed">&#x2715;</button>
  </div>"##,
            id = escaped_id,
            name = escaped_name,
        ));
    }

    html.push_str("\n</nav>");
    html
}

fn render_article_card(article: &Article) -> String {
    let preview = article
        .content
        .as_deref()
        .map(|c| {
            let mut text = String::new();
            let mut in_tag = false;
            for ch in c.chars() {
                match ch {
                    '<' => in_tag = true,
                    '>' => in_tag = false,
                    _ if !in_tag => text.push(ch),
                    _ => {}
                }
                if text.len() >= 200 {
                    break;
                }
            }
            text
        })
        .unwrap_or_default();

    let author = article.author.as_deref().unwrap_or("");
    let date = fmt_date(article.published_at);
    let meta = match (author.is_empty(), date.is_empty()) {
        (false, false) => format!("{} &middot; {}", esc(author), date),
        (false, true) => esc(author),
        (true, false) => date,
        (true, true) => String::new(),
    };

    let id = esc(&article.id);
    let url = esc(&article.url);
    let title = esc(&article.title);
    let preview_esc = esc(preview.trim());

    let mut html = format!(
        r##"<article id="article-{id}" class="article-card">
  <h3 class="article-title">
    <a href="{url}" target="_blank" rel="noopener">{title}</a>
  </h3>"##
    );

    if !meta.is_empty() {
        html.push_str(&format!("\n  <div class=\"article-meta\">{meta}</div>"));
    }
    if !preview_esc.is_empty() {
        html.push_str(&format!(
            "\n  <div class=\"article-preview\">{preview_esc}</div>"
        ));
    }

    html.push_str(&format!(
        r##"
  <div class="article-actions">
    <button class="btn btn-sm"
            hx-post="/articles/{id}/read"
            hx-target="#article-{id}"
            hx-swap="outerHTML">Mark Read</button>
    <button class="btn btn-sm btn-secondary"
            hx-get="/partial/article/{id}"
            hx-target="#article-list"
            hx-swap="innerHTML">Read More</button>
  </div>
</article>"##,
        id = id,
    ));

    html
}

// --- Route handlers ---

async fn index_page() -> Result<Response> {
    // Build the full HTML page. CSS is inlined via format!.
    let mut html = String::new();
    html.push_str(r##"<!DOCTYPE html>
<html lang="en">
<head>
  <meta charset="UTF-8">
  <meta name="viewport" content="width=device-width, initial-scale=1.0">
  <title>home-rss</title>
  <script src="https://unpkg.com/htmx.org@2.0.4" integrity="sha384-HGfztofotfshcF7+8n44JQL2oJmowVChPTg48S+jvZoztPfvwD79OC/LTtG6dMp+" crossorigin="anonymous"></script>
  <style>"##);
    html.push_str(CSS);
    html.push_str(r##"</style>
</head>
<body>
<header>
  <h1>home-rss</h1>
  <span id="stats"
        hx-get="/partial/stats"
        hx-trigger="load, every 60s"></span>
  <div class="header-actions">
    <button class="btn" onclick="document.getElementById('add-feed-dialog').showModal()">+ Feed</button>
    <button class="btn" onclick="document.getElementById('import-dialog').showModal()">Import OPML</button>
    <button class="btn btn-primary"
            hx-post="/articles/read-all"
            hx-target="#article-list"
            hx-swap="innerHTML"
            hx-confirm="Mark all articles as read?">Mark All Read</button>
  </div>
</header>

<dialog id="add-feed-dialog">
  <form hx-post="/feeds"
        hx-target="#feed-nav"
        hx-swap="outerHTML"
        hx-on:htmx:after-request="if(event.detail.successful &amp;&amp; !event.detail.xhr.getResponseHeader('HX-Retarget')){document.getElementById('add-feed-dialog').close(); this.reset();}">
    <h2>Add Feed</h2>
    <input type="url" name="url" placeholder="https://example.com/feed.xml" required autocomplete="off">
    <div id="add-feed-error"></div>
    <div class="dialog-actions">
      <button type="submit" class="btn btn-primary">Add</button>
      <button type="button" class="btn" onclick="document.getElementById('add-feed-dialog').close()">Cancel</button>
    </div>
  </form>
</dialog>

<dialog id="import-dialog">
  <form hx-post="/import/opml"
        hx-target="#import-result"
        hx-on:htmx:after-request="if(event.detail.successful){htmx.trigger(document.getElementById('feed-nav'),'load');}">
    <h2>Import OPML</h2>
    <p class="hint">Paste OPML XML content:</p>
    <textarea name="opml" rows="8" placeholder="Paste OPML XML here..."></textarea>
    <div id="import-result"></div>
    <div class="dialog-actions">
      <button type="submit" class="btn btn-primary">Import</button>
      <button type="button" class="btn" onclick="document.getElementById('import-dialog').close()">Cancel</button>
    </div>
  </form>
</dialog>

<div class="layout">
  <aside>
    <div id="feed-nav"
         hx-get="/partial/feeds"
         hx-trigger="load"
         hx-swap="outerHTML">Loading feeds...</div>
  </aside>
  <main>
    <div id="article-list"
         hx-get="/partial/articles?unread=true"
         hx-trigger="load">Loading articles...</div>
  </main>
</div>
</body>
</html>"##);

    html_ok(html)
}

async fn partial_feeds() -> Result<Response> {
    let body = match api_get("/api/feeds").await {
        Ok(b) => b,
        Err(e) => return html_err(&format!("Failed to load feeds: {e}")),
    };
    let feeds: Vec<Feed> = match serde_json::from_slice(&body) {
        Ok(f) => f,
        Err(e) => return html_err(&format!("Failed to parse feeds: {e}")),
    };
    html_ok(render_feed_nav(&feeds))
}

async fn partial_articles(uri: &str) -> Result<Response> {
    let params = parse_query(uri);
    let feed_id = params.get("feed_id").cloned();
    let unread = params.get("unread").map(|v| v == "true").unwrap_or(false);

    let api_path = match (&feed_id, unread) {
        (Some(id), true) => format!("/api/articles?feed_id={id}&unread=true"),
        (Some(id), false) => format!("/api/articles?feed_id={id}"),
        (None, true) => "/api/articles?unread=true".to_string(),
        (None, false) => "/api/articles".to_string(),
    };

    let body = match api_get(&api_path).await {
        Ok(b) => b,
        Err(e) => return html_err(&format!("Failed to load articles: {e}")),
    };
    let articles: Vec<Article> = match serde_json::from_slice(&body) {
        Ok(a) => a,
        Err(e) => return html_err(&format!("Failed to parse articles: {e}")),
    };

    if articles.is_empty() {
        return html_ok("<div class=\"empty\">No articles found.</div>");
    }

    let html: String = articles.iter().map(render_article_card).collect();
    html_ok(html)
}

async fn partial_article(id: &str) -> Result<Response> {
    let body = match api_get("/api/articles").await {
        Ok(b) => b,
        Err(e) => return html_err(&format!("Failed to load article: {e}")),
    };
    let articles: Vec<Article> = match serde_json::from_slice(&body) {
        Ok(a) => a,
        Err(e) => return html_err(&format!("Failed to parse articles: {e}")),
    };

    let article = match articles.into_iter().find(|a| a.id == id) {
        Some(a) => a,
        None => return html_err("Article not found"),
    };

    let content_html = article
        .content
        .as_deref()
        .map(|c| format!("<div class=\"article-content\">{}</div>", sanitize_html(c)))
        .unwrap_or_else(|| "<p class=\"empty\">No content available.</p>".to_string());

    let author = article.author.as_deref().unwrap_or("");
    let date = fmt_date(article.published_at);
    let meta = match (author.is_empty(), date.is_empty()) {
        (false, false) => format!("{} &middot; {}", esc(author), date),
        (false, true) => esc(author),
        (true, false) => date,
        (true, true) => String::new(),
    };

    let esc_id = esc(id);
    let mut html = format!(
        r##"<div class="article-detail">
  <div class="article-detail-header">
    <button class="btn btn-sm"
            hx-get="/partial/articles?unread=true"
            hx-target="#article-list"
            hx-swap="innerHTML">&#x2190; Back</button>
    <button class="btn btn-sm btn-primary"
            hx-post="/articles/{id}/read"
            hx-target="#article-{id}"
            hx-swap="outerHTML"
            hx-on:htmx:after-request="htmx.ajax('GET','/partial/articles?unread=true',{{target:'#article-list',swap:'innerHTML'}})">Mark Read</button>
  </div>
  <h2><a href="{url}" target="_blank" rel="noopener">{title}</a></h2>"##,
        id = esc_id,
        url = esc(&article.url),
        title = esc(&article.title),
    );

    if !meta.is_empty() {
        html.push_str(&format!("\n  <div class=\"article-meta\">{meta}</div>"));
    }
    html.push_str(&format!("\n  {content_html}\n</div>"));

    html_ok(html)
}

async fn partial_stats() -> Result<Response> {
    let body = match api_get("/api/stats").await {
        Ok(b) => b,
        Err(_) => return html_ok(""),
    };
    let stats: Stats = match serde_json::from_slice(&body) {
        Ok(s) => s,
        Err(_) => return html_ok(""),
    };
    html_ok(format!(
        "<span>{} unread &middot; {} feeds</span>",
        stats.unread, stats.feeds
    ))
}

fn html_err_retarget(msg: &str, target: &str) -> Result<Response> {
    Ok(Response::builder()
        .status(200)
        .header("content-type", "text/html; charset=utf-8")
        .header("HX-Retarget", target)
        .header("HX-Reswap", "innerHTML")
        .body(format!("<div class=\"alert-error\">{}</div>", esc(msg)))
        .build())
}

async fn add_feed(req: &Request) -> Result<Response> {
    let params = parse_form_body(req.body());
    let url = match params.get("url") {
        Some(u) if !u.is_empty() => u.clone(),
        _ => return html_err_retarget("URL is required", "#add-feed-error"),
    };

    let json = format!(r#"{{"url":{}}}"#, serde_json::to_string(&url)?);
    let resp = match api_post_raw("/api/feeds", json.into_bytes(), "application/json").await {
        Ok(r) => r,
        Err(e) => {
            return html_err_retarget(
                &format!("Failed to add feed: {e}"),
                "#add-feed-error",
            )
        }
    };

    if *resp.status() >= 400 {
        let msg = String::from_utf8_lossy(resp.into_body().as_slice()).to_string();
        return html_err_retarget(&format!("Server error: {msg}"), "#add-feed-error");
    }

    partial_feeds().await
}

async fn delete_feed(id: &str) -> Result<Response> {
    let resp = match api_delete(&format!("/api/feeds/{id}")).await {
        Ok(r) => r,
        Err(e) => return html_err(&format!("Failed to delete feed: {e}")),
    };

    if *resp.status() >= 400 {
        return html_err("Failed to delete feed");
    }

    partial_feeds().await
}

async fn mark_read(id: &str) -> Result<Response> {
    let resp = match api_post_empty(&format!("/api/articles/{id}/read")).await {
        Ok(r) => r,
        Err(e) => return html_err(&format!("Failed to mark as read: {e}")),
    };

    if *resp.status() >= 400 {
        return html_err("Failed to mark article as read");
    }

    // Return collapsed article element (removes it from unread view)
    html_ok(format!(
        "<article id=\"article-{}\" class=\"article-card read\"></article>",
        esc(id)
    ))
}

async fn mark_all_read() -> Result<Response> {
    let resp = match api_post_empty("/api/articles/read-all").await {
        Ok(r) => r,
        Err(e) => return html_err(&format!("Failed to mark all as read: {e}")),
    };

    if *resp.status() >= 400 {
        return html_err("Failed to mark all as read");
    }

    html_ok("<div class=\"empty\">All articles marked as read.</div>")
}

async fn import_opml(req: &Request) -> Result<Response> {
    let params = parse_form_body(req.body());
    let opml_content = match params.get("opml") {
        Some(c) if !c.is_empty() => c.clone(),
        _ => return html_err("OPML content is required"),
    };

    let resp = match api_post_raw(
        "/api/import/opml",
        opml_content.into_bytes(),
        "application/xml",
    )
    .await
    {
        Ok(r) => r,
        Err(e) => return html_err(&format!("Failed to import OPML: {e}")),
    };

    if *resp.status() >= 400 {
        let msg = String::from_utf8_lossy(resp.into_body().as_slice()).to_string();
        return html_err(&format!("Import failed: {msg}"));
    }

    let body_bytes = resp.into_body();
    let result: serde_json::Value = serde_json::from_slice(&body_bytes).unwrap_or_default();
    let imported = result.get("imported").and_then(|v| v.as_i64()).unwrap_or(0);

    html_ok(format!(
        "<div class=\"alert-success\">Imported {imported} feed(s).</div>"
    ))
}

// --- CSS ---

const CSS: &str = r#"
*, *::before, *::after { box-sizing: border-box; margin: 0; padding: 0; }
body { font-family: system-ui, -apple-system, sans-serif; background: #f0f2f5; display: flex; flex-direction: column; height: 100vh; overflow: hidden; }

header { background: #1e293b; color: #f8fafc; padding: 0.6rem 1rem; display: flex; align-items: center; gap: 1rem; flex-shrink: 0; }
header h1 { font-size: 1.1rem; font-weight: 700; letter-spacing: -0.02em; }
#stats { font-size: 0.8rem; color: #94a3b8; }
.header-actions { margin-left: auto; display: flex; gap: 0.5rem; }

.layout { display: flex; flex: 1; overflow: hidden; }
aside { width: 220px; flex-shrink: 0; background: #fff; border-right: 1px solid #e2e8f0; overflow-y: auto; }
main { flex: 1; overflow-y: auto; padding: 1rem; }

#feed-nav { display: flex; flex-direction: column; }
.feed-section-title { padding: 0.75rem 1rem 0.25rem; font-size: 0.7rem; font-weight: 600; text-transform: uppercase; letter-spacing: 0.08em; color: #94a3b8; }
.feed-item { display: flex; align-items: center; padding: 0.45rem 0.75rem; font-size: 0.85rem; color: #475569; border-left: 3px solid transparent; gap: 0.4rem; }
.feed-item:hover { background: #f8fafc; }
.feed-item.active { background: #eff6ff; border-left-color: #3b82f6; color: #1d4ed8; font-weight: 500; }
.feed-name { flex: 1; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; cursor: pointer; }

.article-card { background: #fff; border: 1px solid #e2e8f0; border-radius: 8px; padding: 1rem; margin-bottom: 0.75rem; }
.article-card.read { height: 0; padding: 0; margin: 0; border: none; overflow: hidden; }
.article-title { font-size: 1rem; margin-bottom: 0.25rem; }
.article-title a { color: #1e293b; text-decoration: none; }
.article-title a:hover { color: #3b82f6; text-decoration: underline; }
.article-meta { font-size: 0.75rem; color: #94a3b8; margin-bottom: 0.4rem; }
.article-preview { font-size: 0.85rem; color: #64748b; line-height: 1.5; max-height: 72px; overflow: hidden; margin-bottom: 0.5rem; }
.article-actions { display: flex; gap: 0.5rem; }
.article-detail { background: #fff; border-radius: 8px; padding: 1.5rem; }
.article-detail-header { display: flex; gap: 0.5rem; margin-bottom: 1rem; }
.article-detail h2 { font-size: 1.25rem; margin-bottom: 0.5rem; }
.article-detail h2 a { color: #1e293b; text-decoration: none; }
.article-detail h2 a:hover { text-decoration: underline; }
.article-content { font-size: 0.9rem; line-height: 1.7; color: #334155; margin-top: 1rem; }

.btn { padding: 0.3rem 0.75rem; font-size: 0.8rem; border: 1px solid #cbd5e1; border-radius: 5px; background: #fff; color: #475569; cursor: pointer; white-space: nowrap; }
.btn:hover { background: #f8fafc; }
.btn-primary { background: #3b82f6; color: #fff; border-color: #3b82f6; }
.btn-primary:hover { background: #2563eb; }
.btn-secondary { color: #6b7280; }
.btn-sm { padding: 0.2rem 0.5rem; font-size: 0.75rem; }
.btn-icon { background: none; border: none; color: #cbd5e1; cursor: pointer; font-size: 1rem; padding: 0 0.2rem; line-height: 1; }
.btn-icon:hover { color: #ef4444; }

dialog { border-radius: 10px; border: 1px solid #e2e8f0; box-shadow: 0 10px 40px rgba(0,0,0,0.15); padding: 0; min-width: 360px; max-width: 90vw; }
dialog::backdrop { background: rgba(0,0,0,0.4); }
dialog form { padding: 1.5rem; display: flex; flex-direction: column; gap: 0.75rem; }
dialog h2 { font-size: 1.1rem; color: #1e293b; }
dialog input[type=url], dialog textarea { width: 100%; padding: 0.5rem 0.75rem; border: 1px solid #cbd5e1; border-radius: 5px; font-size: 0.9rem; font-family: inherit; }
dialog textarea { resize: vertical; }
.dialog-actions { display: flex; gap: 0.5rem; justify-content: flex-end; }
.hint { font-size: 0.8rem; color: #94a3b8; }

.empty { color: #94a3b8; text-align: center; padding: 3rem 1rem; font-size: 0.95rem; }
.alert-error { background: #fef2f2; border: 1px solid #fecaca; color: #dc2626; padding: 0.5rem 0.75rem; border-radius: 5px; font-size: 0.85rem; }
.alert-success { background: #f0fdf4; border: 1px solid #bbf7d0; color: #16a34a; padding: 0.5rem 0.75rem; border-radius: 5px; font-size: 0.85rem; }

@media (max-width: 640px) {
  header { flex-wrap: wrap; gap: 0.4rem; padding: 0.5rem; }
  .header-actions { margin-left: 0; width: 100%; flex-wrap: wrap; }
  .layout { flex-direction: column; overflow-y: auto; }
  aside { width: 100%; height: auto; max-height: 160px; border-right: none; border-bottom: 1px solid #e2e8f0; }
  #feed-nav { flex-direction: row; flex-wrap: wrap; overflow-x: auto; padding: 0.25rem; }
  .feed-section-title { display: none; }
  .feed-item { border-left: none; border-bottom: 3px solid transparent; padding: 0.35rem 0.6rem; }
  .feed-item.active { border-bottom-color: #3b82f6; border-left: none; }
  main { overflow-y: unset; padding: 0.5rem; }
  dialog { min-width: unset; width: 95vw; }
}
"#;
