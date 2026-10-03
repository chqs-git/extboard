use bevy::picking::hover::Hovered;
use bevy::prelude::*;

use crate::theme::{FG, LABEL_SIZE, PANEL_BG};

// Over everything: the sweep band is at 1000 and the panels at 3.
const TIP_Z: i32 = 2_000;
// Clear of the cursor, which otherwise sits on the word it is asking about.
const RIGHT: f32 = 12.0;
const BELOW: f32 = 18.0;

pub struct TipPlugin;

// What a control is called, said in words while the pointer is on it: a symbol
// button is only as clear as the name behind it. `Hovered` comes with it —
// picking tracks hover for the entities that ask, and this is the asking.
#[derive(Component)]
#[require(Hovered)]
pub struct Tip(pub &'static str);

#[derive(Component)]
struct TipBox;

impl Plugin for TipPlugin {
    fn build(&self, app: &mut App) {
        app.add_systems(Startup, spawn_tip)
            .add_systems(Update, show_tip);
    }
}

// One box for the whole app, hiding itself when nothing is hovered: spawning
// per hover would lag a frame behind the pointer, and a tip that arrives late
// is a tip nobody waited for.
fn spawn_tip(mut commands: Commands) {
    commands.spawn((
        TipBox,
        Node {
            position_type: PositionType::Absolute,
            padding: UiRect::axes(px(5.0), px(2.0)),
            border_radius: BorderRadius::all(px(3.0)),
            ..default()
        },
        BackgroundColor(PANEL_BG),
        GlobalZIndex(TIP_Z),
        Text::default(),
        TextFont::from_font_size(LABEL_SIZE),
        TextColor(FG),
        Visibility::Hidden,
        // Or the tip under the cursor eats the press meant for the button it is
        // naming.
        Pickable::IGNORE,
    ));
}

// The first hovered control wins: they do not overlap, and a tip is one line.
fn show_tip(
    window: Single<&Window>,
    tips: Query<(&Tip, &Hovered)>,
    tip: Single<(&mut Node, &mut Text, &mut Visibility), With<TipBox>>,
) {
    let (mut node, mut text, mut visibility) = tip.into_inner();
    let Some((Tip(label), cursor)) = tips
        .iter()
        .find(|(_, hovered)| hovered.get())
        .map(|(tip, _)| tip)
        .zip(window.cursor_position())
    else {
        visibility.set_if_neq(Visibility::Hidden);
        return;
    };
    if text.0 != *label {
        text.0 = (*label).to_owned();
    }
    // Any write re-runs layout, so write only what moved.
    let want = (px(cursor.x + RIGHT), px(cursor.y + BELOW));
    if (node.left, node.top) != want {
        (node.left, node.top) = want;
    }
    visibility.set_if_neq(Visibility::Visible);
}

#[cfg(test)]
mod tests {
    use super::*;

    // Bevy validates a system's access at init, which is a running app rather
    // than a compile: the box holds `Node` mutably while every panel holds its
    // own, and an overlap there is a panic on startup.
    #[test]
    fn the_tip_and_the_hover_queries_stay_disjoint() {
        let mut app = App::new();
        // Neither the window nor the box exists here, so every parameter fails
        // validation. Init is what is under test, and it runs first.
        app.set_error_handler(bevy::ecs::error::ignore)
            .add_systems(Update, show_tip);
        app.world_mut().run_schedule(Update);
    }
}
