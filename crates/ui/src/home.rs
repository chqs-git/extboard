use bevy::prelude::*;

use crate::backend::folder::{self, Home, Start};
use crate::client::{Notice, Space};
use crate::select::PanelRoot;
use crate::spaces::{EDGE, FG, HOVER, TILE_BG};
use crate::theme::DARK;

// Over the spaces screen at 2000.
const Z: i32 = 3_000;
const LABEL: Color = Color::srgb(0.6, 0.65, 0.72);

pub struct HomePlugin;

impl Plugin for HomePlugin {
    fn build(&self, app: &mut App) {
        if folder::chosen() {
            app.add_systems(Update, (hear, draw));
        }
    }
}

#[derive(Component)]
struct Shown;

#[derive(Component, Clone, Copy)]
enum Act {
    OpenFolder,
    Browser,
    Reconnect,
    Export,
    Import,
}

fn hear(mut space: ResMut<Space>, mut notice: ResMut<Notice>) {
    for news in folder::news() {
        match news {
            Ok(id) => space.0 = Some(id),
            Err(message) => {
                error!("{message}");
                notice.0 = Some(message);
            }
        }
    }
}

// Polled: the state lives in the folder backend's thread-locals, not in Bevy.
fn draw(
    mut commands: Commands,
    space: Res<Space>,
    mut was: Local<Option<(Start, bool)>>,
    shown: Query<Entity, With<Shown>>,
) {
    let now = (folder::start(), space.id().is_some());
    if *was == Some(now) {
        return;
    }
    *was = Some(now);
    for entity in &shown {
        commands.entity(entity).despawn();
    }
    match now {
        (Start::Loading, _) => {}
        (Start::Choose, _) => {
            commands.spawn(screen()).with_children(|parent| {
                parent.spawn(label("ext.board", FG, 20.0));
                if folder::can_pick() {
                    button(parent, Act::OpenFolder, "Open folder");
                } else {
                    parent.spawn(label(
                        "Folder access needs Chrome, Edge or Arc. Boards here stay in this browser.",
                        LABEL,
                        13.0,
                    ));
                }
                button(parent, Act::Browser, "Keep boards in this browser");
            });
        }
        (Start::Reconnect, _) => {
            commands.spawn(badge()).with_children(|parent| {
                button(parent, Act::Reconnect, "Reconnect ~/extboard");
            });
        }
        (Start::Open(Home::Folder), _) => {
            commands.spawn(badge()).with_child(label(
                "Folder: shared with desktop and Claude",
                LABEL,
                12.0,
            ));
        }
        (Start::Open(Home::Browser), open) => {
            commands.spawn(badge()).with_children(|parent| {
                parent.spawn(label("Browser storage: not shared", LABEL, 12.0));
                if open {
                    button(parent, Act::Export, "Export .canvas");
                }
                button(parent, Act::Import, "Import");
            });
        }
    }
}

// The picker and the file dialog are opened right here, inside the click.
fn press(mut press: On<Pointer<Press>>, acts: Query<&Act>, space: Res<Space>) {
    press.propagate(false);
    match acts.get(press.entity) {
        Ok(Act::OpenFolder) => folder::open_folder(),
        Ok(Act::Browser) => folder::keep_in_browser(),
        Ok(Act::Reconnect) => folder::reconnect(),
        Ok(Act::Export) => {
            if let Some(id) = space.id() {
                folder::export(id.to_owned());
            }
        }
        Ok(Act::Import) => folder::import(),
        Err(_) => {}
    }
}

fn hover(over: On<Pointer<Over>>, mut colors: Query<&mut BackgroundColor, With<Act>>) {
    if let Ok(mut color) = colors.get_mut(over.entity) {
        color.0 = HOVER;
    }
}

fn unhover(out: On<Pointer<Out>>, mut colors: Query<&mut BackgroundColor, With<Act>>) {
    if let Ok(mut color) = colors.get_mut(out.entity) {
        color.0 = TILE_BG;
    }
}

fn screen() -> impl Bundle {
    (
        Shown,
        PanelRoot,
        GlobalZIndex(Z),
        Node {
            position_type: PositionType::Absolute,
            width: percent(100.0),
            height: percent(100.0),
            padding: UiRect::all(px(24.0)),
            row_gap: px(16.0),
            flex_direction: FlexDirection::Column,
            align_items: AlignItems::Center,
            justify_content: JustifyContent::Center,
            ..default()
        },
        BackgroundColor(DARK),
        Pickable::default(),
    )
}

// Top right, under the space chip.
fn badge() -> impl Bundle {
    (
        Shown,
        PanelRoot,
        GlobalZIndex(Z),
        Node {
            position_type: PositionType::Absolute,
            right: px(8.0),
            top: px(36.0),
            column_gap: px(6.0),
            align_items: AlignItems::Center,
            ..default()
        },
    )
}

fn label(text: &str, color: Color, size: f32) -> impl Bundle {
    (
        Text::new(text.to_owned()),
        TextFont::from_font_size(size),
        TextColor(color),
        Pickable::IGNORE,
    )
}

fn button(parent: &mut ChildSpawnerCommands, act: Act, text: &str) {
    parent
        .spawn((
            act,
            Node {
                padding: UiRect::axes(px(12.0), px(6.0)),
                border: UiRect::all(px(1.0)),
                border_radius: BorderRadius::all(px(6.0)),
                ..default()
            },
            BackgroundColor(TILE_BG),
            BorderColor::all(EDGE),
            Pickable::default(),
            children![label(text, FG, 13.0)],
        ))
        .observe(press)
        .observe(hover)
        .observe(unhover);
}
