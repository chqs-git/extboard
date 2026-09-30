use crate::api::AppState;
use crate::store::StoreError;
use axum::Json;
use axum::extract::State;
use serde::Serialize;
use std::io::{BufRead, BufReader, Read};
use std::path::Path;
use std::{fs, io};

#[derive(Debug, PartialEq, Serialize)]
pub struct Command {
    pub name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
}

pub async fn list(State(app): State<AppState>) -> Result<Json<Vec<Command>>, StoreError> {
    Ok(Json(executables(&app.store.dir().join("commands"))?))
}

fn executables(dir: &Path) -> io::Result<Vec<Command>> {
    let entries = match fs::read_dir(dir) {
        Ok(entries) => entries,
        // No commands directory is a board with no commands, not a failure.
        Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(e) => return Err(e),
    };

    let mut commands = Vec::new();
    for entry in entries {
        let entry = entry?;
        let name = entry.file_name().to_string_lossy().into_owned();
        if name.starts_with('.') {
            continue;
        }
        // `fs::metadata`, not `entry.metadata()`: a symlink into ~/bin is a
        // command. A dangling one is skipped rather than failing the listing.
        let path = entry.path();
        let Ok(meta) = fs::metadata(&path) else {
            continue;
        };
        if meta.is_file() && is_executable(&meta) {
            commands.push(Command {
                description: description(&path),
                name,
            });
        }
    }
    commands.sort_by(|a, b| a.name.cmp(&b.name));
    Ok(commands)
}

#[cfg(unix)]
fn is_executable(meta: &fs::Metadata) -> bool {
    use std::os::unix::fs::PermissionsExt;
    meta.permissions().mode() & 0o111 != 0
}

// No executable bit off Unix, so no commands. Windows is not a target (README).
#[cfg(not(unix))]
fn is_executable(_: &fs::Metadata) -> bool {
    false
}

fn description(path: &Path) -> Option<String> {
    let file = fs::File::open(path).ok()?;
    BufReader::new(file.take(512))
        .lines()
        .map_while(Result::ok)
        .take(5)
        .find_map(|line| {
            Some(
                line.trim()
                    .strip_prefix("# description:")?
                    .trim()
                    .to_owned(),
            )
        })
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;
    use std::os::unix::fs::PermissionsExt;

    fn write_exec(dir: &Path, name: &str, body: &str) {
        let path = dir.join(name);
        fs::write(&path, body).unwrap();
        fs::set_permissions(&path, fs::Permissions::from_mode(0o755)).unwrap();
    }

    fn names(commands: &[Command]) -> Vec<&str> {
        commands.iter().map(|c| c.name.as_str()).collect()
    }

    #[test]
    fn lists_executables_only_and_picks_new_ones_up_without_a_restart() {
        let dir = std::env::temp_dir().join("extboard-commands-test");
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();

        assert!(executables(&dir.join("absent")).unwrap().is_empty());

        write_exec(&dir, "b-plain", "#!/bin/sh\necho hi\n");
        write_exec(
            &dir,
            "a-described",
            "#!/bin/sh\n# description:  adds a node \necho hi\n",
        );
        write_exec(&dir, ".hidden", "#!/bin/sh\n");
        fs::write(dir.join("notes.md"), "#!/bin/sh\n").unwrap();
        fs::create_dir(dir.join("sub")).unwrap();

        let got = executables(&dir).unwrap();
        assert_eq!(names(&got), ["a-described", "b-plain"]);
        assert_eq!(got[0].description.as_deref(), Some("adds a node"));
        assert_eq!(got[1].description, None);

        // The done-when: dropped in after the first read, no restart.
        write_exec(&dir, "c-new", "#!/bin/sh\n");
        assert_eq!(
            names(&executables(&dir).unwrap()),
            ["a-described", "b-plain", "c-new"]
        );

        fs::remove_dir_all(&dir).unwrap();
    }
}
