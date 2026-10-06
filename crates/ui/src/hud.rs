use bevy::diagnostic::{DiagnosticsStore, FrameTimeDiagnosticsPlugin};
use bevy::prelude::*;

pub struct HudPlugin;

#[derive(Component)]
struct FpsText;

impl Plugin for HudPlugin {
    fn build(&self, app: &mut App) {
        app.add_plugins(FrameTimeDiagnosticsPlugin::default())
            .add_systems(Startup, spawn_fps)
            .add_systems(Update, update_fps);
    }
}

fn spawn_fps(mut commands: Commands) {
    commands.spawn((
        FpsText,
        // Node panels are UI too and spawn later. Under the space's name.
        GlobalZIndex(1),
        Text::new("-- fps"),
        TextFont::from_font_size(14.0),
        TextColor(Color::srgb(0.45, 0.5, 0.55)),
        Node {
            position_type: PositionType::Absolute,
            top: Val::Px(28.0),
            left: Val::Px(8.0),
            ..default()
        },
    ));
}

fn update_fps(diagnostics: Res<DiagnosticsStore>, mut text: Query<&mut Text, With<FpsText>>) {
    let Ok(mut text) = text.single_mut() else {
        return;
    };
    // `smoothed` instead of the raw value: an unsmoothed counter is unreadable.
    let Some(fps) = diagnostics
        .get(&FrameTimeDiagnosticsPlugin::FPS)
        .and_then(|fps| fps.smoothed())
    else {
        return;
    };
    text.0 = format!("{fps:.0} fps");
}
