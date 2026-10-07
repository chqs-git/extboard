use extboard_core::{Canvas, rev, url_path};
use serde_json::Value;
use std::collections::{BTreeMap, HashMap};
use std::ffi::OsString;
use std::io::Write;
use std::path::PathBuf;
use std::sync::{Arc, Mutex, PoisonError};
use std::time::{SystemTime, UNIX_EPOCH};
use std::{env, fs, io};
use tokio::sync::RwLock;

pub struct Store {
    dir: PathBuf,
    spaces: Mutex<HashMap<String, Arc<RwLock<Space>>>>,
}

pub struct Space {
    id: String,
    path: PathBuf,
}

#[derive(Debug, thiserror::Error)]
pub enum StoreError {
    #[error("{0} is not a usable name")]
    BadId(String),

    #[error("no space named {0}")]
    NotFound(String),

    #[error("{0} already exists")]
    Exists(String),

    #[error("{id} is not valid JSONCanvas: {source}")]
    Parse {
        id: String,
        source: serde_json::Error,
    },

    #[error(transparent)]
    Io(#[from] io::Error),
}

type Result<T> = std::result::Result<T, StoreError>;

// How long garbage waits before the trash, and the trash before it is gone: an
// undo, an upload or a door still being saved has time to reach it again.
pub const GRACE: u64 = 5 * 24 * 60 * 60;

impl Space {
    pub fn exists(&self) -> bool {
        self.path.is_file()
    }

    pub fn load(&self) -> Result<Canvas> {
        let bytes = fs::read(&self.path).map_err(|e| match e.kind() {
            io::ErrorKind::NotFound => StoreError::NotFound(self.id.to_string()),
            _ => StoreError::Io(e),
        })?;
        serde_json::from_slice(&bytes).map_err(|source| StoreError::Parse {
            id: self.id.to_string(),
            source,
        })
    }

    // atomic op
    pub fn save(&mut self, canvas: &Canvas) -> Result<String> {
        let tmp = self.path.with_extension("canvas.tmp");

        let mut file = fs::File::create(&tmp)?;
        file.write_all(canvas.to_pretty_string().as_bytes())?;
        file.sync_all()?;
        drop(file);
        fs::rename(&tmp, &self.path)?;

        Ok(rev(canvas))
    }
}

impl Store {
    // `EXTBOARD_DIR`, else `~/extboard`. Creates the directory, so a first
    // run does not require the user to have made it.
    pub fn new() -> Result<Self> {
        let dir = resolve_dir(env::var_os("EXTBOARD_DIR"))?;
        fs::create_dir_all(&dir)?;
        Ok(Self::at(dir))
    }

    // Space ids, from the `*.canvas` filenames. Sorted so output is stable.
    pub fn list(&self) -> Result<Vec<String>> {
        let mut ids = Vec::new();
        for entry in fs::read_dir(&self.dir)? {
            let path = entry?.path();
            if path.extension().is_some_and(|ext| ext == "canvas")
                && let Some(id) = path.file_stem().and_then(|stem| stem.to_str())
            {
                ids.push(id.to_string());
            }
        }
        ids.sort();
        Ok(ids)
    }

    pub fn dir(&self) -> &std::path::Path {
        &self.dir
    }

    // A dropped file, into the one directory extd serves and the app resolves
    // a node's path against. The reply is that path, which is not always the
    // name sent: two photos called `IMG_0001.jpg` both have to land.
    pub fn put_file(&self, sub: &str, name: &str, bytes: &[u8]) -> Result<String> {
        validate_id(name)?;
        let dir = self.dir.join(sub);
        fs::create_dir_all(&dir)?;
        Ok(format!("{sub}/{}", write_new(&dir, name.as_ref(), bytes)?))
    }

