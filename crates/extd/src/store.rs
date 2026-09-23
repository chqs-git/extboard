use extboard_core::{Canvas, rev};
use std::collections::HashMap;
use std::ffi::OsString;
use std::io::Write;
use std::path::PathBuf;
use std::sync::{Arc, Mutex, PoisonError};
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
    #[error("{0} is not a usable space id")]
    BadId(String),

    #[error("no space named {0}")]
    NotFound(String),

    #[error("{id} is not valid JSONCanvas: {source}")]
    Parse {
        id: String,
        source: serde_json::Error,
    },

    #[error(transparent)]
    Io(#[from] io::Error),
}

type Result<T> = std::result::Result<T, StoreError>;

impl Space {
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

    #[test]
    fn home_is_the_fallback() {
        let dir = resolve_dir(None).unwrap();
        assert!(dir.ends_with("extboard"), "{dir:?}");
        assert!(dir.is_absolute(), "{dir:?}");
    }
}
