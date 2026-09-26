//! The HTTP surface. Phase 2.
//!
//!   GET  /api/spaces          list
//!   GET  /api/spaces/{id}     the document + ETag
//!   PUT  /api/spaces/{id}     whole doc, If-Match
//!

use crate::events::{Events, events, watch};
use crate::store::{Store, StoreError};
use axum::extract::{Path, State};
use axum::http::header::{CONTENT_TYPE, ETAG, IF_MATCH, IF_NONE_MATCH};
use axum::http::{HeaderMap, HeaderName, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::routing::{get, put};
use axum::{Json, Router};
use extboard_core::{Canvas, rev, validate};
use std::net::{Ipv4Addr, SocketAddr};
use std::sync::Arc;
use tokio::net::TcpListener;

pub type AppState = Arc<App>;

pub struct App {
    pub store: Store,
    pub events: Events,
}

pub async fn serve(port: u16) -> Result<(), Box<dyn std::error::Error>> {
    let state: AppState = Arc::new(App {
        store: Store::new()?,
        events: Events::new(),
    });
    let _watcher = watch(state.clone())?;

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
        .route("/api/spaces/{id}", put(put_space))
        .route("/api/events", get(events))
        .with_state(state)
}

/// GET endpoints
async fn health(State(app): State<AppState>) -> Result<&'static str, StatusCode> {
    match app.store.list() {
        Ok(_) => Ok("ok"),
        Err(_) => Err(StatusCode::SERVICE_UNAVAILABLE),
    }
}

async fn list_spaces(State(app): State<AppState>) -> Result<Json<Vec<String>>, StoreError> {
    Ok(Json(app.store.list()?))
}

async fn get_space(
    State(app): State<AppState>,
    Path(id): Path<String>,
    headers: HeaderMap,
) -> Result<Response, StoreError> {
    let canvas = app.store.space(&id)?.read().await.load()?;

    let etag = format!("\"{}\"", rev(&canvas));

    // The client already holds this version: 304, no body, nothing transferred.
    if etag_matches(&headers, IF_NONE_MATCH, &etag) {
        return Ok((StatusCode::NOT_MODIFIED, [(ETAG, etag)]).into_response());
    }

    Ok((
        [(ETAG, etag), (CONTENT_TYPE, "application/json".to_owned())],
        canvas.to_pretty_string(),
    )
        .into_response())
}

// write endpoints
async fn put_space(
    State(app): State<AppState>,
    Path(id): Path<String>,
    headers: HeaderMap,
    Json(incoming): Json<Canvas>,
) -> Result<Response, StoreError> {
    // Unconditional writes are how you lose someone else's edit.
    if headers.get(IF_MATCH).is_none() {
        return Ok(StatusCode::PRECONDITION_REQUIRED.into_response());
    }

    let space = app.store.space(&id)?;
    let mut guard = space.write().await;

    let current = guard.load()?;
    let etag = etag(&current);
    if !etag_matches(&headers, IF_MATCH, &etag) {
        // The body saves the client a GET: it reconciles from this response.
        return Ok((
            StatusCode::CONFLICT,
            [(ETAG, etag), (CONTENT_TYPE, "application/json".to_owned())],
            current.to_pretty_string(),
        )
            .into_response());
    }

    // validate incoming canvas
    if let Err(errors) = validate(&incoming) {
        let errors: Vec<String> = errors.iter().map(ToString::to_string).collect();
        return Ok((
            StatusCode::UNPROCESSABLE_ENTITY,
            Json(serde_json::json!({ "errors": errors })),
        )
            .into_response());
    }

    let saved = guard.save(&incoming)?;
    app.events.emit(&id, &saved);
    let saved = format!("\"{}\"", saved);
    Ok((StatusCode::NO_CONTENT, [(ETAG, saved)]).into_response())
}

fn etag(canvas: &Canvas) -> String {
    format!("\"{}\"", rev(canvas))
}

