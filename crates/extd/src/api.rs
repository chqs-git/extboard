//! The HTTP surface. Phase 2.
//!
//!   GET  /api/spaces          list
//!   GET  /api/spaces/{id}     the document + ETag
//!   PUT  /api/spaces/{id}     whole doc, If-Match
//!

use crate::store::{Store, StoreError};
use axum::extract::{Path, State};
use axum::http::header::{CONTENT_TYPE, ETAG, IF_NONE_MATCH};
use axum::http::{HeaderMap, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::routing::get;
use axum::{Json, Router};
use extboard_core::rev;
use std::net::{Ipv4Addr, SocketAddr};
use std::sync::Arc;
use tokio::net::TcpListener;

type AppState = Arc<Store>; // Shared state of Store

pub async fn serve(port: u16) -> Result<(), Box<dyn std::error::Error>> {
    let state: AppState = Arc::new(Store::new()?);

    let addr = SocketAddr::from((Ipv4Addr::LOCALHOST, port));
    let listener = TcpListener::bind(addr).await?;
    println!("extd listening on http://{addr}");

    axum::serve(listener, router(state))
        .with_graceful_shutdown(shutdown_signal())
        .await?;
    Ok(())
}

fn router(state: AppState) -> Router {
    Router::new()
        .route("/health", get(health))
        .route("/api/spaces", get(list_spaces))
        .route("/api/spaces/{id}", get(get_space))
        .with_state(state)
}

/// GET endpoints
async fn health(State(store): State<AppState>) -> Result<&'static str, StatusCode> {
    match store.list() {
        Ok(_) => Ok("ok"),
        Err(_) => Err(StatusCode::SERVICE_UNAVAILABLE),
    }
}

async fn list_spaces(State(store): State<AppState>) -> Result<Json<Vec<String>>, StoreError> {
    Ok(Json(store.list()?))
}

async fn get_space(
    State(store): State<AppState>,
    Path(id): Path<String>,
    headers: HeaderMap,
) -> Result<Response, StoreError> {
    let canvas = store.space(&id)?.read().await.load()?;

    let etag = format!("\"{}\"", rev(&canvas));

    // The client already holds this version: 304, no body, nothing transferred.
    if headers
        .get(IF_NONE_MATCH)
        .is_some_and(|sent| sent.as_bytes() == etag.as_bytes())
    {
        return Ok((StatusCode::NOT_MODIFIED, [(ETAG, etag)]).into_response());
    }

    Ok((
        [(ETAG, etag), (CONTENT_TYPE, "application/json".to_owned())],
        canvas.to_pretty_string(),
    )
        .into_response())
}

/// Map StoreError -> StatusCode
impl IntoResponse for StoreError {
    fn into_response(self) -> Response {
        let status = match &self {
            // An id we refuse to look up is the caller's mistake, not a
            // missing resource.
            StoreError::BadId(_) => StatusCode::BAD_REQUEST,
            StoreError::NotFound(_) => StatusCode::NOT_FOUND,
            // A corrupt file or an unreadable directory is ours.
            StoreError::Parse { .. } | StoreError::Io(_) => StatusCode::INTERNAL_SERVER_ERROR,
        };
        (
            status,
            Json(serde_json::json!({ "error": self.to_string() })),
        )
            .into_response()
    }
}

async fn shutdown_signal() {
    let _ = tokio::signal::ctrl_c().await;
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::body::Body;
    use axum::http::Request;
    use tower::ServiceExt;

    // `oneshot` feeds one request straight into the Router and returns the
    // response — no socket, no port, no runtime teardown. This is the way to
    // test axum handlers.
    #[tokio::test]
    async fn health_is_ok() {
        let dir = std::env::temp_dir().join("extboard-health-test");
        std::fs::create_dir_all(&dir).unwrap();
        let app = router(Arc::new(Store::at(dir.clone())));

        let response = app
            .oneshot(Request::get("/health").body(Body::empty()).unwrap())
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::OK);
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[tokio::test]
    async fn list_spaces_returns_a_json_array_of_ids() {
        let dir = std::env::temp_dir().join("extboard-list-spaces-test");
        std::fs::create_dir_all(&dir).unwrap();
        for name in ["b.canvas", "a.canvas", "notes.md"] {
            std::fs::write(dir.join(name), "{}").unwrap();
        }
        let app = router(Arc::new(Store::at(dir.clone())));

        let response = app
            .oneshot(Request::get("/api/spaces").body(Body::empty()).unwrap())
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::OK);
        assert_eq!(response.headers()["content-type"], "application/json");
        let body = axum::body::to_bytes(response.into_body(), usize::MAX)
            .await
            .unwrap();
        assert_eq!(body, r#"["a","b"]"#);
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[tokio::test]
    async fn get_space_etag_304_and_404() {
        let dir = std::env::temp_dir().join("extboard-get-space-test");
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("a.canvas"), r#"{"nodes":[],"edges":[]}"#).unwrap();
        let state: AppState = Arc::new(Store::at(dir.clone()));

        let got = router(state.clone())
            .oneshot(Request::get("/api/spaces/a").body(Body::empty()).unwrap())
            .await
            .unwrap();
        assert_eq!(got.status(), StatusCode::OK);
        assert_eq!(got.headers()[CONTENT_TYPE], "application/json");
        let etag = got.headers()[ETAG].to_str().unwrap().to_owned();
        assert!(etag.starts_with('"') && etag.ends_with('"'), "{etag}");

        // Same rev in hand: nothing transferred.
        let cached = router(state.clone())
            .oneshot(
                Request::get("/api/spaces/a")
                    .header(IF_NONE_MATCH, &etag)
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(cached.status(), StatusCode::NOT_MODIFIED);

        // A stale rev still gets the document.
        let stale = router(state.clone())
            .oneshot(
                Request::get("/api/spaces/a")
                    .header(IF_NONE_MATCH, "\"0000000000000000\"")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(stale.status(), StatusCode::OK);

        let missing = router(state.clone())
            .oneshot(
                Request::get("/api/spaces/nope")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(missing.status(), StatusCode::NOT_FOUND);
        assert_eq!(missing.headers()[CONTENT_TYPE], "application/json");

        let bad = router(state)
            .oneshot(
                Request::get("/api/spaces/.hidden")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(bad.status(), StatusCode::BAD_REQUEST);

        std::fs::remove_dir_all(&dir).unwrap();
    }
}
