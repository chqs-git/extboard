use bevy::diagnostic::FrameCount;
use bevy::picking::PickingSystems;
use bevy::prelude::*;

use crate::client::Document;
use crate::select::{PanelPress, PanelRoot, pressed_a_panel};
use crate::theme::{FG, LABEL_SIZE, PANEL_BG};

// Under the spaces screen, which covers the board this drives.
const Z: i32 = 900;
const GAP: f32 = 4.0;
const BOTTOM: f32 = 12.0;
const IDLE: Color = Color::NONE;
const HOVER: Color = Color::srgb(0.16, 0.18, 0.22);
const ON: Color = Color::srgb(0.20, 0.24, 0.33);

pub struct PalettePlugin;

// What a left drag on the board does.
#[derive(Resource, Default, Clone, Copy, PartialEq, Debug)]
pub enum Tool {
    Pan,
    #[default]
    Select,
    // A sweep draws a node, as shift does under `Select`.
    Node,
    // The only tool that shows a node's anchors or starts an edge.
    Edge,
}

// A `Pan` drag under way, which the camera follows.
#[derive(Resource, Default)]
pub struct Grab(pub bool);

#[derive(Component, Clone, Copy, PartialEq)]
enum Action {
    Tool(Tool),
    Theme,
    Scripts,
}

const ACTIONS: [(Action, &str); 6] = [
    (Action::Tool(Tool::Pan), "pan"),
    (Action::Tool(Tool::Select), "select"),
    (Action::Tool(Tool::Node), "node"),
    (Action::Tool(Tool::Edge), "edge"),
    (Action::Theme, "theme"),
    (Action::Scripts, "scripts"),
];

#[derive(Component)]
struct Hovering;

impl Plugin for PalettePlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<Tool>()
            .init_resource::<Grab>()
            .add_systems(Startup, spawn)
            // After picking, which is what says the press landed on a panel.
            .add_systems(
                PreUpdate,
                swallow
                    .after(PickingSystems::Last)
                    .before(crate::text::ReadPress),
            )
            .add_systems(Update, paint.run_if(resource_exists::<Document>));
    }
}

// Under `Pan` a left press on the board is the camera's alone: cleared here,
// nothing else sees it. A press on a panel is still the panel's.
fn swallow(
    tool: Res<Tool>,
    frames: Res<FrameCount>,
    panel: Res<PanelPress>,
    mut buttons: ResMut<ButtonInput<MouseButton>>,
    mut grab: ResMut<Grab>,
) {
    if !buttons.pressed(MouseButton::Left) {
        grab.0 = false;
    }
    if *tool == Tool::Pan
        && buttons.just_pressed(MouseButton::Left)
        && !pressed_a_panel(frames, panel)
    {
        buttons.clear_just_pressed(MouseButton::Left);
        grab.0 = true;
    }
}

fn spawn(mut commands: Commands) {
    commands
        .spawn((
            PanelRoot,
            GlobalZIndex(Z),
            // Full width, so the bar centres itself without knowing its size.
            Node {
                position_type: PositionType::Absolute,
                bottom: px(BOTTOM),
                width: percent(100.0),
                justify_content: JustifyContent::Center,
                ..default()
            },
            Pickable::IGNORE,
        ))
        .with_children(|parent| {
            parent
                .spawn((
                    Node {
                        padding: UiRect::all(px(GAP)),
                        column_gap: px(GAP),
                        border_radius: BorderRadius::all(px(6.0)),
                        ..default()
                    },
                    BackgroundColor(PANEL_BG),
                    Pickable::default(),
                ))
                .with_children(|bar| {
                    for (action, label) in ACTIONS {
                        bar.spawn(button(action, label))
                            .observe(press)
                            .observe(over)
                            .observe(out);
                    }
                });
        });
}

fn button(action: Action, label: &str) -> impl Bundle {
    (
        action,
        Node {
            padding: UiRect::axes(px(8.0), px(4.0)),
            border_radius: BorderRadius::all(px(4.0)),
            ..default()
        },
        BackgroundColor(IDLE),
        Pickable::default(),
        children![(
            Text::new(label.to_owned()),
            TextFont::from_font_size(LABEL_SIZE),
            TextColor(FG),
            Pickable::IGNORE,
        )],
    )
}

fn press(
    press: On<Pointer<Press>>,
    actions: Query<&Action>,
    document: Option<Res<Document>>,
    mut tool: ResMut<Tool>,
    mut commands: Commands,
) {
    match actions.get(press.entity) {
        Ok(Action::Tool(picked)) => *tool = *picked,
        // Both windows edit the document, so neither opens before it arrives.
        Ok(Action::Theme) if document.is_some() => commands.run_system_cached(crate::theme::flip),
        Ok(Action::Scripts) if document.is_some() => {
            commands.run_system_cached(crate::script::flip_sidebar)
        }
        _ => {}
    }
}

fn over(over: On<Pointer<Over>>, mut commands: Commands) {
    commands.entity(over.entity).insert(Hovering);
}

fn out(out: On<Pointer<Out>>, mut commands: Commands) {
    commands.entity(out.entity).remove::<Hovering>();
}

// Lit for the tool in hand and the windows that are up, whatever opened them.
fn paint(
    tool: Res<Tool>,
    theme: Res<crate::theme::Open>,
    sidebar: Res<crate::script::Sidebar>,
    mut buttons: Query<(&Action, Has<Hovering>, &mut BackgroundColor)>,
) {
    for (action, hovering, mut background) in &mut buttons {
        let on = match action {
            Action::Tool(this) => this == &*tool,
            Action::Theme => theme.0,
            Action::Scripts => sidebar.open,
        };
        let want = match (on, hovering) {
            (true, _) => ON,
            (false, true) => HOVER,
            (false, false) => IDLE,
        };
        background.set_if_neq(BackgroundColor(want));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn press(tool: Tool) -> App {
        let mut app = App::new();
        app.insert_resource(tool)
            .init_resource::<Grab>()
            .init_resource::<PanelPress>()
            .init_resource::<FrameCount>()
            .init_resource::<ButtonInput<MouseButton>>()
            .add_systems(Update, swallow);
        app.world_mut()
            .resource_mut::<ButtonInput<MouseButton>>()
            .press(MouseButton::Left);
        app.update();
        app
    }

    #[test]
    fn only_pan_takes_the_press_off_the_board() {
        let app = press(Tool::Pan);
        let buttons = app.world().resource::<ButtonInput<MouseButton>>();
        assert!(!buttons.just_pressed(MouseButton::Left));
        assert!(app.world().resource::<Grab>().0);

        let app = press(Tool::Select);
        let buttons = app.world().resource::<ButtonInput<MouseButton>>();
        assert!(buttons.just_pressed(MouseButton::Left));
        assert!(!app.world().resource::<Grab>().0);
    }
}
