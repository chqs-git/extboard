use crate::events::{Events, events, watch};
use crate::store::{Store, StoreError};
use axum::body::Bytes;
use axum::extract::{DefaultBodyLimit, Path, State};
use axum::http::header::{CONTENT_TYPE, ETAG, IF_MATCH, IF_NONE_MATCH};
use axum::http::{HeaderMap, HeaderName, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post, put};
use axum::{Json, Router};
use extboard_core::{Canvas, is_font, is_image, rev, validate};
use std::net::{Ipv4Addr, SocketAddr};
use std::path::PathBuf;
use std::sync::Arc;
use tokio::net::TcpListener;
use tower_http::compression::Compression;
use tower_http::services::{ServeDir, ServeFile};

pub type AppState = Arc<App>;

pub struct App {
    pub store: Store,
    pub events: Events,
}

pub async fn serve(port: u16, dist: PathBuf) -> Result<(), Box<dyn std::error::Error>> {
    let state: AppState = Arc::new(App {
        store: Store::new()?,
        events: Events::new(),
    });
    let _watcher = watch(state.clone())?;

    if !dist.join("index.html").is_file() {
        eprintln!(
            "no bundle at {}: the API works, the editor 404s. `trunk build --release` first.",
            dist.display()
        );
    }

    let addr = SocketAddr::from((Ipv4Addr::LOCALHOST, port));
    let listener = TcpListener::bind(addr).await?;
    println!("extd listening on http://{addr}");

    axum::serve(listener, app(state, dist))
        .with_graceful_shutdown(shutdown_signal())
        .await?;
    Ok(())
}

// The API plus the bundle. Split from `router` so the tests can drive the API
// without a `dist` directory to point at.
fn app(state: AppState, dist: PathBuf) -> Router {
    let dir = state.store.dir().to_path_buf();
    router(state, dir).fallback_service(bundle(dist))
}

fn bundle(dist: PathBuf) -> Compression<ServeDir<ServeFile>> {
    let index = ServeFile::new(dist.join("index.html"));
    Compression::new(ServeDir::new(dist).fallback(index)).br(true)
}

fn router(state: AppState, dir: PathBuf) -> Router {
    Router::new()
        .route("/health", get(health))
        .route("/api/spaces", get(list_spaces))
        .route("/api/spaces/{id}", get(get_space))
        .route("/api/spaces/{id}", put(put_space))
        .route("/api/spaces/{id}", post(create_space))
        .route("/api/events", get(events))
        .route(
            "/api/files/{name}",
            post(upload).layer(DefaultBodyLimit::max(UPLOAD_LIMIT)),
        )
        .nest_service("/f", ServeDir::new(dir))
        .with_state(state)
}

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

// A new space is an empty canvas. Creation is its own verb so a PUT can stay a
// compare-and-swap over a document that exists.
async fn create_space(
    State(app): State<AppState>,
    Path(id): Path<String>,
) -> Result<Response, StoreError> {
    let space = app.store.space(&id)?;
    let mut guard = space.write().await;
    if guard.exists() {
        return Ok(StatusCode::CONFLICT.into_response());
    }

    let saved = guard.save(&Canvas::default())?;
    app.events.emit(&id, &saved);
    Ok((StatusCode::CREATED, [(ETAG, format!("\"{saved}\""))]).into_response())
}

// A dropped photo off a phone, with room to spare.
const UPLOAD_LIMIT: usize = 32 * 1024 * 1024;

// The browser has no filesystem, so a file dropped on the board arrives here
// as bytes. Native drops take the same route: one directory, one writer.
async fn upload(
    State(app): State<AppState>,
    Path(name): Path<String>,
    body: Bytes,
) -> Result<Response, StoreError> {
    let Some(sub) = upload_dir(&name) else {
        return Ok((
            StatusCode::UNSUPPORTED_MEDIA_TYPE,
            Json(serde_json::json!({ "error": format!("{name} is neither an image nor a font") })),
        )
            .into_response());
    };
    let file = app.store.put_file(sub, &name, &body)?;
    Ok((StatusCode::CREATED, file).into_response())
}

