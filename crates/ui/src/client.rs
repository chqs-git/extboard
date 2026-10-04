use bevy::prelude::*;
use ehttp::streaming::Part;
use extboard_core::Canvas;
use std::ops::ControlFlow;
use std::sync::{Arc, Mutex, PoisonError};
use std::time::Duration;

// extd serves the bundle on wasm, so a relative path is same-origin and needs
// no CORS. Native has no origin to be relative to.
#[cfg(target_arch = "wasm32")]
pub fn base_url() -> &'static str {
    ""
}

#[cfg(not(target_arch = "wasm32"))]
fn remote() -> Option<&'static str> {
    static URL: std::sync::LazyLock<Option<String>> =
        std::sync::LazyLock::new(|| std::env::var("EXTBOARD_SERVER").ok());
    URL.as_deref().map(|url| url.trim_end_matches('/'))
}

#[cfg(not(target_arch = "wasm32"))]
pub fn base_url() -> &'static str {
    remote().unwrap_or("http://127.0.0.1:7777")
}

#[cfg(not(target_arch = "wasm32"))]
pub fn asset_path(file: &str) -> String {
    spaces_file(remote(), file)
}

// `url_path` is what the upload and the phone view already use: a space in
// `my holiday.png` is a URL ureq refuses, and a `#` is an asset label.
#[cfg(not(target_arch = "wasm32"))]
fn spaces_file(base: Option<&str>, file: &str) -> String {
    match base {
        Some(base) => format!("{base}/f/{}", extboard_core::url_path(file)),
        None => file.to_owned(),
    }
}

#[cfg(target_arch = "wasm32")]
pub fn asset_path(file: &str) -> String {
    file.to_owned()
}
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

// The space on screen. `None` is the spaces list: no space is open, so there
// is nothing to fetch and nothing to save.
#[derive(Resource, Default)]
pub struct Space(pub Option<String>);

impl Space {
    pub fn id(&self) -> Option<&str> {
        self.0.as_deref()
    }
}

#[cfg(target_arch = "wasm32")]
fn page_space_id() -> Option<String> {
    let path = web_sys::window()?.location().pathname().ok()?;
    Some(extboard_core::space_path(&path)?.to_owned())
}

#[cfg(not(target_arch = "wasm32"))]
fn page_space_id() -> Option<String> {
    None
}

#[cfg(target_arch = "wasm32")]
fn track_url(space: Res<Space>) {
    let Some(history) = web_sys::window().and_then(|window| window.history().ok()) else {
        return;
    };
    let path = match space.id() {
        Some(id) => format!("/s/{}", extboard_core::url_path(id)),
        None => "/".to_owned(),
    };
    let _ = history.replace_state_with_url(&wasm_bindgen::JsValue::NULL, "", Some(&path));
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

// Every update names the space it is about: a reply from the space just left
// must not land on the one just opened.
#[derive(Debug)]
enum Update {
    Loaded(String, Canvas, String),
    Changed(String, String),
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
        app.insert_resource(Space(page_space_id()))
            .init_resource::<Live>()
            .init_resource::<Rev>()
            .init_resource::<Notice>()
            .add_systems(
                Update,
                (
                    (switch.run_if(resource_changed::<Space>), apply, connect).chain(),
                    show_notice.run_if(resource_changed::<Notice>),
                ),
            );
        #[cfg(target_arch = "wasm32")]
        app.add_systems(Update, track_url.run_if(resource_changed::<Space>));
    }
}

fn connect(time: Res<Time>, mut live: ResMut<Live>, rev: Res<Rev>, space: Res<Space>) {
    let Some(id) = space.id() else {
        return;
    };
    if live.subscribed || !live.retry.tick(time.delta()).is_finished() {
        return;
    }
    // Refetch as well as resubscribe: the file can have moved on while we were
    // not listening.
    fetch(&live.inbox, &rev.0, id);
    subscribe(&live.inbox);
    live.subscribed = true;
}

// Another space is another document: the rev it is compared and swapped against
// is gone, and an empty canvas is what clears the board while the new one is in
// the air. The stream stays open -- it carries every space, and `apply` is what
// picks ours out of it.
fn switch(mut commands: Commands, space: Res<Space>, mut rev: ResMut<Rev>, live: Res<Live>) {
    rev.0.clear();
    commands.insert_resource(Document(Canvas::default()));
    // Not on the first frame: `connect` has not subscribed yet, and its own
    // fetch is the one that lands.
    if let (Some(id), true) = (space.id(), live.subscribed) {
        fetch(&live.inbox, "", id);
    }
}

