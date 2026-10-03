use bevy::prelude::*;
use ehttp::streaming::Part;
use extboard_core::Canvas;
use std::ops::ControlFlow;
use std::sync::{Arc, Mutex, OnceLock, PoisonError};
use std::time::Duration;

// extd serves the bundle on wasm, so a relative path is same-origin and needs
// no CORS. Native has no origin to be relative to.
#[cfg(target_arch = "wasm32")]
pub const BASE_URL: &str = "";
// extd's default port
#[cfg(not(target_arch = "wasm32"))]
pub const BASE_URL: &str = "http://127.0.0.1:7777";
pub const TESTING_SPACE_ID: &str = "kitchen-sink";
const RETRY_SECS: f32 = 2.0;

// Where a `file` node's path resolves from: extd's spaces dir. In the browser
// that is the path extd serves it under, absolute so it does not resolve
// against `/s/<space>`; natively it is the directory itself, resolved the way
// extd's store.rs resolves it.
#[cfg(target_arch = "wasm32")]
pub fn files_root() -> String {
    "/f".to_owned()
}

#[cfg(not(target_arch = "wasm32"))]
pub fn files_root() -> String {
    std::env::var_os("EXTBOARD_DIR")
        .map(std::path::PathBuf::from)
        .or_else(|| std::env::home_dir().map(|home| home.join("extboard")))
        .unwrap_or_default()
        .to_string_lossy()
        .into_owned()
}

pub fn space_id() -> &'static str {
    static ID: OnceLock<String> = OnceLock::new();
    ID.get_or_init(|| page_space_id().unwrap_or_else(|| TESTING_SPACE_ID.to_owned()))
}

#[cfg(target_arch = "wasm32")]
fn page_space_id() -> Option<String> {
    let path = web_sys::window()?.location().pathname().ok()?;
    Some(space_from_path(&path)?.to_owned())
}

#[cfg(not(target_arch = "wasm32"))]
fn page_space_id() -> Option<String> {
    None
}

// Only the first segment: `/s/a/b` is not an id, and the store would refuse it
// with a 400 that reads like a server bug.
#[cfg_attr(
    not(target_arch = "wasm32"),
    allow(dead_code, reason = "no URL natively")
)]
fn space_from_path(path: &str) -> Option<&str> {
    let id = path.strip_prefix("/s/")?.split('/').next()?;
    (!id.is_empty()).then_some(id)
}

pub struct ClientPlugin;

#[derive(Resource)]
pub struct Document(pub Canvas);

#[derive(Resource, Default)]
pub struct Rev(pub String);

#[derive(Resource, Default)]
pub struct Notice(pub Option<String>);

#[derive(Component)]
struct ErrorText;

#[derive(Debug)]
enum Update {
    Loaded(Canvas, String),
    Changed(String),
    Disconnected,
    Failed(String),
}

type Inbox = Arc<Mutex<Vec<Update>>>;

#[derive(Resource)]
struct Live {
    inbox: Inbox,
    subscribed: bool,
    retry: Timer,
}

impl Default for Live {
    fn default() -> Self {
        let mut retry = Timer::from_seconds(RETRY_SECS, TimerMode::Once);
        retry.tick(Duration::from_secs_f32(RETRY_SECS)); // connect on frame one
        Self {
            inbox: Inbox::default(),
            subscribed: false,
            retry,
        }
    }
}

impl Plugin for ClientPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<Live>()
            .init_resource::<Rev>()
            .init_resource::<Notice>()
            .add_systems(
                Update,
                (
                    (apply, connect).chain(),
                    show_notice.run_if(resource_changed::<Notice>),
                ),
            );
    }
}

fn connect(time: Res<Time>, mut live: ResMut<Live>, rev: Res<Rev>) {
    if live.subscribed || !live.retry.tick(time.delta()).is_finished() {
        return;
    }
    // Refetch as well as resubscribe: the file can have moved on while we were
    // not listening.
    fetch(&live.inbox, &rev.0);
    subscribe(&live.inbox);
    live.subscribed = true;
}

