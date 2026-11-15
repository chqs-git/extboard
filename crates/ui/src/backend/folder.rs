use bevy::prelude::*;
use extboard_core::{
    Canvas, child_of, inner_id, is_font, is_image, is_space_id, nth_name, reparented, untitled,
    validate,
};
use js_sys::{Function, Promise, Reflect, Uint8Array};
use std::cell::RefCell;
use std::future::Future;
use wasm_bindgen::JsCast;
use wasm_bindgen::prelude::*;
use wasm_bindgen_futures::{JsFuture, spawn_local};
use web_sys::{
    File, FileSystemDirectoryHandle, FileSystemFileHandle, FileSystemGetDirectoryOptions,
    FileSystemGetFileOptions, FileSystemWritableFileStream,
};

use super::{Backend, Event, Loaded, Reply, Saved};
use crate::client::Notice;

#[wasm_bindgen(inline_js = r#"
function db() {
  return new Promise((ok, err) => {
    const r = indexedDB.open('extboard', 1);
    r.onupgradeneeded = () => r.result.createObjectStore('handles');
    r.onsuccess = () => ok(r.result);
    r.onerror = () => err(r.error);
  });
}
async function idb(mode, f) {
  const req = f((await db()).transaction('handles', mode).objectStore('handles'));
  return new Promise((ok, err) => { req.onsuccess = () => ok(req.result); req.onerror = () => err(req.error); });
}
export async function pick() {
  const h = await showDirectoryPicker({ id: 'extboard', mode: 'readwrite' });
  await idb('readwrite', s => s.put(h, 'root'));
  return h;
}
export async function stored() { return (await idb('readonly', s => s.get('root'))) ?? null; }
export function query(h) { return h.queryPermission({ mode: 'readwrite' }); }
export function request(h) { return h.requestPermission({ mode: 'readwrite' }); }
"#)]
extern "C" {
    fn pick() -> Promise;
    fn stored() -> Promise;
    fn query(handle: &FileSystemDirectoryHandle) -> Promise;
    fn request(handle: &FileSystemDirectoryHandle) -> Promise;
}

pub fn chosen() -> bool {
    web_sys::window()
        .and_then(|window| window.location().search().ok())
        .is_some_and(|search| search.contains("folder"))
}

enum Waiting {
    Pick,
    Reconnect(FileSystemDirectoryHandle),
}

thread_local! {
    static OPENED: (Promise, Function) = {
        let mut resolve = None;
        let opened = Promise::new(&mut |ok, _| resolve = Some(ok));
        (opened, resolve.expect("the executor runs synchronously"))
    };
    static WAITING: RefCell<Option<Waiting>> = const { RefCell::new(None) };
}

fn open(dir: FileSystemDirectoryHandle) {
    OPENED.with(|(_, resolve)| {
        let _ = resolve.call1(&JsValue::NULL, &dir);
    });
}

async fn root() -> FileSystemDirectoryHandle {
    let opened = OPENED.with(|(opened, _)| opened.clone());
    JsFuture::from(opened)
        .await
        .map(JsCast::unchecked_into)
        .expect("never rejects")
}

pub struct FolderPlugin;

impl Plugin for FolderPlugin {
    fn build(&self, app: &mut App) {
        if chosen() {
            app.add_systems(Startup, restore).add_systems(Update, ask);
        }
    }
}

fn restore() {
    spawn_local(async {
        let waiting = match JsFuture::from(stored()).await {
            Ok(dir) if !dir.is_null() => {
                let dir: FileSystemDirectoryHandle = dir.unchecked_into();
                let state = JsFuture::from(query(&dir)).await.ok();
                if state.and_then(|state| state.as_string()).as_deref() == Some("granted") {
                    return open(dir);
                }
                Waiting::Reconnect(dir)
            }
            _ => Waiting::Pick,
        };
        WAITING.set(Some(waiting));
    });
}

// The call has to happen in the system that sees the click, or the activation is gone.
fn ask(
    buttons: Res<ButtonInput<MouseButton>>,
    mut notice: ResMut<Notice>,
    mut shown: Local<Option<&'static str>>,
) {
    let Some(waiting) = WAITING.take() else {
        if let Some(text) = shown.take()
            && notice.0.as_deref() == Some(text)
        {
            notice.0 = None;
        }
        return;
    };
    if !buttons.just_pressed(MouseButton::Left) {
        let text = match &waiting {
            Waiting::Pick => "Click to open your extboard folder",
            Waiting::Reconnect(_) => "Click to reconnect ~/extboard",
        };
        if *shown != Some(text) {
            notice.0 = Some(text.to_owned());
            *shown = Some(text);
        }
        WAITING.set(Some(waiting));
        return;
    }
    let asked = match &waiting {
        Waiting::Pick => pick(),
        Waiting::Reconnect(dir) => request(dir),
    };
    spawn_local(async move {
        match (JsFuture::from(asked).await, waiting) {
            (Ok(dir), Waiting::Pick) => open(dir.unchecked_into()),
            (Ok(state), Waiting::Reconnect(dir))
                if state.as_string().as_deref() == Some("granted") =>
            {
                open(dir);
            }
            (_, waiting) => WAITING.set(Some(waiting)),
        }
    });
}

