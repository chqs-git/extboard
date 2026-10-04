use bevy::diagnostic::FrameCount;
use bevy::prelude::*;
use extboard_core::{NodeKind, PRIMARY_TEXT, is_font, is_image, url_path};
use std::path::Path;
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};

use crate::camera::screen_to_world;
use crate::client::{Document, base_url};
use crate::theme::write_fonts;

use super::added;

// What a dropped image lands as; it resizes like any other node.
const DROP_SIZE: Vec2 = Vec2::new(320.0, 240.0);
const CAPTION_SIZE: Vec2 = Vec2::new(320.0, 60.0);

// A file extd has written into the spaces dir: its path there, and where on
// screen it was dropped. Screen rather than world, because on the web the drop
// is a DOM event with no camera in reach.
struct Landed {
    file: String,
    screen: Option<Vec2>,
}

// Uploads answer off-thread, so the node they add waits for a frame.
#[derive(Resource, Default, Clone)]
pub(super) struct Uploads(Arc<Mutex<Vec<Landed>>>);

#[cfg(not(target_arch = "wasm32"))]
pub(super) fn dropped(
    mut dropped: MessageReader<bevy::window::FileDragAndDrop>,
    window: Single<&Window>,
    uploads: Res<Uploads>,
) {
    use bevy::window::FileDragAndDrop;

    for event in dropped.read() {
        let FileDragAndDrop::DroppedFile { path_buf, .. } = event else {
            continue;
        };
        let name = path_buf
            .file_name()
            .unwrap_or_default()
            .to_string_lossy()
            .into_owned();
        match std::fs::read(path_buf) {
            // A drag does not move the cursor on every platform; `landed`
            // falls back to the middle of the view when there is none to read.
            Ok(bytes) => upload(&uploads, &name, bytes, window.cursor_position()),
            Err(e) => error!("{}: {e}", path_buf.display()),
        }
    }
}

// extd owns the spaces dir on both targets: the browser cannot write to it at
// all, and one uploader beats two ways of putting a file in one directory.
pub(super) fn upload(uploads: &Uploads, name: &str, bytes: Vec<u8>, screen: Option<Vec2>) {
    if !is_image(name) && !is_font(name) {
        warn!("{name} is neither an image nor a font, so nothing was added");
        return;
    }
    let request = ehttp::Request::post(
        format!("{}/api/files/{}", base_url(), url_path(name)),
        bytes,
    );

    let uploads = uploads.clone();
    ehttp::fetch(request, move |result| match result {
        // The reply is the path extd settled on, which is not always the name
        // we sent: it never overwrites.
        Ok(response) if response.ok => lock(&uploads).push(Landed {
            file: String::from_utf8_lossy(&response.bytes).into_owned(),
            screen,
        }),
        Ok(response) => error!("extd refused the upload: {}", response.status),
        Err(e) => error!("upload failed: {e}"),
    });
}

pub(super) fn landed(
    uploads: Res<Uploads>,
    frames: Res<FrameCount>,
    camera: Single<(&Camera, &GlobalTransform), With<Camera2d>>,
    mut document: ResMut<Document>,
) {
    let (camera, cam_global) = *camera;
    for Landed { file, screen } in std::mem::take(&mut *lock(&uploads)) {
        // A font is the space's, not a node's: it lands in the library and
        // takes the primary text, because every text on the board follows that
        // one -- a drop you cannot see is a drop that did not work.
        if is_font(&file) {
            let mut fonts = document.0.fonts();
            fonts.add(&file);
            fonts.set(PRIMARY_TEXT, &file);
            write_fonts(&mut document, &fonts);
            continue;
        }

        let at = screen
            .and_then(|at| screen_to_world(camera, cam_global, at))
            .unwrap_or(cam_global.translation().truncate());
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

// A browser drop is a DOM event: bevy's winit gives us a filename with no
// bytes behind it, so the page's own listener is what reads the file.
#[cfg(target_arch = "wasm32")]
pub(super) fn listen(uploads: Res<Uploads>) {
    use wasm_bindgen::JsCast;
    use wasm_bindgen::closure::Closure;

    let Some(document) = web_sys::window().and_then(|window| window.document()) else {
        error!("no document: a drop in the browser cannot be read");
        return;
    };

    // Without this the browser opens the file instead of letting it drop.
    let over = Closure::<dyn FnMut(web_sys::DragEvent)>::new(|event: web_sys::DragEvent| {
        event.prevent_default();
    });
    for name in ["dragenter", "dragover"] {
        let _ = document.add_event_listener_with_callback(name, over.as_ref().unchecked_ref());
    }
    over.forget();

    let pasting = uploads.clone();
    let pasted = Closure::<dyn FnMut(web_sys::ClipboardEvent)>::new(
        move |event: web_sys::ClipboardEvent| {
            let Some(files) = event.clipboard_data().and_then(|data| data.files()) else {
                return;
            };
            if files.length() == 0 {
                return;
            }
            event.prevent_default();
            for i in 0..files.length() {
                if let Some(file) = files.get(i) {
                    read(&pasting, &file, None);
                }
            }
        },
    );
    let _ = document.add_event_listener_with_callback("paste", pasted.as_ref().unchecked_ref());
    pasted.forget();

    let uploads = uploads.clone();
    let dropped =
        Closure::<dyn FnMut(web_sys::DragEvent)>::new(move |event: web_sys::DragEvent| {
            event.prevent_default();
            let Some(files) = event.data_transfer().and_then(|data| data.files()) else {
                return;
            };
            // Client coordinates are CSS pixels off the viewport, which is what
            // bevy calls a cursor position as long as the canvas fills the page.
            let at = Vec2::new(event.client_x() as f32, event.client_y() as f32);
            for i in 0..files.length() {
                if let Some(file) = files.get(i) {
                    read(&uploads, &file, Some(at));
                }
            }
        });
    let _ = document.add_event_listener_with_callback("drop", dropped.as_ref().unchecked_ref());
    dropped.forget();
}

// FileReader rather than a future: it keeps the whole path callback-shaped,
// the way ehttp already is, and needs no async runtime in the tab.
#[cfg(target_arch = "wasm32")]
fn read(uploads: &Uploads, file: &web_sys::File, screen: Option<Vec2>) {
    use wasm_bindgen::JsCast;
    use wasm_bindgen::closure::Closure;

    let Ok(reader) = web_sys::FileReader::new() else {
        error!("no FileReader: a drop in the browser cannot be read");
        return;
    };
    let name = file.name();
    let uploads = uploads.clone();
    let done = Closure::<dyn FnMut()>::new({
        let reader = reader.clone();
        move || match reader.result() {
            Ok(buffer) => upload(
                &uploads,
                &name,
                js_sys::Uint8Array::new(&buffer).to_vec(),
                screen,
            ),
            Err(_) => error!("{name} could not be read"),
        }
    });
    reader.set_onload(Some(done.as_ref().unchecked_ref()));
    done.forget();

    if reader.read_as_array_buffer(file).is_err() {
        error!("{} could not be read", file.name());
    }
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

fn lock(uploads: &Uploads) -> MutexGuard<'_, Vec<Landed>> {
    uploads.0.lock().unwrap_or_else(PoisonError::into_inner)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_caption_is_a_heading_made_of_the_filename() {
        assert_eq!(caption("images/a-red_bike.png"), "## a red bike");
        assert_eq!(caption("images/IMG_0001.JPG"), "## IMG 0001");
    }
}
