use bevy::diagnostic::FrameCount;
use bevy::prelude::*;
use bevy::window::FileDragAndDrop;
use extboard_core::{NodeKind, is_image};
use std::path::{Path, PathBuf};

use crate::client::{Document, files_root};
use crate::select::cursor_world;

use super::added;

// What a dropped image lands as; it resizes like any other node.
const DROP_SIZE: Vec2 = Vec2::new(320.0, 240.0);
const CAPTION_SIZE: Vec2 = Vec2::new(320.0, 60.0);
// Under the spaces dir, which is both what extd serves and the asset root.
const IMAGES: &str = "images";

pub(super) fn dropped(
    mut dropped: MessageReader<FileDragAndDrop>,
    frames: Res<FrameCount>,
    window: Single<&Window>,
    camera: Single<(&Camera, &GlobalTransform), With<Camera2d>>,
    mut document: ResMut<Document>,
) {
    for event in dropped.read() {
        let FileDragAndDrop::DroppedFile { path_buf, .. } = event else {
            continue;
        };
        if !is_image(&path_buf.to_string_lossy()) {
            warn!(
                "{} is not an image, so nothing was added",
                path_buf.display()
            );
            continue;
        }
        let Some(file) = copy_in(path_buf) else {
            continue;
        };

        // A drag does not move the cursor on every platform, so the middle of
        // the view is where a drop lands when there is no cursor to read.
        let at = cursor_world(&window, *camera).unwrap_or(camera.1.translation().truncate());
        added(
            &mut document.0,
            frames.0,
            Rect::from_center_size(at, DROP_SIZE),
            NodeKind::File {
                file: file.clone(),
                subpath: None,
            },
        );
        // The caption, as its own node: it is what `extd project` sends the
        // model, and what the editor already knows how to rewrite.
        added(
            &mut document.0,
            frames.0,
            Rect::from_center_size(
                at - Vec2::new(0.0, (DROP_SIZE.y + CAPTION_SIZE.y) / 2.0),
                CAPTION_SIZE,
            ),
            NodeKind::Text {
                text: caption(&file),
            },
        );
    }
}

// Into the spaces dir, because that is the one directory extd serves and the
// one the app resolves a node's path against. The node keeps the relative path.
fn copy_in(from: &Path) -> Option<String> {
    let dir = PathBuf::from(files_root()).join(IMAGES);
    if let Err(e) = std::fs::create_dir_all(&dir) {
        error!("{}: {e}", dir.display());
        return None;
    }
    let name = free_name(&dir, from.file_name()?.as_ref());
    if let Err(e) = std::fs::copy(from, dir.join(&name)) {
        error!("{} -> {}: {e}", from.display(), dir.display());
        return None;
    }
    Some(format!("{IMAGES}/{name}"))
}

// Never overwrite: two different photos both called `IMG_0001.jpg` have to land.
fn free_name(dir: &Path, name: &Path) -> String {
    let stem = name.file_stem().unwrap_or_default().to_string_lossy();
    let ext = match name.extension() {
        Some(ext) => format!(".{}", ext.to_string_lossy()),
        None => String::new(),
    };
    (0..)
        .map(|n| match n {
            0 => format!("{stem}{ext}"),
            n => format!("{stem}-{n}{ext}"),
        })
        .find(|name| !dir.join(name).exists())
        .expect("0.. never runs out")
}

// The filename is the only alt text a drop can know, so it is a starting point
// to retype rather than an answer.
fn caption(file: &str) -> String {
    let stem = Path::new(file)
        .file_stem()
        .unwrap_or_default()
        .to_string_lossy();
    format!("## {}", stem.replace(['-', '_'], " "))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_caption_is_a_heading_made_of_the_filename() {
        assert_eq!(caption("images/a-red_bike.png"), "## a red bike");
        assert_eq!(caption("images/IMG_0001.JPG"), "## IMG 0001");
    }

    #[test]
    fn a_taken_name_is_never_overwritten() {
        let dir = std::env::temp_dir().join("extboard-drop-test");
        std::fs::create_dir_all(&dir).unwrap();
        let taken = |name: &str| std::fs::write(dir.join(name), b"x").unwrap();

        assert_eq!(free_name(&dir, "a.png".as_ref()), "a.png");
        taken("a.png");
        assert_eq!(free_name(&dir, "a.png".as_ref()), "a-1.png");
        taken("a-1.png");
        assert_eq!(free_name(&dir, "a.png".as_ref()), "a-2.png");
        assert_eq!(free_name(&dir, "b".as_ref()), "b");

        std::fs::remove_dir_all(&dir).unwrap();
    }
}
