use bevy::prelude::*;
use std::sync::{Mutex, PoisonError};

// The open space extd refused, and its reason: locked, or a wrong password.
#[derive(Resource, Default)]
pub struct Locked(pub Option<(String, String)>);

// Every password typed this run. extd tries each against the space's lock, so
// none has to be filed under the space it opens. A tab has its cookie instead.
static KEYRING: Mutex<Vec<String>> = Mutex::new(Vec::new());

pub fn keyed(mut request: ehttp::Request) -> ehttp::Request {
    for password in keyring().iter() {
        request.headers.insert("x-extboard-password", password);
    }
    request
}

fn keyring() -> std::sync::MutexGuard<'static, Vec<String>> {
    KEYRING.lock().unwrap_or_else(PoisonError::into_inner)
}

// Board keys stand down while the lock is up: there is no board under it.
pub fn typing(locked: Res<Locked>) -> bool {
    locked.0.is_some()
}

pub struct LockPlugin;

impl Plugin for LockPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<Locked>();
        #[cfg(target_arch = "wasm32")]
        app.add_systems(Update, web::draw.run_if(resource_changed::<Locked>));
        #[cfg(not(target_arch = "wasm32"))]
        app.init_resource::<native::Typed>().add_systems(
            Update,
            (
                native::forget.run_if(resource_changed::<Locked>),
                native::draw.run_if(resource_changed::<Locked>),
                native::mask,
                native::submit,
            )
                .chain(),
        );
    }
}

#[cfg(not(target_arch = "wasm32"))]
mod native {
    use super::*;
    use crate::client::Space;
    use crate::select::PanelRoot;
    use crate::spaces::{EDGE, FG, TILE_BG};
    use crate::theme::DARK;
    use bevy::input_focus::AutoFocus;
    use bevy::text::{EditableText, TextCursorStyle};

    // Under the spaces chrome at 2000, so the way back to the list stays on top.
    const Z: i32 = 1_999;

    // The last password typed, and the space it was typed for: refused there
    // again, it is wrong and leaves the keyring.
    #[derive(Resource, Default)]
    pub struct Typed(Option<(String, String)>);

    #[derive(Component)]
    pub struct LockScreen;

    #[derive(Component)]
    pub struct PasswordField;

    #[derive(Component)]
    pub struct Dots;

    pub fn forget(locked: Res<Locked>, mut typed: ResMut<Typed>) {
        let Some((space, _)) = &locked.0 else {
            return;
        };
        if let Some((_, password)) = typed.0.take_if(|(tried, _)| tried == space) {
            keyring().retain(|held| *held != password);
        }
    }

    pub fn draw(
        mut commands: Commands,
        locked: Res<Locked>,
        existing: Query<Entity, With<LockScreen>>,
    ) {
        for entity in &existing {
            commands.entity(entity).despawn();
        }
        let Some((_, reason)) = &locked.0 else {
            return;
        };
        let font = TextFont::from_font_size(16.0);
        commands.spawn((
            LockScreen,
            PanelRoot,
            GlobalZIndex(Z),
            Node {
                position_type: PositionType::Absolute,
                width: percent(100.0),
                height: percent(100.0),
                flex_direction: FlexDirection::Column,
                align_items: AlignItems::Center,
                justify_content: JustifyContent::Center,
                row_gap: px(12.0),
                ..default()
            },
            BackgroundColor(DARK),
            Pickable::default(),
            children![
                (
                    Text::new(reason.clone()),
                    TextFont::from_font_size(14.0),
                    TextColor(FG),
                ),
                (
                    Node {
                        width: px(220.0),
                        padding: UiRect::axes(px(8.0), px(6.0)),
                        border: UiRect::all(px(2.0)),
                        border_radius: BorderRadius::all(px(8.0)),
                        ..default()
                    },
                    BackgroundColor(TILE_BG),
                    BorderColor::all(EDGE),
                    children![
                        // Typed into but never shown: bevy's field has no mask.
                        (
                            PasswordField,
                            Node {
                                width: percent(100.0),
                                ..default()
                            },
                            EditableText::new(""),
                            font.clone(),
                            TextColor(Color::NONE),
                            TextCursorStyle {
                                color: Color::NONE,
                                selection_color: Color::NONE,
                                unfocused_selection_color: Color::NONE,
                                selected_text_color: Some(Color::NONE),
                            },
                            AutoFocus,
                        ),
                        (
                            Dots,
                            Text::new("|"),
                            font,
                            TextColor(FG),
                            Node {
                                position_type: PositionType::Absolute,
                                left: px(8.0),
                                top: px(6.0),
                                ..default()
                            },
                            Pickable::IGNORE,
                        ),
                    ],
                ),
            ],
        ));
    }