fn apply(
    mut commands: Commands,
    mut live: ResMut<Live>,
    mut rev: ResMut<Rev>,
    mut notice: ResMut<Notice>,
    space: Res<Space>,
) {
    let batch = std::mem::take(&mut *lock(&live.inbox));
    for update in batch {
        // Anything about a space we no longer have open is stale by now.
        if let Some(about) = update.space()
            && space.id() != Some(about)
        {
            continue;
        }
        match update {
            Update::Loaded(_, canvas, loaded) => {
                info!("canvas {loaded}: {} nodes", canvas.nodes.len());
                rev.0 = loaded;
                commands.insert_resource(Document(canvas));
            }
            // A rev we already hold is our own save echoing back off the disk.
            Update::Changed(_, changed) if changed != rev.0 => {
                if let Some(id) = space.id() {
                    fetch(&live.inbox, &rev.0, id);
                }
            }
            Update::Changed(..) => {}
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

pub fn space_url(space: &str) -> String {
    format!("{}/api/spaces/{space}", base_url())
}

fn fetch(inbox: &Inbox, rev: &str, space: &str) {
    let mut request = ehttp::Request::get(space_url(space));
    if !rev.is_empty() {
        request
            .headers
            .insert("If-None-Match", format!("\"{rev}\""));
    }

    let inbox = inbox.clone();
    let space = space.to_owned();
    ehttp::fetch(request, move |result| {
        if let Some(update) = loaded(&space, result) {
            lock(&inbox).push(update);
        }
    });
}

fn loaded(space: &str, result: ehttp::Result<ehttp::Response>) -> Option<Update> {
    // A dead server is a transport error; a live one can still answer 404/500,
    // which ehttp reports as Ok. Both have to read as a failure.
    let response = match result {
        Ok(response) => response,
        Err(e) => {
            return Some(Update::Failed(format!(
                "extd unreachable at {}: {e}",
                base_url()
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
        Ok(canvas) => Update::Loaded(space.to_owned(), canvas, etag_rev(&response)),
        Err(e) => Update::Failed(format!("bad canvas from extd: {e}")),
    })
}

impl Update {
    // The space it is about, where it is about one: a failure or a dropped
    // stream is the client's own news and belongs to whatever is open.
    fn space(&self) -> Option<&str> {
        match self {
            // A lagged stream names no space, so it is news about ours too.
            Update::Loaded(space, ..) | Update::Changed(space, _) => {
                (!space.is_empty()).then_some(space.as_str())
            }
            Update::Disconnected | Update::Failed(_) => None,
        }
    }
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
    let mut request = ehttp::Request::get(format!("{}/api/events", base_url()));
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
            if let Some((space, rev)) = changed_rev(&frame) {
                lock(&inbox).push(Update::Changed(space, rev));
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

// `(space, rev)`. A lagged stream names neither, and an empty rev is one no
// client can be holding, so it reads as a change to whatever is open.
fn changed_rev(frame: &str) -> Option<(String, String)> {
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
        (Some(space), Some(rev)) => Some((space.as_str()?.to_owned(), rev.as_str()?.to_owned())),
        (None, None) => Some((String::new(), String::new())),
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

    #[cfg(not(target_arch = "wasm32"))]
    #[test]
    fn a_remote_spaces_file_is_a_url_and_a_local_one_is_a_bare_path() {
        assert_eq!(
            spaces_file(Some("https://box.ts.net"), "images/my holiday.png"),
            "https://box.ts.net/f/images/my%20holiday.png"
        );
        assert_eq!(
            spaces_file(None, "images/my holiday.png"),
            "images/my holiday.png"
        );
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
        let failed = |result| matches!(loaded("a", result), Some(Update::Failed(_)));
        // Server down: this is the case that used to panic off-thread.
        assert!(failed(Err("connection refused".to_owned())));
        // Server up, space missing: ehttp calls this Ok.
        assert!(failed(Ok(response(404, "not found"))));
        assert!(failed(Ok(response(200, "<html>"))));
    }

    #[test]
    fn a_load_carries_the_etags_rev_and_304_carries_nothing() {
        let got = loaded("a", Ok(response(200, r#"{"nodes":[],"edges":[]}"#)));
        assert!(
            matches!(&got, Some(Update::Loaded(space, _, rev)) if space == "a" && rev == "r1"),
            "{got:?}"
        );
        assert!(loaded("a", Ok(response(304, ""))).is_none());
    }

    #[test]
    fn frames_survive_being_split_across_chunks() {
        let mut buffer = String::new();
        buffer.push_str(":ping\n\nevent: changed\ndata: {\"space\":\"a\",\"re");
        assert_eq!(drain_frames(&mut buffer), [":ping"]);

        buffer.push_str("v\":\"r2\"}\n\n");
        let frames = drain_frames(&mut buffer);
        assert_eq!(frames.len(), 1);
        assert_eq!(
            changed_rev(&frames[0]),
            Some(("a".to_owned(), "r2".to_owned()))
        );
        assert!(buffer.is_empty());
    }

    // Which space a change is about is the frame's to say; `apply` is what
    // decides whether we care. Only a space we have open is `Update::space`'s.
    #[test]
    fn a_change_carries_its_space_and_only_the_open_one_is_ours() {
        let frame = |data: &str| format!("event: changed\ndata: {data}");
        assert_eq!(
            changed_rev(&frame(r#"{"space":"b","rev":"r2"}"#)),
            Some(("b".to_owned(), "r2".to_owned()))
        );
        assert_eq!(changed_rev(":ping"), None);
        assert_eq!(changed_rev("event: hello\ndata: {}"), None);
        // Lagged: no space and no rev to compare, so it is a change to ours.
        assert_eq!(
            changed_rev(&frame("{}")),
            Some((String::new(), String::new()))
        );

        let about = |space: &str| {
            Update::Changed(space.to_owned(), "r2".to_owned())
                .space()
                .map(ToOwned::to_owned)
        };
        assert_eq!(about("b").as_deref(), Some("b"));
        assert_eq!(about(""), None);
        assert_eq!(Update::Disconnected.space(), None);
    }
}