fn apply(
    mut commands: Commands,
    mut live: ResMut<Live>,
    mut rev: ResMut<Rev>,
    mut notice: ResMut<Notice>,
) {
    let batch = std::mem::take(&mut *lock(&live.inbox));
    for update in batch {
        match update {
            Update::Loaded(canvas, loaded) => {
                info!("canvas {loaded}: {} nodes", canvas.nodes.len());
                rev.0 = loaded;
                commands.insert_resource(Document(canvas));
            }
            // A rev we already hold is our own save echoing back off the disk.
            Update::Changed(changed) if changed != rev.0 => fetch(&live.inbox, &rev.0),
            Update::Changed(_) => {}
            Update::Disconnected => {
                live.subscribed = false;
                live.retry.reset();
            }
            Update::Failed(message) => {
                error!("{message}");
                notice.0 = Some(message);
            }
        }
    }
}

fn show_notice(
    mut commands: Commands,
    notice: Res<Notice>,
    existing: Query<Entity, With<ErrorText>>,
) {
    for entity in &existing {
        commands.entity(entity).despawn();
    }
    let Some(message) = &notice.0 else {
        return;
    };
    commands.spawn((
        ErrorText,
        // Node panels are UI too and respawn on every change.
        GlobalZIndex(1),
        Text::new(message.clone()),
        TextFont::from_font_size(14.0),
        TextColor(Color::srgb(0.9, 0.35, 0.35)),
        Node {
            position_type: PositionType::Absolute,
            bottom: Val::Px(8.0),
            left: Val::Px(8.0),
            ..default()
        },
    ));
}

pub fn space_url() -> String {
    format!("{BASE_URL}/api/spaces/{}", space_id())
}

fn fetch(inbox: &Inbox, rev: &str) {
    let mut request = ehttp::Request::get(space_url());
    if !rev.is_empty() {
        request
            .headers
            .insert("If-None-Match", format!("\"{rev}\""));
    }

    let inbox = inbox.clone();
    ehttp::fetch(request, move |result| {
        if let Some(update) = loaded(result) {
            lock(&inbox).push(update);
        }
    });
}

fn loaded(result: ehttp::Result<ehttp::Response>) -> Option<Update> {
    // A dead server is a transport error; a live one can still answer 404/500,
    // which ehttp reports as Ok. Both have to read as a failure.
    let response = match result {
        Ok(response) => response,
        Err(e) => {
            return Some(Update::Failed(format!(
                "extd unreachable at {BASE_URL}: {e}"
            )));
        }
    };
    if response.status == 304 {
        return None;
    }
    if !response.ok {
        return Some(Update::Failed(format!("extd returned {}", response.status)));
    }
    Some(match serde_json::from_slice(&response.bytes) {
        Ok(canvas) => Update::Loaded(canvas, etag_rev(&response)),
        Err(e) => Update::Failed(format!("bad canvas from extd: {e}")),
    })
}

pub fn etag_rev(response: &ehttp::Response) -> String {
    response
        .headers
        .get("etag")
        .unwrap_or_default()
        .trim_matches('"')
        .to_owned()
}

fn subscribe(inbox: &Inbox) {
    let mut request = ehttp::Request::get(format!("{BASE_URL}/api/events"));
    request.timeout = None; // the stream is meant to stay open

    let inbox = inbox.clone();
    let buffer = Mutex::new(String::new());
    ehttp::streaming::fetch(request, move |part| {
        let chunk = match part {
            Ok(Part::Response(response)) if response.ok => return ControlFlow::Continue(()),
            Ok(Part::Chunk(chunk)) if !chunk.is_empty() => chunk,
            // An empty chunk is the end of the stream; the rest is extd gone.
            Ok(_) | Err(_) => {
                lock(&inbox).push(Update::Disconnected);
                return ControlFlow::Break(());
            }
        };

        let mut buffer = lock_string(&buffer);
        buffer.push_str(&String::from_utf8_lossy(&chunk));
        for frame in drain_frames(&mut buffer) {
            if let Some(rev) = changed_rev(&frame, space_id()) {
                lock(&inbox).push(Update::Changed(rev));
            }
        }
        ControlFlow::Continue(())
    });
}

