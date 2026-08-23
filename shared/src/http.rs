use bytes::Bytes;
use spin_sdk::http::{FullBody, Response, StatusCode};

pub type Resp = Response<FullBody<Bytes>>;

pub fn empty(status: StatusCode) -> Resp {
    Response::builder()
        .status(status)
        .body(FullBody::new(Bytes::new()))
        .unwrap()
}

pub fn text(status: StatusCode, body: impl Into<Bytes>) -> Resp {
    Response::builder()
        .status(status)
        .header("content-type", "text/plain; charset=utf-8")
        .body(FullBody::new(body.into()))
        .unwrap()
}

pub fn json(status: StatusCode, body: impl Into<Bytes>) -> Resp {
    Response::builder()
        .status(status)
        .header("content-type", "application/json")
        .body(FullBody::new(body.into()))
        .unwrap()
}