    pub fn mask(
        fields: Query<&EditableText, With<PasswordField>>,
        mut dots: Query<&mut Text, With<Dots>>,
    ) {
        let (Ok(field), Ok(mut dots)) = (fields.single(), dots.single_mut()) else {
            return;
        };
        let shown = format!("{}|", "*".repeat(field.value().to_string().chars().count()));
        if dots.0 != shown {
            dots.0 = shown;
        }
    }

    // Enter tries again with the keyring one longer; escape goes back to the list.
    pub fn submit(
        keys: Res<ButtonInput<KeyCode>>,
        fields: Query<&EditableText, With<PasswordField>>,
        mut locked: ResMut<Locked>,
        mut typed: ResMut<Typed>,
        mut space: ResMut<Space>,
    ) {
        let Some((id, _)) = &locked.0 else {
            return;
        };
        if keys.just_pressed(KeyCode::Escape) {
            space.0 = None;
            return;
        }
        let Ok(field) = fields.single() else {
            return;
        };
        let password = field.value().to_string().trim().to_owned();
        if password.is_empty() || !keys.any_just_pressed([KeyCode::Enter, KeyCode::NumpadEnter]) {
            return;
        }
        let mut keyring = keyring();
        if !keyring.contains(&password) {
            keyring.push(password.clone());
        }
        typed.0 = Some((id.clone(), password));
        locked.0 = None;
        // A switch to the same space: the board, its path and the list refetch.
        space.set_changed();
    }
}

// An HTML form, not a bevy field: it masks what is typed and raises a phone's
// keyboard. extd answers it with a cookie, kept by the browser, and this page.
#[cfg(target_arch = "wasm32")]
mod web {
    use super::*;
    use wasm_bindgen::JsCast;

    const PROMPT: &str = "lock";

    pub fn draw(locked: Res<Locked>) {
        match &locked.0 {
            Some((space, reason)) => {
                prompt(space, reason);
            }
            None => unprompt(),
        }
    }

    fn prompt(space: &str, reason: &str) -> Option<()> {
        let document = web_sys::window()?.document()?;
        if document.get_element_by_id(PROMPT).is_some() {
            return Some(());
        }
        let form = document.create_element("form").ok()?;
        form.set_id(PROMPT);
        form.set_attribute("method", "post").ok()?;
        let action = format!("/api/spaces/{}/unlock", extboard_core::url_path(space));
        form.set_attribute("action", &action).ok()?;
        form.set_inner_html(
            r#"<p></p><input type="password" name="password" autocomplete="current-password" placeholder="password"><a href="/">&larr; spaces</a>"#,
        );
        form.first_element_child()?.set_text_content(Some(reason));
        document.body()?.append_child(&form).ok()?;
        let input = form.query_selector("input").ok()??;
        input.dyn_into::<web_sys::HtmlElement>().ok()?.focus().ok()
    }

    fn unprompt() {
        if let Some(form) = web_sys::window()
            .and_then(|window| window.document())
            .and_then(|document| document.get_element_by_id(PROMPT))
        {
            form.remove();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_password_held_rides_on_the_request() {
        keyring().extend(["a".to_owned(), "b".to_owned()]);
        let request = keyed(ehttp::Request::get("http://x"));
        let sent: Vec<&str> = request.headers.get_all("x-extboard-password").collect();
        assert_eq!(sent, ["a", "b"]);
    }
}