    // Under both locks, taken in id order so two renames crossing each other
    // cannot deadlock. `fs::rename` clobbers, so the target is checked first.
    pub async fn rename(&self, from: &str, to: &str) -> Result<()> {
        let (source, target) = (self.space(from)?, self.space(to)?);
        if from == to {
            return Ok(());
        }
        let (first, second) = if from < to {
            (&source, &target)
        } else {
            (&target, &source)
        };
        let (first, second) = (first.write().await, second.write().await);
        let (source, target) = if from < to {
            (first, second)
        } else {
            (second, first)
        };
        if !source.exists() {
            return Err(StoreError::NotFound(from.to_string()));
        }
        if target.exists() {
            return Err(StoreError::Exists(to.to_string()));
        }
        fs::rename(&source.path, &target.path)?;
        Ok(())
    }

    // What nothing reaches: an inner space no other space links to, then the
    // images and fonts no remaining space names. A space that will not parse stops it.
    pub fn garbage(&self) -> Result<Vec<PathBuf>> {
        let mut spaces = BTreeMap::new();
        for id in self.list()? {
            let bytes = fs::read(self.dir.join(format!("{id}.canvas")))?;
            let canvas: Value =
                serde_json::from_slice(&bytes).map_err(|source| StoreError::Parse {
                    id: id.clone(),
                    source,
                })?;
            let inner = canvas.pointer("/extboard/parent").is_some();
            let mut text = String::new();
            strings(&canvas, &mut text);
            spaces.insert(id, (inner, text));
        }

        let mut garbage = Vec::new();
        // Again until nothing goes, so an orphan's own inner spaces follow it.
        while let Some(id) = spaces
            .iter()
            .find(|(id, (inner, _))| {
                *inner
                    && !spaces
                        .iter()
                        .any(|(other, (_, text))| other != *id && links_to(text, id))
            })
            .map(|(id, _)| id.clone())
        {
            spaces.remove(&id);
            garbage.push(self.dir.join(format!("{id}.canvas")));
        }

        let named: String = spaces.into_values().map(|(_, text)| text).collect();
        for sub in ["images", "fonts"] {
            let entries = match fs::read_dir(self.dir.join(sub)) {
                Err(e) if e.kind() == io::ErrorKind::NotFound => continue,
                entries => entries?,
            };
            for entry in entries {
                let entry = entry?;
                let file = format!("{sub}/{}", entry.file_name().to_string_lossy());
                if !named.contains(&file) && !named.contains(&url_path(&file)) {
                    garbage.push(entry.path());
                }
            }
        }
        garbage.sort();
        Ok(garbage)
    }

    // Moves into `.trash/<now>/` whatever has been garbage, and untouched, for
    // GRACE; empties older trash. Returns what moved.
    pub fn collect(&self, now: u64) -> Result<Vec<PathBuf>> {
        let trash = self.dir.join(".trash");
        fs::create_dir_all(&trash)?;
        let ledger = trash.join("seen.json");
        // A ledger that will not read starts the clocks again: later, never sooner.
        let seen: BTreeMap<String, u64> = match fs::read(&ledger) {
            Ok(bytes) => serde_json::from_slice(&bytes).unwrap_or_default(),
            Err(e) if e.kind() == io::ErrorKind::NotFound => BTreeMap::new(),
            Err(e) => return Err(e.into()),
        };

        let (mut waiting, mut moved) = (BTreeMap::new(), Vec::new());
        for path in self.garbage()? {
            let name = path
                .strip_prefix(&self.dir)
                .expect("garbage is in the dir")
                .to_string_lossy()
                .into_owned();
            let since = seen
                .get(&name)
                .copied()
                .unwrap_or(now)
                .max(secs(fs::metadata(&path)?.modified()?));
            if now.saturating_sub(since) < GRACE {
                waiting.insert(name, since);
                continue;
            }
            let to = trash.join(now.to_string()).join(&name);
            fs::create_dir_all(to.parent().expect("a batch dir above it"))?;
            fs::rename(&path, &to)?;
            moved.push(path);
        }
        fs::write(
            &ledger,
            serde_json::to_vec_pretty(&waiting).expect("a map of numbers"),
        )?;

        for entry in fs::read_dir(&trash)? {
            let entry = entry?;
            if let Some(at) = entry
                .file_name()
                .to_str()
                .and_then(|n| n.parse::<u64>().ok())
                && now.saturating_sub(at) >= GRACE
            {
                fs::remove_dir_all(entry.path())?;
            }
        }
        Ok(moved)
    }