fn drain_frames(buffer: &mut String) -> Vec<String> {
    let mut frames = Vec::new();
    while let Some(end) = buffer.find("\n\n") {
        frames.push(buffer[..end].to_owned());
        buffer.replace_range(..end + 2, "");
    }
    frames
}

fn changed_rev(frame: &str, space: &str) -> Option<String> {
    let mut changed = false;
    let mut data = None;
    for line in frame.lines() {
        if let Some(name) = line.strip_prefix("event:") {
            changed = name.trim() == "changed";
        }
        if let Some(payload) = line.strip_prefix("data:") {
            data = Some(payload.trim().to_owned());
        }
    }
    if !changed {
        return None;
    }

    let data: serde_json::Value = serde_json::from_str(&data?).ok()?;
    match (data.get("space"), data.get("rev")) {
        (Some(s), Some(rev)) if s.as_str() == Some(space) => Some(rev.as_str()?.to_owned()),
        (None, None) => Some(String::new()),
        _ => None,
    }
}

fn lock(inbox: &Inbox) -> std::sync::MutexGuard<'_, Vec<Update>> {
    inbox.lock().unwrap_or_else(PoisonError::into_inner)
}

fn lock_string(buffer: &Mutex<String>) -> std::sync::MutexGuard<'_, String> {
    buffer.lock().unwrap_or_else(PoisonError::into_inner)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_space_is_the_first_segment_after_s() {
        assert_eq!(space_from_path("/s/kitchen-sink"), Some("kitchen-sink"));
        assert_eq!(space_from_path("/s/lisbon-trip/"), Some("lisbon-trip"));
        // No id in the path: `space_id` falls back to the testing space.
        assert_eq!(space_from_path("/"), None);
        assert_eq!(space_from_path("/s/"), None);
        assert_eq!(space_from_path("/v/kitchen-sink"), None);
    }

    fn response(status: u16, body: &str) -> ehttp::Response {
        ehttp::Response {
            url: String::new(),
            ok: (200..300).contains(&status),
            status,
            status_text: String::new(),
            headers: ehttp::Headers::new(&[("etag", "\"r1\"")]),
            bytes: body.as_bytes().to_vec(),
        }
    }

    #[test]
    fn every_failure_carries_a_message() {
        let failed = |result| matches!(loaded(result), Some(Update::Failed(_)));
        // Server down: this is the case that used to panic off-thread.
        assert!(failed(Err("connection refused".to_owned())));
        // Server up, space missing: ehttp calls this Ok.
        assert!(failed(Ok(response(404, "not found"))));
        assert!(failed(Ok(response(200, "<html>"))));
    }

    #[test]
    fn a_load_carries_the_etags_rev_and_304_carries_nothing() {
        let got = loaded(Ok(response(200, r#"{"nodes":[],"edges":[]}"#)));
        assert!(
            matches!(&got, Some(Update::Loaded(_, rev)) if rev == "r1"),
            "{got:?}"
        );
        assert!(loaded(Ok(response(304, ""))).is_none());
    }

    #[test]
    fn frames_survive_being_split_across_chunks() {
        let mut buffer = String::new();
        buffer.push_str(":ping\n\nevent: changed\ndata: {\"space\":\"a\",\"re");
        assert_eq!(drain_frames(&mut buffer), [":ping"]);

        buffer.push_str("v\":\"r2\"}\n\n");
        let frames = drain_frames(&mut buffer);
        assert_eq!(frames.len(), 1);
        assert_eq!(changed_rev(&frames[0], "a").as_deref(), Some("r2"));
        assert!(buffer.is_empty());
    }

    #[test]
    fn only_our_spaces_changes_count() {
        let frame = |data: &str| format!("event: changed\ndata: {data}");
        assert_eq!(
            changed_rev(&frame(r#"{"space":"b","rev":"r2"}"#), "a"),
            None
        );
        assert_eq!(changed_rev(":ping", "a"), None);
        assert_eq!(changed_rev("event: hello\ndata: {}", "a"), None);
        // Lagged: no rev to compare, so it has to look like a change.
        assert_eq!(changed_rev(&frame("{}"), "a"), Some(String::new()));
    }
}
