//! `GET /api/events` — SSE, one event type: `changed`. Phase 2.
//!
//! Carries the new `rev` and an `id:`, never the document. Both fields are
//! free today and are what turns "add node deltas later" from a protocol
//! break into an additive change.
//!
//! Also the `notify` watcher on the spaces dir, so an Obsidian save or a
//! `git pull` reaches connected clients. Needs a `: ping` comment every
//! 15-30s or idle intermediaries reap the connection.

use crate::api::AppState;
use axum::response::sse::{Event, KeepAlive, Sse};
use extboard_core::rev;
use notify::{RecommendedWatcher, RecursiveMode, Watcher};
use std::collections::{HashMap, HashSet};
use std::convert::Infallible;
use std::path::Path;
use std::sync::Mutex;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;
use tokio::sync::{broadcast, mpsc};
use tokio::time::timeout;
use tokio_stream::wrappers::BroadcastStream;
use tokio_stream::wrappers::errors::BroadcastStreamRecvError;
use tokio_stream::{Stream, StreamExt};

/// Editors write a file two or three times per save.
const DEBOUNCE: Duration = Duration::from_millis(200);
const KEEPALIVE: Duration = Duration::from_secs(20);
const BACKLOG: usize = 64;

#[derive(Clone, Debug)]
pub struct Change {
    pub id: u64,
    pub space: String,
    pub rev: String,
}

pub struct Events {
    tx: broadcast::Sender<Change>,
    next_id: AtomicU64,
    last: Mutex<HashMap<String, String>>,
}

impl Events {
    pub fn new() -> Self {
        Self {
            tx: broadcast::channel(BACKLOG).0,
            next_id: AtomicU64::new(1),
            last: Mutex::new(HashMap::new()),
        }
    }

    /// Announce a space's new rev. A repeat of the rev we last announced is
    /// our own save echoing back through the watcher, and is dropped.
    pub fn emit(&self, space: &str, rev: &str) {
        let mut last = self.last.lock().unwrap_or_else(|e| e.into_inner());
        if last.get(space).is_some_and(|seen| seen == rev) {
            return;
        }
        last.insert(space.to_string(), rev.to_string());
        drop(last);

        let _ = self.tx.send(Change {
            id: self.next_id.fetch_add(1, Ordering::Relaxed),
            space: space.to_string(),
            rev: rev.to_string(),
        });
    }
}

pub async fn events(
    axum::extract::State(app): axum::extract::State<AppState>,
) -> Sse<impl Stream<Item = Result<Event, Infallible>>> {
    let stream = BroadcastStream::new(app.events.tx.subscribe()).map(|msg| {
        Ok(match msg {
            Ok(change) => Event::default()
                .event("changed")
                .id(change.id.to_string())
                .data(serde_json::json!({ "space": change.space, "rev": change.rev }).to_string()),
            // Dropped messages: no rev to offer, so the client refetches.
            Err(BroadcastStreamRecvError::Lagged(_)) => {
                Event::default().event("changed").data("{}")
            }
        })
    });

    Sse::new(stream).keep_alive(KeepAlive::new().interval(KEEPALIVE).text("ping"))
}

/// Watches the spaces dir. The returned watcher must be kept alive.
pub fn watch(app: AppState) -> notify::Result<RecommendedWatcher> {
    let (tx, mut rx) = mpsc::unbounded_channel();

    // notify's callback is sync, so it only hands paths to the async side.
    let mut watcher = notify::recommended_watcher(move |res: notify::Result<notify::Event>| {
        if let Ok(event) = res {
            for path in event.paths {
                let _ = tx.send(path);
            }
        }
    })?;
    watcher.watch(app.store.dir(), RecursiveMode::NonRecursive)?;

    tokio::spawn(async move {
        while let Some(path) = rx.recv().await {
            let mut paths = HashSet::from([path]);
            while let Ok(Some(next)) = timeout(DEBOUNCE, rx.recv()).await {
                paths.insert(next);
            }
            for id in paths.iter().filter_map(|path| space_id(path)) {
                reload(&app, &id).await;
            }
        }
    });

    Ok(watcher)
}

async fn reload(app: &AppState, id: &str) {
    let Ok(space) = app.store.space(id) else {
        return;
    };
    let guard = space.write().await;
    if let Ok(canvas) = guard.load() {
        app.events.emit(id, &rev(&canvas));
    }
}

fn space_id(path: &Path) -> Option<String> {
    (path.extension()? == "canvas")
        .then(|| path.file_stem()?.to_str().map(str::to_string))
        .flatten()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    #[test]
    fn only_canvas_files_are_spaces() {
        assert_eq!(
            space_id(&PathBuf::from("/s/a.canvas")).as_deref(),
            Some("a")
        );
        assert_eq!(space_id(&PathBuf::from("/s/a.canvas.tmp")), None);
        assert_eq!(space_id(&PathBuf::from("/s/notes.md")), None);
    }

    #[tokio::test]
    async fn repeated_rev_is_our_own_save_echoing() {
        let events = Events::new();
        let mut rx = events.tx.subscribe();

        events.emit("a", "r1");
        events.emit("a", "r1");
        events.emit("a", "r2");
        events.emit("b", "r1");

        let seen: Vec<_> = std::iter::from_fn(|| rx.try_recv().ok())
            .map(|c| (c.id, c.space, c.rev))
            .collect();
        assert_eq!(
            seen,
            [
                (1, "a".into(), "r1".into()),
                (2, "a".into(), "r2".into()),
                (3, "b".into(), "r1".into())
            ]
        );
    }
}
