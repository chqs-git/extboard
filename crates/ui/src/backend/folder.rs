use bevy::asset::io::{AssetReader, AssetReaderError, PathStream, Reader, VecReader};
use bevy::prelude::*;
use extboard_core::{
    Canvas, child_of, inner_id, is_font, is_image, is_space_id, nth_name, reparented, untitled,
    validate,
};
use js_sys::{Array, Function, Promise, Reflect, Uint8Array};
use std::cell::{Cell, RefCell};
use std::future::Future;
use std::path::Path;
use std::sync::Arc;
use wasm_bindgen::JsCast;
use wasm_bindgen::prelude::*;
use wasm_bindgen_futures::{JsFuture, spawn_local};
use web_sys::{
    File, FileSystemDirectoryHandle, FileSystemFileHandle, FileSystemGetDirectoryOptions,
    FileSystemGetFileOptions, FileSystemWritableFileStream,
};

use super::{Backend, Event, Loaded, Reply, Saved};
use crate::client::page_space_id;

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
export function canPick() { return 'showDirectoryPicker' in self; }
// The private dir's handle is not stored, only that it was chosen.
export async function browser() {
  const h = await navigator.storage.getDirectory();
  await idb('readwrite', s => s.put('browser', 'root')).catch(() => {});
  return h;
}
export function choose() {
  return new Promise(ok => {
    const input = document.createElement('input');
    Object.assign(input, { type: 'file', accept: '.canvas', multiple: true });
    input.onchange = () => ok([...input.files]);
    input.oncancel = () => ok([]);
    input.click();
  });
}
export function download(name, bytes) {
  const a = document.createElement('a');
  a.href = URL.createObjectURL(new Blob([bytes], { type: 'application/json' }));
  a.download = name;
  a.click();
  setTimeout(() => URL.revokeObjectURL(a.href));
}
export async function stored() { return (await idb('readonly', s => s.get('root'))) ?? null; }
export function query(h) { return h.queryPermission({ mode: 'readwrite' }); }
export function request(h) { return h.requestPermission({ mode: 'readwrite' }); }
export function sleep(ms) { return new Promise(ok => setTimeout(ok, ms)); }
"#)]
extern "C" {
    fn pick() -> Promise;
    #[wasm_bindgen(js_name = canPick)]
    pub fn can_pick() -> bool;
    fn browser() -> Promise;
    fn choose() -> Promise;
    fn download(name: &str, bytes: &Uint8Array);
    fn stored() -> Promise;
    fn query(handle: &FileSystemDirectoryHandle) -> Promise;
    fn request(handle: &FileSystemDirectoryHandle) -> Promise;
    fn sleep(ms: u32) -> Promise;
}

pub fn chosen() -> bool {
    web_sys::window()
        .and_then(|window| window.location().search().ok())
        .is_some_and(|search| search.contains("folder"))
}

// Where the spaces live: the folder on disk, or the browser's private storage.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Home {
    Folder,
    Browser,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Start {
    Loading,
    Choose,
    Reconnect,
    Open(Home),
}

enum Waiting {
    Choose,
    Reconnect(FileSystemDirectoryHandle),
}

thread_local! {
    static OPENED: (Promise, Function) = {
        let mut resolve = None;
        let opened = Promise::new(&mut |ok, _| resolve = Some(ok));
        (opened, resolve.expect("the executor runs synchronously"))
    };
    static WAITING: RefCell<Option<Waiting>> = const { RefCell::new(None) };
    static HOME: Cell<Option<Home>> = const { Cell::new(None) };
    // A space imported, to open, or what went wrong.
    static NEWS: RefCell<Vec<Result<String, String>>> = const { RefCell::new(Vec::new()) };
}

pub fn start() -> Start {
    if let Some(home) = HOME.get() {
        return Start::Open(home);
    }
    WAITING.with_borrow(|waiting| match waiting {
        None => Start::Loading,
        Some(Waiting::Choose) => Start::Choose,
        Some(Waiting::Reconnect(_)) => Start::Reconnect,
    })
}

pub fn news() -> Vec<Result<String, String>> {
    NEWS.take()
}

fn tell(news: Result<String, String>) {
    NEWS.with_borrow_mut(|all| all.push(news));
}

fn open(dir: FileSystemDirectoryHandle, home: Home) {
    WAITING.set(None);
    HOME.set(Some(home));
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
            app.add_systems(Startup, restore);
        }
    }
}

fn restore() {
    spawn_local(async {
        let waiting = match JsFuture::from(stored()).await {
            Ok(home) if home.as_string().as_deref() == Some("browser") => {
                return keep_in_browser();
            }
            Ok(dir) if !dir.is_null() => {
                let dir: FileSystemDirectoryHandle = dir.unchecked_into();
                let state = JsFuture::from(query(&dir)).await.ok();
                if state.and_then(|state| state.as_string()).as_deref() == Some("granted") {
                    return open(dir, Home::Folder);
                }
                Waiting::Reconnect(dir)
            }
            _ => Waiting::Choose,
        };
        WAITING.set(Some(waiting));
    });
}

// This one and `reconnect` are called from the system that sees the click, or
// the activation is gone. Nothing is awaited before the call.
pub fn open_folder() {
    let asked = pick();
    spawn_local(async move {
        if let Ok(dir) = JsFuture::from(asked).await {
            open(dir.unchecked_into(), Home::Folder);
        }
    });
}

