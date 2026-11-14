use bevy::prelude::*;
use extboard_core::Canvas;

use crate::backend::{Event, Io, Loaded};
use crate::lock::Locked;
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
    // The space and extd's reason: locked, or the password sent was wrong.
    Locked(String, String),
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
            .init_resource::<Io>()
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

fn connect(time: Res<Time>, io: Res<Io>, mut live: ResMut<Live>, rev: Res<Rev>, space: Res<Space>) {
    let Some(id) = space.id() else {
        return;
    };
    if live.subscribed || !live.retry.tick(time.delta()).is_finished() {
        return;
    }
    // Refetch as well as resubscribe: the file can have moved on while we were
    // not listening.
    fetch(&io, &live.inbox, &rev.0, id);
    subscribe(&io, &live.inbox);
    live.subscribed = true;
}

// Another space is another document: the rev it is compared and swapped against
// is gone, and an empty canvas is what clears the board while the new one is in
// the air. The stream stays open -- it carries every space, and `apply` is what
// picks ours out of it.
fn switch(
    mut commands: Commands,
    space: Res<Space>,
    mut rev: ResMut<Rev>,
    mut locked: ResMut<Locked>,
    io: Res<Io>,
    live: Res<Live>,
) {
    locked.0 = None;
    rev.0.clear();
    commands.insert_resource(Document(Canvas::default()));
    // Not on the first frame: `connect` has not subscribed yet, and its own
    // fetch is the one that lands.
    if let (Some(id), true) = (space.id(), live.subscribed) {
        fetch(&io, &live.inbox, "", id);
    }
}

fn apply(
    mut commands: Commands,
    io: Res<Io>,
    mut live: ResMut<Live>,
    mut rev: ResMut<Rev>,
    mut notice: ResMut<Notice>,
    mut locked: ResMut<Locked>,
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
                    fetch(&io, &live.inbox, &rev.0, id);
                }
            }
            Update::Changed(..) => {}
            Update::Locked(id, reason) => locked.0 = Some((id, reason)),
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
            right: Val::Px(8.0),
            ..default()
        },
    ));
}

fn fetch(io: &Io, inbox: &Inbox, rev: &str, space: &str) {
    let (inbox, id) = (inbox.clone(), space.to_owned());
    io.get(
        space,
        rev,
        Box::new(move |loaded| {
            let update = match loaded {
                Ok(Loaded::Canvas(canvas, rev)) => Update::Loaded(id, canvas, rev),
                Ok(Loaded::Unchanged) => return,
                Ok(Loaded::Locked(reason)) => Update::Locked(id, reason),
                Err(message) => Update::Failed(message),
            };
            lock(&inbox).push(update);
        }),
    );
}

impl Update {
    // The space it is about, where it is about one: a failure or a dropped
    // stream is the client's own news and belongs to whatever is open.
    fn space(&self) -> Option<&str> {
        match self {
            // A lagged stream names no space, so it is news about ours too.
            Update::Loaded(space, ..) | Update::Changed(space, _) | Update::Locked(space, _) => {
                (!space.is_empty()).then_some(space.as_str())
            }
            Update::Disconnected | Update::Failed(_) => None,
        }
    }
}

fn subscribe(io: &Io, inbox: &Inbox) {
    let inbox = inbox.clone();
    io.subscribe(Box::new(move |event| {
        lock(&inbox).push(match event {
            Event::Changed(space, rev) => Update::Changed(space, rev),
            Event::Disconnected => Update::Disconnected,
        });
    }));
}

fn lock(inbox: &Inbox) -> std::sync::MutexGuard<'_, Vec<Update>> {
    inbox.lock().unwrap_or_else(PoisonError::into_inner)
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

    // `apply` is what decides whether we care: only a space we have open is
    // `Update::space`'s.
    #[test]
    fn only_the_open_space_is_ours() {
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