pub struct Folder;

fn run<T: 'static, F: Future<Output = T>>(
    reply: Reply<T>,
    op: impl FnOnce(FileSystemDirectoryHandle) -> F + 'static,
) {
    spawn_local(async move { reply(op(root().await).await) });
}

impl Backend for Folder {
    fn get(&self, space: &str, rev: &str, reply: Reply<Result<Loaded, String>>) {
        let (space, asked) = (space.to_owned(), rev.to_owned());
        run(reply, move |dir| async move {
            let canvas = load(&dir, &space).await?;
            let rev = extboard_core::rev(&canvas);
            Ok(if rev == asked {
                Loaded::Unchanged
            } else {
                Loaded::Canvas(canvas, rev)
            })
        });
    }

    fn subscribe(&self, _: Box<dyn Fn(Event) + Send>) {}

    // The rev check and the write have no lock between them: a writer in that gap loses (E10-T9).
    fn put(&self, space: &str, canvas: &Canvas, base: &str, reply: Reply<Result<Saved, String>>) {
        let (space, canvas, base) = (space.to_owned(), canvas.clone(), base.to_owned());
        run(reply, move |dir| async move {
            let current = load(&dir, &space).await?;
            let rev = extboard_core::rev(&current);
            if rev != base {
                return Ok(Saved::Conflict(current, rev));
            }
            if let Err(errors) = validate(&canvas) {
                let errors: Vec<String> = errors.iter().map(ToString::to_string).collect();
                return Err(format!("refused the save: {}", errors.join("; ")));
            }
            save(&dir, &space, &canvas).await.map(Saved::Saved)
        });
    }

    fn list(&self, reply: Reply<Result<Vec<String>, String>>) {
        run(reply, |dir| async move {
            let mut top = Vec::new();
            for id in ids(&dir).await? {
                if load(&dir, &id)
                    .await
                    .is_ok_and(|canvas| canvas.parent().is_none())
                {
                    top.push(id);
                }
            }
            Ok(top)
        });
    }

    fn path(&self, space: &str, reply: Reply<Option<Vec<String>>>) {
        let space = space.to_owned();
        run(reply, move |dir| async move {
            let mut path = vec![space.clone()];
            let mut parent = load(&dir, &space).await.ok()?.parent().map(str::to_owned);
            while let Some(next) = parent {
                if path.contains(&next) {
                    return Some(vec![space]);
                }
                let Ok(canvas) = load(&dir, &next).await else {
                    break;
                };
                parent = canvas.parent().map(str::to_owned);
                path.push(next);
            }
            path.reverse();
            Some(path)
        });
    }

    fn create(&self, reply: Reply<Result<String, String>>) {
        run(reply, |dir| async move {
            for id in (1..).map(untitled) {
                if !exists(&dir, &file_of(&id)).await? {
                    save(&dir, &id, &Canvas::default()).await?;
                    return Ok(id);
                }
            }
            unreachable!("1.. never runs out")
        });
    }

    fn create_inner(&self, parent: &str, name: &str, reply: Reply<Result<String, String>>) {
        let (parent, name) = (parent.to_owned(), name.to_owned());
        run(reply, move |dir| async move {
            let id = inner_id(&parent, &name);
            if !is_space_id(&name) || !is_space_id(&id) {
                return Err(format!("{name} is not a usable name"));
            }
            let canvas = match parent.as_str() {
                "" => Canvas::default(),
                parent => child_of(parent, &load(&dir, parent).await?),
            };
            if exists(&dir, &file_of(&id)).await? {
                return Err(format!("{name} already exists here"));
            }
            save(&dir, &id, &canvas).await?;
            Ok(id)
        });
    }

    fn rename(&self, from: &str, to: &str, reply: Reply<Result<(), String>>) {
        let (from, to) = (from.to_owned(), to.to_owned());
        run(reply, move |dir| async move {
            if !is_space_id(&to) {
                return Err(format!("{to} is not a usable name"));
            }
            if from == to {
                return Ok(());
            }
            if exists(&dir, &file_of(&to)).await? {
                return Err(format!("{to} already exists"));
            }
            let bytes = read(&dir, &file_of(&from))
                .await?
                .ok_or_else(|| format!("no space {from}"))?;
            write(&dir, &file_of(&to), &bytes).await?;
            wait::<JsValue>(dir.remove_entry(&file_of(&from))).await?;
            for child in ids(&dir).await? {
                let moved = load(&dir, &child)
                    .await
                    .ok()
                    .and_then(|canvas| reparented(canvas, &from, &to));
                if let Some(canvas) = moved {
                    save(&dir, &child, &canvas).await?;
                }
            }
            Ok(())
        });
    }

