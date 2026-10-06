use bevy::diagnostic::FrameCount;
use bevy::input_focus::InputFocus;
use bevy::prelude::*;
use bevy::text::{EditableText, TextEdit};
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

// What a drop or a paste leaves for the app: a file extd has written into the
// spaces dir and where on screen it arrived, or clipboard text to paste as
// nodes. Screen rather than world, because a DOM event has no camera in reach.
enum Landed {
    File {
        file: String,
        screen: Option<Vec2>,
    },
    #[cfg_attr(
        not(target_arch = "wasm32"),
        expect(
            dead_code,
            reason = "natively `keys::paste` reads the clipboard itself"
        )
    )]
    Nodes(String),
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
        Ok(response) if response.ok => lock(&uploads).push(Landed::File {
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
    window: Single<&Window>,
    camera: Single<(&Camera, &GlobalTransform), With<Camera2d>>,
    focus: Res<InputFocus>,
    mut editors: Query<&mut EditableText>,
    mut document: ResMut<Document>,
) {
    let (camera, cam_global) = *camera;
    for item in std::mem::take(&mut *lock(&uploads)) {
        let (file, screen) = match item {
            Landed::File { file, screen } => (file, screen),
            // Whatever holds the keyboard takes it at the caret, the way bevy's
            // own editor would on a key it never sees on this target.
            Landed::Nodes(text) => {
                match focus.get().and_then(|entity| editors.get_mut(entity).ok()) {
                    Some(mut editor) => editor.queue_edit(TextEdit::Insert(text.as_str().into())),
                    None => {
                        let at = world(camera, cam_global, window.cursor_position());
                        if super::keys::pasted(&mut document.0, frames.0, &text, at).is_empty() {
                            info!("paste: the clipboard holds no canvas nodes");
                        }
                    }
                }
                continue;
            }
        };
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

        let at = world(camera, cam_global, screen);
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

// Where a drop or a paste lands: where the pointer put it, else the middle of
// the view.
fn world(camera: &Camera, cam_global: &GlobalTransform, screen: Option<Vec2>) -> Vec2 {
    screen
        .and_then(|at| screen_to_world(camera, cam_global, at))
        .unwrap_or(cam_global.translation().truncate())
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

    // winit prevents the default on every keydown the canvas sees, and a
    // prevented command-V is a paste the browser never performs: no `paste`
    // event, no image. The capture phase is what takes the combo first.
    let combo =
        Closure::<dyn FnMut(web_sys::KeyboardEvent)>::new(|event: web_sys::KeyboardEvent| {
            if (event.meta_key() || event.ctrl_key()) && event.key().eq_ignore_ascii_case("v") {
                event.stop_propagation();
            }
        });
    let _ = document.add_event_listener_with_callback_and_bool(
        "keydown",
        combo.as_ref().unchecked_ref(),
        true,
    );
    combo.forget();

    let pasting = uploads.clone();
    let pasted = Closure::<dyn FnMut(web_sys::ClipboardEvent)>::new(
        move |event: web_sys::ClipboardEvent| {
            let Some(data) = event.clipboard_data() else {
                return;
            };
            if let Some(files) = data.files().filter(|files| files.length() > 0) {
                event.prevent_default();
                for i in 0..files.length() {
                    if let Some(file) = files.get(i) {
                        read(&pasting, &file, None);
                    }
                }
                return;
            }
            // Nodes come off the same event: bevy never sees the key, so this
            // is the whole of a paste on this target.
            if let Ok(text) = data.get_data("text/plain")
                && !text.is_empty()
            {
                event.prevent_default();
                lock(&pasting).push(Landed::Nodes(text));
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