    pub fn at(dir: PathBuf) -> Self {
        Self {
            dir,
            spaces: Mutex::new(HashMap::new()),
        }
    }

    // The one way to reach a space's file. Callers get the lock, not the file:
    // upsert op
    pub fn space(&self, id: &str) -> Result<Arc<RwLock<Space>>> {
        validate_id(id)?;

        let mut spaces = self.spaces.lock().unwrap_or_else(PoisonError::into_inner);
        Ok(Arc::clone(spaces.entry(id.to_string()).or_insert_with(
            || {
                Arc::new(RwLock::new(Space {
                    id: id.to_string(),
                    path: self.dir.join(format!("{id}.canvas")),
                }))
            },
        )))
    }
}

// Never overwrite: two different photos both called `IMG_0001.jpg` have to
// land. `create_new` rather than a prior `exists` check, because a multi-file
// drop uploads them at once and both would pick the same free name.
fn write_new(dir: &std::path::Path, name: &std::path::Path, bytes: &[u8]) -> io::Result<String> {
    let stem = name.file_stem().unwrap_or_default().to_string_lossy();
    let ext = match name.extension() {
        Some(ext) => format!(".{}", ext.to_string_lossy()),
        None => String::new(),
    };
    for n in 0u32.. {
        let candidate = match n {
            0 => format!("{stem}{ext}"),
            n => format!("{stem}-{n}{ext}"),
        };
        match fs::File::create_new(dir.join(&candidate)) {
            Ok(mut file) => {
                file.write_all(bytes)?;
                return Ok(candidate);
            }
            Err(e) if e.kind() == io::ErrorKind::AlreadyExists => continue,
            Err(e) => return Err(e),
        }
    }
    unreachable!("0.. never runs out")
}

// Every string in a space, one per line: a file node's path, a theme's font, a
// door's link, wherever they sit.
fn strings(value: &Value, out: &mut String) {
    match value {
        Value::String(s) => {
            out.push_str(s);
            out.push('\n');
        }
        Value::Array(items) => items.iter().for_each(|item| strings(item, out)),
        Value::Object(map) => map.values().for_each(|item| strings(item, out)),
        _ => {}
    }
}

// A door's markdown escapes the id, and no id holds a backslash, so dropping them
// all reads it back.
fn links_to(text: &str, id: &str) -> bool {
    let text = text.replace('\\', "");
    text.contains(&format!("/s/{id}")) || text.contains(&format!("/s/{}", url_path(id)))
}

pub fn secs(time: SystemTime) -> u64 {
    time.duration_since(UNIX_EPOCH)
        .map_or(0, |since| since.as_secs())
}

// Path traversal boundary: an id is one filename component, never a path.
fn validate_id(id: &str) -> Result<()> {
    if id.is_empty() || id.starts_with('.') || id.contains(['/', '\\']) {
        return Err(StoreError::BadId(id.to_string()));
    }
    Ok(())
}

fn resolve_dir(from_env: Option<OsString>) -> io::Result<PathBuf> {
    match from_env {
        Some(dir) => Ok(PathBuf::from(dir)),
        None => env::home_dir()
            .map(|home| home.join("extboard"))
            .ok_or_else(|| io::Error::other("no home directory; set EXTBOARD_DIR")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn env_override_wins_over_home() {
        let dir = resolve_dir(Some("/tmp/extboard-spaces".into())).unwrap();
        assert_eq!(dir, PathBuf::from("/tmp/extboard-spaces"));
    }

    #[test]
    fn list_keeps_canvas_files_only() {
        let dir = std::env::temp_dir().join("extboard-list-test");
        fs::create_dir_all(&dir).unwrap();
        for name in ["b.canvas", "a.canvas", "a.canvas.tmp", "notes.md"] {
            fs::write(dir.join(name), "{}").unwrap();
        }
        assert_eq!(Store::at(dir.clone()).list().unwrap(), ["a", "b"]);
        fs::remove_dir_all(&dir).unwrap();
    }

    #[tokio::test]
    async fn load_save_roundtrip_and_traversal_rejected() {
        let dir = std::env::temp_dir().join("extboard-load-test");
        fs::create_dir_all(&dir).unwrap();
        fs::write(dir.join("a.canvas"), r#"{"nodes":[],"edges":[]}"#).unwrap();
        let store = Store::at(dir.clone());
        let a = store.space("a").unwrap();

        let mut canvas = a.read().await.load().unwrap();
        canvas.nodes.push(
            serde_json::from_str(
                r#"{"id":"n1","type":"text","x":0,"y":0,"width":10,"height":10,"text":"hi"}"#,
            )
            .unwrap(),
        );
        let saved = a.write().await.save(&canvas).unwrap();
        assert_eq!(saved, rev(&a.read().await.load().unwrap()));

        // Same id twice is the same lock, or it locks nothing.
        assert!(Arc::ptr_eq(&a, &store.space("a").unwrap()));

        let missing = store.space("nope").unwrap();
        assert!(matches!(
            missing.read().await.load(),
            Err(StoreError::NotFound(_))
        ));
        for bad in ["../../etc/passwd", ".hidden", "a/b", ""] {
            assert!(
                matches!(store.space(bad), Err(StoreError::BadId(_))),
                "{bad}"
            );
        }
        fs::remove_dir_all(&dir).unwrap();
    }

    fn text_node(id: &str) -> extboard_core::Node {
        serde_json::from_str(&format!(
            r#"{{"id":"{id}","type":"text","x":0,"y":0,"width":10,"height":10,"text":"hi"}}"#
        ))
        .unwrap()
    }

    // One writer, start to finish: the whole read-modify-write under a single
    // write guard. Returns (rev it started from, rev it produced).
    async fn add_node(store: &Store, id: &str, node_id: &str) -> (String, String) {
        let space = store.space(id).unwrap();
        let mut guard = space.write().await;
        let mut canvas = guard.load().unwrap();
        let before = rev(&canvas);
        canvas.nodes.push(text_node(node_id));
        (before, guard.save(&canvas).unwrap())
    }

    // E2-T2's done-when. Without the lock both tasks read the same canvas and
    // the second save erases the first: one node on disk instead of two.
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn concurrent_writes_serialize() {
        let dir = std::env::temp_dir().join("extboard-concurrent-test");
        fs::create_dir_all(&dir).unwrap();
        fs::write(dir.join("a.canvas"), r#"{"nodes":[],"edges":[]}"#).unwrap();
        let store = Arc::new(Store::at(dir.clone()));

        let (one, two) = tokio::join!(
            tokio::spawn({
                let store = Arc::clone(&store);
                async move { add_node(&store, "a", "n1").await }
            }),
            tokio::spawn({
                let store = Arc::clone(&store);
                async move { add_node(&store, "a", "n2").await }
            }),
        );
        let (one, two) = (one.unwrap(), two.unwrap());

        // Neither write was lost, and the file is still parseable.
        let canvas = store.space("a").unwrap().read().await.load().unwrap();
        assert_eq!(canvas.nodes.len(), 2, "a write was lost");

        // The second writer started from the first writer's rev, whichever way
        // round they ran. That is the serialization, stated directly.
        assert!(
            one.1 == two.0 || two.1 == one.0,
            "writes overlapped: {one:?} {two:?}"
        );
        assert!(rev(&canvas) == one.1 || rev(&canvas) == two.1);

        fs::remove_dir_all(&dir).unwrap();
    }

    #[tokio::test]
    async fn rename_moves_the_file_and_never_clobbers() {
        let dir = std::env::temp_dir().join("extboard-rename-test");
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        for id in ["a", "b"] {
            fs::write(dir.join(format!("{id}.canvas")), id).unwrap();
        }
        let store = Store::at(dir.clone());

        store.rename("a", "c").await.unwrap();
        assert_eq!(store.list().unwrap(), ["b", "c"]);
        assert!(matches!(
            store.rename("c", "b").await,
            Err(StoreError::Exists(_))
        ));
        assert_eq!(fs::read_to_string(dir.join("b.canvas")).unwrap(), "b");
        assert!(matches!(
            store.rename("a", "d").await,
            Err(StoreError::NotFound(_))
        ));
        assert!(matches!(
            store.rename("c", "../x").await,
            Err(StoreError::BadId(_))
        ));
        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn garbage_is_what_no_space_reaches() {
        let dir = std::env::temp_dir().join("extboard-garbage-test");
        let _ = fs::remove_dir_all(&dir);
        for sub in ["images", "fonts"] {
            fs::create_dir_all(dir.join(sub)).unwrap();
        }
        let space = |id: &str, parent: Option<&str>, text: &str| {
            let parent = parent.map_or(String::new(), |p| {
                format!(r#","extboard":{{"parent":"{p}"}}"#)
            });
            fs::write(
                dir.join(format!("{id}.canvas")),
                format!(
                    r#"{{"nodes":[{{"id":"n","type":"text","x":0,"y":0,"width":1,"height":1,"text":{}}}]{parent}}}"#,
                    serde_json::to_string(text).unwrap()
                ),
            )
            .unwrap();
        };
        space(
            "top",
            None,
            r"[in](</s/top_in\<1\>>) ![](/f/images/my%20cat.png)",
        );
        space("top_in<1>", Some("top"), "images/kept.png");
        space(
            "top_orphan",
            Some("top"),
            "[deeper](/s/top_orphan_deeper) images/lost.png",
        );
        space("top_orphan_deeper", Some("top_orphan"), "");
        fs::write(
            dir.join("styled.canvas"),
            r#"{"nodes":[],"theme":{"fonts":["fonts/Used.ttf"]}}"#,
        )
        .unwrap();
        for file in [
            "images/kept.png",
            "images/my cat.png",
            "images/lost.png",
            "fonts/Used.ttf",
            "fonts/Unused.otf",
        ] {
            fs::write(dir.join(file), "").unwrap();
        }

        let garbage = Store::at(dir.clone()).garbage().unwrap();
        let names: Vec<_> = garbage
            .iter()
            .map(|path| path.strip_prefix(&dir).unwrap().to_str().unwrap())
            .collect();
        assert_eq!(
            names,
            [
                "fonts/Unused.otf",
                "images/lost.png",
                "top_orphan.canvas",
                "top_orphan_deeper.canvas"
            ]
        );

        fs::write(dir.join("broken.canvas"), "{").unwrap();
        assert!(matches!(
            Store::at(dir.clone()).garbage(),
            Err(StoreError::Parse { .. })
        ));
        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn garbage_waits_then_trashes_then_goes() {
        let dir = std::env::temp_dir().join("extboard-collect-test");
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(dir.join("images")).unwrap();
        fs::write(dir.join("images/lost.png"), "").unwrap();
        let (store, now) = (Store::at(dir.clone()), secs(SystemTime::now()));

        assert!(
            store.collect(now).unwrap().is_empty(),
            "first seen, not yet gone"
        );
        assert!(dir.join("images/lost.png").exists());

        let later = now + GRACE;
        assert_eq!(store.collect(later).unwrap(), [dir.join("images/lost.png")]);
        let trashed = dir.join(format!(".trash/{later}/images/lost.png"));
        assert!(trashed.exists());

        store.collect(later + GRACE).unwrap();
        assert!(!trashed.exists(), "old trash is emptied");
        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn home_is_the_fallback() {
        let dir = resolve_dir(None).unwrap();
        assert!(dir.ends_with("extboard"), "{dir:?}");
        assert!(dir.is_absolute(), "{dir:?}");
    }
}