    fn read(&self, file: &str, reply: Reply<Result<Vec<u8>, String>>) {
        let file = file.to_owned();
        run(reply, move |dir| async move {
            read(&dir, &file)
                .await?
                .ok_or_else(|| format!("no file {file}"))
        });
    }

    fn upload(&self, name: &str, bytes: Vec<u8>, reply: Reply<Result<String, String>>) {
        let name = name.to_owned();
        run(reply, move |dir| async move {
            let sub = match name.as_str() {
                _ if !is_space_id(&name) => return Err(format!("{name} is not a usable name")),
                _ if is_image(&name) => "images",
                _ if is_font(&name) => "fonts",
                _ => return Err(format!("{name} is neither an image nor a font")),
            };
            let options = FileSystemGetDirectoryOptions::new();
            options.set_create(true);
            let into: FileSystemDirectoryHandle =
                wait(dir.get_directory_handle_with_options(sub, &options)).await?;
            for n in 0.. {
                let candidate = nth_name(&name, n);
                if !exists(&into, &candidate).await? {
                    write(&into, &candidate, &bytes).await?;
                    return Ok(format!("{sub}/{candidate}"));
                }
            }
            unreachable!("0.. never runs out")
        });
    }
}

fn file_of(id: &str) -> String {
    format!("{id}.canvas")
}

fn message(e: JsValue) -> String {
    Reflect::get(&e, &"message".into())
        .ok()
        .and_then(|message| message.as_string())
        .or_else(|| e.as_string())
        .unwrap_or_else(|| format!("{e:?}"))
}

fn not_found(e: &JsValue) -> bool {
    Reflect::get(e, &"name".into())
        .ok()
        .and_then(|name| name.as_string())
        .is_some_and(|name| name == "NotFoundError")
}

async fn wait<T: JsCast>(promise: Promise) -> Result<T, String> {
    JsFuture::from(promise)
        .await
        .map(JsCast::unchecked_into)
        .map_err(message)
}

async fn handle(
    dir: &FileSystemDirectoryHandle,
    name: &str,
) -> Result<Option<FileSystemFileHandle>, String> {
    match JsFuture::from(dir.get_file_handle(name)).await {
        Ok(file) => Ok(Some(file.unchecked_into())),
        Err(e) if not_found(&e) => Ok(None),
        Err(e) => Err(message(e)),
    }
}

async fn exists(dir: &FileSystemDirectoryHandle, name: &str) -> Result<bool, String> {
    Ok(handle(dir, name).await?.is_some())
}

async fn read(dir: &FileSystemDirectoryHandle, name: &str) -> Result<Option<Vec<u8>>, String> {
    let Some(file) = handle(dir, name).await? else {
        return Ok(None);
    };
    let file: File = wait(file.get_file()).await?;
    let buffer: JsValue = wait(file.array_buffer()).await?;
    Ok(Some(Uint8Array::new(&buffer).to_vec()))
}

// The bytes are copied out first: the write outlives the call.
async fn write(dir: &FileSystemDirectoryHandle, name: &str, bytes: &[u8]) -> Result<(), String> {
    let options = FileSystemGetFileOptions::new();
    options.set_create(true);
    let file: FileSystemFileHandle = wait(dir.get_file_handle_with_options(name, &options)).await?;
    let stream: FileSystemWritableFileStream = wait(file.create_writable()).await?;
    let written = stream
        .write_with_js_u8_array(&Uint8Array::from(bytes))
        .map_err(message)?;
    wait::<JsValue>(written).await?;
    wait::<JsValue>(stream.close()).await.map(drop)
}

async fn load(dir: &FileSystemDirectoryHandle, id: &str) -> Result<Canvas, String> {
    let bytes = read(dir, &file_of(id))
        .await?
        .ok_or_else(|| format!("no space {id}"))?;
    serde_json::from_slice(&bytes).map_err(|e| format!("{id} is not a canvas: {e}"))
}

async fn save(
    dir: &FileSystemDirectoryHandle,
    id: &str,
    canvas: &Canvas,
) -> Result<String, String> {
    write(dir, &file_of(id), canvas.to_pretty_string().as_bytes()).await?;
    Ok(extboard_core::rev(canvas))
}

async fn ids(dir: &FileSystemDirectoryHandle) -> Result<Vec<String>, String> {
    let names = dir.keys();
    let mut ids = Vec::new();
    loop {
        let next: JsValue = wait(names.next().map_err(message)?).await?;
        if Reflect::get(&next, &"done".into())
            .map_err(message)?
            .is_truthy()
        {
            break;
        }
        let name = Reflect::get(&next, &"value".into()).map_err(message)?;
        if let Some(id) = name
            .as_string()
            .and_then(|name| name.strip_suffix(".canvas").map(str::to_owned))
        {
            ids.push(id);
        }
    }
    ids.sort();
    Ok(ids)
}