// Compare client's rev, as sent in `header`, against ours.
fn etag_matches(headers: &HeaderMap, header: HeaderName, etag: &str) -> bool {
    headers
        .get(header)
        .is_some_and(|sent| sent.as_bytes() == etag.as_bytes())
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

    fn state(dir: std::path::PathBuf) -> AppState {
        Arc::new(App {
            store: Store::at(dir),
            events: Events::new(),
        })
    }

    // `oneshot` feeds one request straight into the Router and returns the
    // response — no socket, no port, no runtime teardown. This is the way to
    // test axum handlers.
    #[tokio::test]
    async fn health_is_ok() {
        let dir = std::env::temp_dir().join("extboard-health-test");
        std::fs::create_dir_all(&dir).unwrap();
        let app = router(state(dir.clone()));

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
        let app = router(state(dir.clone()));

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

    // The ticket's done-when: 204 on the right rev, 409 + body on the stale
    // one, 422 for a dangling edge and the file untouched.
    #[tokio::test]
    async fn put_space_compare_and_swap() {
        let dir = std::env::temp_dir().join("extboard-put-space-test");
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("a.canvas"), r#"{"nodes":[],"edges":[]}"#).unwrap();
        let state: AppState = state(dir.clone());

        let put = |etag: Option<&str>, body: &'static str| {
            let mut req = Request::put("/api/spaces/a").header(CONTENT_TYPE, "application/json");
            if let Some(etag) = etag {
                req = req.header(IF_MATCH, etag);
            }
            router(state.clone()).oneshot(req.body(Body::from(body)).unwrap())
        };

        let got = router(state.clone())
            .oneshot(Request::get("/api/spaces/a").body(Body::empty()).unwrap())
            .await
            .unwrap();
        let first = got.headers()[ETAG].to_str().unwrap().to_owned();

        assert_eq!(
            put(None, r#"{"nodes":[],"edges":[]}"#)
                .await
                .unwrap()
                .status(),
            StatusCode::PRECONDITION_REQUIRED
        );

        let node = r#"{"nodes":[{"id":"n1","type":"text","x":0,"y":0,"width":10,"height":10,"text":"hi"}],"edges":[]}"#;
        let ok = put(Some(&first), node).await.unwrap();
        assert_eq!(ok.status(), StatusCode::NO_CONTENT);
        let second = ok.headers()[ETAG].to_str().unwrap().to_owned();
        assert_ne!(first, second);

        // Same request again: the rev it was based on is gone now.
        let stale = put(Some(&first), node).await.unwrap();
        assert_eq!(stale.status(), StatusCode::CONFLICT);
        assert_eq!(stale.headers()[ETAG].to_str().unwrap(), second);
        let body = axum::body::to_bytes(stale.into_body(), usize::MAX)
            .await
            .unwrap();
        assert!(!body.is_empty(), "409 must carry the current document");

        let dangling = r#"{"nodes":[],"edges":[{"id":"e1","fromNode":"nope","toNode":"gone"}]}"#;
        let rejected = put(Some(&second), dangling).await.unwrap();
        assert_eq!(rejected.status(), StatusCode::UNPROCESSABLE_ENTITY);
        let body = axum::body::to_bytes(rejected.into_body(), usize::MAX)
            .await
            .unwrap();
        assert!(
            String::from_utf8_lossy(&body).contains("does not exist"),
            "422 must name the fault: {}",
            String::from_utf8_lossy(&body)
        );
        assert_eq!(
            etag(&state.store.space("a").unwrap().read().await.load().unwrap()),
            second,
            "a rejected write touched the file"
        );

        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[tokio::test]
    async fn get_space_etag_304_and_404() {
        let dir = std::env::temp_dir().join("extboard-get-space-test");
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("a.canvas"), r#"{"nodes":[],"edges":[]}"#).unwrap();
        let state: AppState = state(dir.clone());

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