pub fn reconnect() {
    let Some(dir) = WAITING.with_borrow(|waiting| match waiting {
        Some(Waiting::Reconnect(dir)) => Some(dir.clone()),
        _ => None,
    }) else {
        return;
    };
    let asked = request(&dir);
    spawn_local(async move {
        let state = JsFuture::from(asked).await.ok();
        if state.and_then(|state| state.as_string()).as_deref() == Some("granted") {
            open(dir, Home::Folder);
        }
    });
}

// Private browsing in Firefox refuses it; the start screen stays up.
pub fn keep_in_browser() {
    spawn_local(async {
        match wait(browser()).await {
            Ok(dir) => open(dir, Home::Browser),
            Err(e) => {
                WAITING.with_borrow_mut(|waiting| {
                    waiting.get_or_insert(Waiting::Choose);
                });
                tell(Err(format!("no browser storage here: {e}")));
            }
        }
    });
}

pub fn export(space: String) {
    spawn_local(async move {
        let name = file_of(&space);
        match read(&root().await, &name).await {
            Ok(Some(bytes)) => download(&name, &Uint8Array::from(&bytes[..])),
            Ok(None) => tell(Err(format!("no space {space}"))),
            Err(e) => tell(Err(e)),
        }
    });
}

// Called from the click, like `open_folder`: the file dialog needs it too.
pub fn import() {
    let chosen = choose();
    spawn_local(async move {
        let Ok(files) = JsFuture::from(chosen).await else {
            return;
        };
        let dir = root().await;
        for file in Array::from(&files) {
            tell(import_one(&dir, &file.unchecked_into()).await);
        }
    });
}

// Never over a space already there: `x.canvas` lands as `x-1` beside it.
async fn import_one(dir: &FileSystemDirectoryHandle, file: &File) -> Result<String, String> {
    let name = file.name();
    let stem = name.strip_suffix(".canvas").unwrap_or(&name);
    let buffer: JsValue = wait(file.array_buffer()).await?;
    let canvas: Canvas = serde_json::from_slice(&Uint8Array::new(&buffer).to_vec())
        .map_err(|e| format!("{name} is not a canvas: {e}"))?;
    checked(&canvas)?;
    for n in 0.. {
        let id = nth_name(stem, n);
        if !is_space_id(&id) {
            return Err(format!("{name} is not a usable name"));
        }
        if !exists(dir, &file_of(&id)).await? {
            save(dir, &id, &canvas).await?;
            return Ok(id);
        }
    }
    unreachable!("0.. never runs out")
}

fn checked(canvas: &Canvas) -> Result<(), String> {
    validate(canvas).map_err(|errors| {
        let errors: Vec<String> = errors.iter().map(ToString::to_string).collect();
        format!("refused the save: {}", errors.join("; "))
    })
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

    fn subscribe(&self, on: Box<dyn Fn(Event) + Send>) {
        spawn_local(async move {
            let dir = root().await;
            let mut seen: Option<(String, f64)> = None;
            loop {
                let _ = JsFuture::from(sleep(1000)).await;
                let Some(space) = page_space_id() else {
                    continue;
                };
                let Ok(Some(file)) = handle(&dir, &file_of(&space)).await else {
                    continue;
                };
                let Ok(file) = wait::<File>(file.get_file()).await else {
                    continue;
                };
                let modified = file.last_modified();
                // A space just opened was just fetched: only a later change is news.
                if let Some((was, at)) = &seen
                    && *was == space
                    && *at != modified
                {
                    on(Event::Changed(space.clone(), modified.to_string()));
                }
                seen = Some((space, modified));
            }
        });
    }

    // The rev check and the write have no lock between them: a writer in that gap loses (E10-T9).
    fn put(&self, space: &str, canvas: &Canvas, base: &str, reply: Reply<Result<Saved, String>>) {
        let (space, canvas, base) = (space.to_owned(), canvas.clone(), base.to_owned());
        run(reply, move |dir| async move {
            let current = load(&dir, &space).await?;
            let rev = extboard_core::rev(&current);
            if rev != base {
                return Ok(Saved::Conflict(current, rev));
            }
            checked(&canvas)?;
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

// File nodes and theme fonts, as `folder://images/x.png`.
pub struct FolderReader;

impl AssetReader for FolderReader {
    async fn read<'a>(&'a self, path: &'a Path) -> Result<impl Reader + 'a, AssetReaderError> {
        let failed = |e: String| AssetReaderError::Io(Arc::new(std::io::Error::other(e)));
        let missing = || AssetReaderError::NotFound(path.to_owned());
        let mut parts: Vec<&str> = path.iter().filter_map(|part| part.to_str()).collect();
        let name = parts.pop().ok_or_else(missing)?;
        let mut dir = root().await;
        for part in parts {
            dir = match JsFuture::from(dir.get_directory_handle(part)).await {
                Ok(sub) => sub.unchecked_into(),
                Err(e) if not_found(&e) => return Err(missing()),
                Err(e) => return Err(failed(message(e))),
            };
        }
        let bytes = read(&dir, name)
            .await
            .map_err(failed)?
            .ok_or_else(missing)?;
        Ok(VecReader::new(bytes))
    }

    async fn read_meta<'a>(&'a self, path: &'a Path) -> Result<VecReader, AssetReaderError> {
        Err(AssetReaderError::NotFound(path.to_owned()))
    }

    async fn read_directory<'a>(
        &'a self,
        path: &'a Path,
    ) -> Result<Box<PathStream>, AssetReaderError> {
        Err(AssetReaderError::NotFound(path.to_owned()))
    }

    async fn is_directory<'a>(&'a self, _: &'a Path) -> Result<bool, AssetReaderError> {
        Ok(false)
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