// Which subdirectory a file belongs in, by what it is: a node's picture or the
// space's typeface. Anything else has nowhere to go.
fn upload_dir(name: &str) -> Option<&'static str> {
    match name {
        _ if is_image(name) => Some("images"),
        _ if is_font(name) => Some("fonts"),
        _ => None,
    }
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

    // The bundle answers every path the API does not claim, and claims none of
    // the API's own. Getting that wrong serves HTML to the client's fetches.
    #[tokio::test]
    async fn unmatched_paths_get_the_bundle_and_api_paths_do_not() {
        let dir = std::env::temp_dir().join("extboard-bundle-test");
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("index.html"), "<!doctype html>bundle").unwrap();
        let app = || app(state(dir.clone()), dir.clone());

        for path in ["/", "/s/kitchen-sink", "/s/anything"] {
            let got = app()
                .oneshot(Request::get(path).body(Body::empty()).unwrap())
                .await
                .unwrap();
            assert_eq!(got.status(), StatusCode::OK, "{path}");
        }

        let api = app()
            .oneshot(Request::get("/api/spaces").body(Body::empty()).unwrap())
            .await
            .unwrap();
        assert_eq!(api.headers()[CONTENT_TYPE], "application/json");
        std::fs::remove_dir_all(&dir).unwrap();
    }

    // The images an image node points at, and the trust boundary around them:
    // the dir is served whole, so nothing above it may be reachable.
    #[tokio::test]
    async fn the_spaces_dir_is_served_under_f_and_nothing_above_it_is() {
        let root = std::env::temp_dir().join("extboard-files-test");
        let dir = root.join("spaces");
        std::fs::create_dir_all(dir.join("images")).unwrap();
        std::fs::write(dir.join("images/a.png"), b"\x89PNG").unwrap();
        std::fs::write(root.join("secret"), b"not yours").unwrap();
        let app = || router(state(dir.clone()), dir.clone());

        let served = app()
            .oneshot(Request::get("/f/images/a.png").body(Body::empty()).unwrap())
            .await
            .unwrap();
        assert_eq!(served.status(), StatusCode::OK);
        assert_eq!(served.headers()[CONTENT_TYPE], "image/png");

        for path in ["/f/../secret", "/f/%2e%2e/secret", "/f/images/../../secret"] {
            let got = app()
                .oneshot(Request::get(path).body(Body::empty()).unwrap())
                .await
                .unwrap();
            assert_ne!(got.status(), StatusCode::OK, "{path} escaped the dir");
        }

        std::fs::remove_dir_all(&root).unwrap();
    }

    // An upload is a write into the dir `/f` serves, so the same boundary
    // holds: a name is one filename component and one of two kinds of file.
    #[tokio::test]
    async fn an_upload_lands_by_kind_never_overwrites_and_never_escapes() {
        let root = std::env::temp_dir().join("extboard-upload-test");
        let dir = root.join("spaces");
        std::fs::create_dir_all(&dir).unwrap();
        let app = || router(state(dir.clone()), dir.clone());
        let put = |path: &str| {
            app().oneshot(
                Request::post(path)
                    .body(Body::from(b"\x89PNG".to_vec()))
                    .unwrap(),
            )
        };
        let body = async |response: Response| {
            let bytes = axum::body::to_bytes(response.into_body(), 64)
                .await
                .unwrap();
            String::from_utf8(bytes.to_vec()).unwrap()
        };

        let first = put("/api/files/a%20b.png").await.unwrap();
        assert_eq!(first.status(), StatusCode::CREATED);
        assert_eq!(body(first).await, "images/a b.png");
        // Same name, different photo: both land.
        assert_eq!(
            body(put("/api/files/a%20b.png").await.unwrap()).await,
            "images/a b-1.png"
        );
        assert_eq!(
            body(put("/api/files/x.ttf").await.unwrap()).await,
            "fonts/x.ttf"
        );

        for name in ["x.exe", "x", ".ssh", "..%2fsecret.png"] {
            let got = put(&format!("/api/files/{name}")).await.unwrap();
            assert_ne!(got.status(), StatusCode::CREATED, "{name} was accepted");
        }
        assert!(!root.join("secret.png").exists());

        std::fs::remove_dir_all(&root).unwrap();
    }

    // `oneshot` feeds one request straight into the Router and returns the
    // response — no socket, no port, no runtime teardown. This is the way to
    // test axum handlers.
    #[tokio::test]
    async fn health_is_ok() {
        let dir = std::env::temp_dir().join("extboard-health-test");
        std::fs::create_dir_all(&dir).unwrap();
        let app = router(state(dir.clone()), dir.clone());

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
        let app = router(state(dir.clone()), dir.clone());

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
            router(state.clone(), dir.clone()).oneshot(req.body(Body::from(body)).unwrap())
        };

        let got = router(state.clone(), dir.clone())
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

    // The list screen's one write: an empty space appears, and a second POST
    // does not blank the one already there.
    #[tokio::test]
    async fn create_space_makes_an_empty_canvas_once() {
        let dir = std::env::temp_dir().join("extboard-create-space-test");
        std::fs::create_dir_all(&dir).unwrap();
        let state: AppState = state(dir.clone());
        let post = || {
            router(state.clone(), dir.clone()).oneshot(
                Request::post("/api/spaces/fresh")
                    .body(Body::empty())
                    .unwrap(),
            )
        };

        let made = post().await.unwrap();
        assert_eq!(made.status(), StatusCode::CREATED);
        let canvas = state.store.space("fresh").unwrap().read().await.load();
        assert_eq!(canvas.unwrap(), Canvas::default());

        let node = r#"{"nodes":[{"id":"n1","type":"text","x":0,"y":0,"width":10,"height":10,"text":"hi"}],"edges":[]}"#;
        std::fs::write(dir.join("fresh.canvas"), node).unwrap();
        assert_eq!(post().await.unwrap().status(), StatusCode::CONFLICT);
        assert_eq!(
            std::fs::read_to_string(dir.join("fresh.canvas")).unwrap(),
            node,
            "a refused create overwrote the space"
        );

        let bad = router(state, dir.clone())
            .oneshot(Request::post("/api/spaces/..").body(Body::empty()).unwrap())
            .await
            .unwrap();
        assert_eq!(bad.status(), StatusCode::BAD_REQUEST);

        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[tokio::test]
    async fn get_space_etag_304_and_404() {
        let dir = std::env::temp_dir().join("extboard-get-space-test");
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("a.canvas"), r#"{"nodes":[],"edges":[]}"#).unwrap();
        let state: AppState = state(dir.clone());

        let got = router(state.clone(), dir.clone())
            .oneshot(Request::get("/api/spaces/a").body(Body::empty()).unwrap())
            .await
            .unwrap();
        assert_eq!(got.status(), StatusCode::OK);
        assert_eq!(got.headers()[CONTENT_TYPE], "application/json");
        let etag = got.headers()[ETAG].to_str().unwrap().to_owned();
        assert!(etag.starts_with('"') && etag.ends_with('"'), "{etag}");

        // Same rev in hand: nothing transferred.
        let cached = router(state.clone(), dir.clone())
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
        let stale = router(state.clone(), dir.clone())
            .oneshot(
                Request::get("/api/spaces/a")
                    .header(IF_NONE_MATCH, "\"0000000000000000\"")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(stale.status(), StatusCode::OK);

        let missing = router(state.clone(), dir.clone())
            .oneshot(
                Request::get("/api/spaces/nope")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(missing.status(), StatusCode::NOT_FOUND);
        assert_eq!(missing.headers()[CONTENT_TYPE], "application/json");

        let bad = router(state, dir.clone())
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
