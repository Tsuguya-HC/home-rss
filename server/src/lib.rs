use anyhow::Result;
use spin_sdk::http::{IntoResponse, Params, Request, Response, Router};
use spin_sdk::http_component;

#[http_component]
fn handle(req: Request) -> Result<impl IntoResponse> {
    let mut router = Router::default();
    router.get("/api/feeds", list_feeds);
    router.post("/api/feeds", add_feed);
    router.delete("/api/feeds/:id", delete_feed);
    router.get("/api/articles", list_articles);
    router.post("/api/articles/:id/read", mark_read);
    router.post("/api/articles/read-all", mark_all_read);
    router.post("/api/import/opml", import_opml);
    router.get("/api/stats", get_stats);
    Ok(router.handle(req))
}

fn json_ok(body: impl Into<String>) -> Result<Response> {
    Ok(Response::builder()
        .status(200)
        .header("content-type", "application/json")
        .body(body.into())
        .build())
}

fn not_implemented() -> Result<Response> {
    Ok(Response::builder().status(501).body(()).build())
}

fn list_feeds(_req: Request, _params: Params) -> Result<Response> {
    // TODO: Query feeds from DB (#7)
    json_ok("[]")
}

fn add_feed(_req: Request, _params: Params) -> Result<Response> {
    // TODO: Parse {url} body, insert into DB (#7)
    not_implemented()
}

fn delete_feed(_req: Request, _params: Params) -> Result<Response> {
    // TODO: Delete feed by :id from DB (#7)
    not_implemented()
}

fn list_articles(_req: Request, _params: Params) -> Result<Response> {
    // TODO: Query articles with optional ?feed_id=&unread=true (#7)
    json_ok("[]")
}

fn mark_read(_req: Request, _params: Params) -> Result<Response> {
    // TODO: Insert into read_status for :id (#7)
    not_implemented()
}

fn mark_all_read(_req: Request, _params: Params) -> Result<Response> {
    // TODO: Insert into read_status for all articles (#7)
    not_implemented()
}

fn import_opml(_req: Request, _params: Params) -> Result<Response> {
    // TODO: Parse OPML body, bulk-insert feeds (#7)
    not_implemented()
}

fn get_stats(_req: Request, _params: Params) -> Result<Response> {
    // TODO: Return unread counts etc. (#7)
    json_ok(r#"{"unread":0,"feeds":0}"#)
}
