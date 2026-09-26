//! E3-T1 · the text spike. Throwaway: `unwrap()` everywhere, no error handling,
//! no tests. Deleted once PLAN.md records the decision.
//!
//! Two markdown renderers, same source, side by side, both anchored to a
//! world-space rect so camera zoom drives them:
//!
//!   left  — path A: bevy_egui + egui_commonmark (off the shelf)
//!   right — path C: bevy_ui `Text`/`TextSpan` + `Display::Grid` (new in 0.19;
//!           did not exist when the card was written)
//!
//! `-`/`=` zoom, `space` toggles the right panel between rendered markdown and
//! an `EditableText` source view, `t` switches the native panel's zoom between
//! `UiScale` (re-layout + re-rasterise) and `UiTransform.scale` (transform the
//! laid-out node). Crispness at 4x is the whole question there.

use bevy::prelude::*;
use bevy::render::view::screenshot::{save_to_disk, Screenshot};
use bevy::text::{EditableText, FontSmoothing, FontSource, FontWeight, LineHeight};
use bevy_egui::{egui, EguiContexts, EguiPlugin, EguiPrimaryContextPass};
use egui_commonmark::{CommonMarkCache, CommonMarkViewer};
use pulldown_cmark::{Event, HeadingLevel, Parser, Tag, TagEnd};

/// Everything the spike has to survive: headings, emphasis, a list, a code
/// block, a link, and a table. The table is the real question.
const MD: &str = r#"# Node title

A paragraph with *emphasis*, **strong**, `inline_code()` and a
[link](https://bevy.org).

- first item
- second item with **bold**
- third

```rust
fn main() {
    println!("code block");
}
```

| crate | version | verdict |
|---|---|---|
| bevy | 0.19.1 | parley text, EditableText |
| bevy_egui | 0.42 | egui 0.36 |
| egui_commonmark | 0.25 | tables via egui_extras |
"#;

/// World-space node rect, shared by both panels (x is the left edge).
const NODE: Rect = Rect {
    min: Vec2::new(-620.0, -300.0),
    max: Vec2::new(-20.0, 300.0),
};

#[derive(Resource)]
struct Zoom(f32);

#[derive(Resource, Default)]
struct SourceMode(bool);

/// false = `UiScale`, true = `UiTransform.scale`.
#[derive(Resource, Default)]
struct TransformZoom(bool);

#[derive(Component)]
struct NativePanel;

#[derive(Component)]
struct Hud;

fn main() {
    App::new()
        .add_plugins(DefaultPlugins.set(WindowPlugin {
            primary_window: Some(Window {
                title: "text spike".into(),
                resolution: (1400u32, 760u32).into(),
                ..default()
            }),
            ..default()
        }))
        .add_plugins(EguiPlugin::default())
        .insert_resource(Zoom(1.0))
        .init_resource::<SourceMode>()
        .init_resource::<TransformZoom>()
        .init_resource::<MdCache>()
        .add_systems(Startup, setup)
        .add_systems(Update, (input, track_native_panel).chain())
        .add_systems(Update, shoot.after(track_native_panel))
        .add_systems(EguiPrimaryContextPass, egui_panel)
        .run();
}

#[derive(Resource, Default, Deref, DerefMut)]
struct MdCache(CommonMarkCache);

fn setup(mut commands: Commands) {
    commands.spawn(Camera2d);

    // The world-space rects the panels have to stay glued to.
    for (i, tint) in [Color::srgb(0.12, 0.13, 0.17), Color::srgb(0.10, 0.15, 0.13)]
        .into_iter()
        .enumerate()
    {
        let offset = i as f32 * 640.0;
        commands.spawn((
            Sprite::from_color(tint, NODE.size()),
            Transform::from_xyz(NODE.center().x + offset, NODE.center().y, -1.0),
        ));
    }

    commands.spawn((
        Node {
            position_type: PositionType::Absolute,
            top: px(8.0),
            left: px(8.0),
            ..default()
        },
        Text::default(),
        TextFont::from_font_size(14.0),
        Hud,
    ));

    commands.spawn((
        NativePanel,
        UiTransform::IDENTITY,
        Node {
            position_type: PositionType::Absolute,
            width: px(NODE.width()),
            height: px(NODE.height()),
            padding: UiRect::all(px(12.0)),
            flex_direction: FlexDirection::Column,
            row_gap: px(8.0),
            overflow: Overflow::clip(),
            ..default()
        },
    ));
}

fn input(
    keys: Res<ButtonInput<KeyCode>>,
    mut zoom: ResMut<Zoom>,
    mut mode: ResMut<SourceMode>,
    mut tz: ResMut<TransformZoom>,
    mut panel: Single<&mut UiTransform, With<NativePanel>>,
    mut cam: Single<&mut Projection, With<Camera2d>>,
    mut ui_scale: ResMut<UiScale>,
    mut hud: Single<&mut Text, With<Hud>>,
) {
    if keys.just_pressed(KeyCode::Minus) {
        zoom.0 = (zoom.0 / 1.25).max(0.25);
    }
    if keys.just_pressed(KeyCode::Equal) {
        zoom.0 = (zoom.0 * 1.25).min(4.0);
    }
    if keys.just_pressed(KeyCode::Space) {
        mode.0 = !mode.0;
    }
    if keys.just_pressed(KeyCode::KeyT) {
        tz.0 = !tz.0;
    }

    if let Projection::Orthographic(ortho) = &mut **cam {
        ortho.scale = 1.0 / zoom.0;
    }
    // bevy_ui has no world space. Two ways to make a screen-space node follow a
    // world-space rect's zoom:
    //   UiScale     — global, re-runs layout and re-rasterises glyphs (crisp)
    //   UiTransform — per node, scales the already-laid-out node (check blur)
    if tz.0 {
        ui_scale.0 = 1.0;
        panel.scale = Vec2::splat(zoom.0);
    } else {
        ui_scale.0 = zoom.0;
        panel.scale = Vec2::ONE;
    }

    hud.0 = format!(
        "zoom {:.2}   left: egui_commonmark   right: bevy_ui {} via {}\n\
         -/= zoom   space: {}   t: {}",
        zoom.0,
        if mode.0 { "EditableText" } else { "Text/TextSpan + Grid" },
        if tz.0 { "UiTransform" } else { "UiScale" },
        if mode.0 { "rendered" } else { "source" },
        if tz.0 { "UiScale" } else { "UiTransform" },
    );
}

/// Anchor the native panel to the second world rect. Positions are divided by
/// `UiScale` because the whole tree is already scaled by it.
fn track_native_panel(
    zoom: Res<Zoom>,
    mode: Res<SourceMode>,
    tz: Res<TransformZoom>,
    window: Single<&Window>,
    panel: Single<(Entity, &mut Node), With<NativePanel>>,
    mut commands: Commands,
    mut last: Local<Option<bool>>,
) {
    let (entity, mut node) = panel.into_inner();
    if tz.0 {
        // UiTransform scales about the node's centre, so anchor by centre.
        let c = NODE.center() + Vec2::new(640.0, 0.0);
        node.left = px(window.width() / 2.0 + c.x * zoom.0 - NODE.width() / 2.0);
        node.top = px(window.height() / 2.0 - c.y * zoom.0 - NODE.height() / 2.0);
    } else {
        let screen_x = window.width() / 2.0 + (NODE.min.x + 640.0) * zoom.0;
        let screen_y = window.height() / 2.0 - NODE.max.y * zoom.0;
        node.left = px(screen_x / zoom.0);
        node.top = px(screen_y / zoom.0);
    }

    if *last == Some(mode.0) {
        return;
    }
    *last = Some(mode.0);
    commands.entity(entity).despawn_related::<Children>();
    if mode.0 {
        commands.entity(entity).with_children(|p| {
            p.spawn((
                EditableText::new(MD),
                TextFont {
                    font: FontSource::Monospace,
                    font_smoothing: FontSmoothing::AntiAliased,
                    ..TextFont::from_font_size(13.0)
                },
                TextColor(Color::srgb(0.8, 0.85, 0.8)),
                LineHeight::RelativeToFont(1.4),
                Node {
                    width: percent(100.0),
                    height: percent(100.0),
                    ..default()
                },
            ));
        });
    } else {
        commands.entity(entity).with_children(|p| render_markdown(MD, p));
    }
}

fn egui_panel(
    mut contexts: EguiContexts,
    mut cache: ResMut<MdCache>,
    zoom: Res<Zoom>,
    window: Single<&Window>,
) -> Result {
    let ctx = contexts.ctx_mut()?;
    // The one thing path A hangs on: egui in screen space tracking a
    // world-space rect. Canvas zoom is uniform, so one scalar covers it.
    ctx.set_zoom_factor(zoom.0);
    let screen = egui::pos2(
        (window.width() / 2.0 + NODE.min.x * zoom.0) / zoom.0,
        (window.height() / 2.0 - NODE.max.y * zoom.0) / zoom.0,
    );
    egui::Area::new("node".into())
        .fixed_pos(screen)
        .show(ctx, |ui| {
            ui.set_width(NODE.width());
            ui.set_max_height(NODE.height());
            egui::ScrollArea::vertical().show(ui, |ui| {
                CommonMarkViewer::new().show(ui, &mut cache, MD);
            });
        });
    Ok(())
}

// ---------------------------------------------------------------------------
// path C: markdown subset -> bevy_ui. Ugly on purpose.
// ---------------------------------------------------------------------------

fn font(size: f32, bold: bool, mono: bool) -> TextFont {
    let mut f = TextFont::from_font_size(size);
    if bold {
        f.weight = FontWeight::BOLD;
    }
    if mono {
        f.font = FontSource::Monospace;
    }
    f
}

const FG: Color = Color::srgb(0.88, 0.9, 0.93);
const DIM: Color = Color::srgb(0.6, 0.64, 0.7);
const LINK: Color = Color::srgb(0.44, 0.62, 1.0);

fn render_markdown(md: &str, parent: &mut ChildSpawnerCommands) {
    let mut spans: Vec<(String, TextFont, Color)> = Vec::new();
    let mut bold = 0usize;
    let mut italic = 0usize;
    let mut size = 15.0f32;
    let mut mono = false;
    let mut link = false;
    // table state
    let mut cells: Vec<(String, bool)> = Vec::new();
    let mut cols = 0usize;
    let mut in_head = false;
    let mut in_table = false;

    let flush = |parent: &mut ChildSpawnerCommands,
                 spans: &mut Vec<(String, TextFont, Color)>| {
        if spans.is_empty() {
            return;
        }
        let (text, f, c) = spans.remove(0);
        parent
            .spawn((Text::new(text), f, TextColor(c)))
            .with_children(|p| {
                for (text, f, c) in spans.drain(..) {
                    p.spawn((TextSpan::new(text), f, TextColor(c)));
                }
            });
    };

    for ev in Parser::new_ext(md, pulldown_cmark::Options::ENABLE_TABLES) {
        match ev {
            Event::Start(Tag::Heading { level, .. }) => {
                size = match level {
                    HeadingLevel::H1 => 26.0,
                    HeadingLevel::H2 => 21.0,
                    _ => 17.0,
                };
                bold += 1;
            }
            Event::End(TagEnd::Heading(_)) => {
                flush(parent, &mut spans);
                size = 15.0;
                bold -= 1;
            }
            Event::Start(Tag::Emphasis) => italic += 1,
            Event::End(TagEnd::Emphasis) => italic -= 1,
            Event::Start(Tag::Strong) => bold += 1,
            Event::End(TagEnd::Strong) => bold -= 1,
            Event::Start(Tag::Link { .. }) => link = true,
            Event::End(TagEnd::Link) => link = false,
            Event::Start(Tag::Item) => spans.push(("• ".into(), font(size, false, false), DIM)),
            Event::End(TagEnd::Item) => flush(parent, &mut spans),
            Event::Start(Tag::CodeBlock(_)) => mono = true,
            Event::End(TagEnd::CodeBlock) => {
                let code: String = spans.iter().map(|(t, ..)| t.as_str()).collect();
                spans.clear();
                mono = false;
                parent
                    .spawn((
                        Node {
                            padding: UiRect::all(px(8.0)),
                            // 0.19: BorderRadius lives on Node now.
                            border_radius: BorderRadius::all(px(4.0)),
                            ..default()
                        },
                        BackgroundColor(Color::srgb(0.06, 0.07, 0.09)),
                    ))
                    .with_child((
                        Text::new(code.trim_end().to_string()),
                        font(13.0, false, true),
                        TextColor(Color::srgb(0.7, 0.85, 0.72)),
                    ));
            }
            Event::Start(Tag::Table(aligns)) => {
                in_table = true;
                cols = aligns.len();
            }
            Event::Start(Tag::TableHead) => in_head = true,
            Event::End(TagEnd::TableHead) => in_head = false,
            Event::End(TagEnd::TableCell) => {
                let text: String = spans.drain(..).map(|(t, ..)| t).collect();
                cells.push((text, in_head));
            }
            Event::End(TagEnd::Table) => {
                in_table = false;
                parent
                    .spawn(Node {
                        display: Display::Grid,
                        grid_template_columns: RepeatedGridTrack::auto(cols as u16),
                        ..default()
                    })
                    .with_children(|p| {
                        for (text, head) in cells.drain(..) {
                            p.spawn((
                                Node {
                                    padding: UiRect::axes(px(8.0), px(4.0)),
                                    border: UiRect::all(px(1.0)),
                                    ..default()
                                },
                                BorderColor::all(Color::srgb(0.25, 0.27, 0.32)),
                            ))
                            .with_child((
                                Text::new(text),
                                font(13.0, head, false),
                                TextColor(if head { FG } else { DIM }),
                            ));
                        }
                    });
            }
            Event::Text(t) => {
                let code = mono;
                spans.push((
                    t.to_string(),
                    font(if code { 13.0 } else { size }, bold > 0, code),
                    if link {
                        LINK
                    } else if code {
                        Color::srgb(0.7, 0.85, 0.72)
                    } else if italic > 0 {
                        DIM
                    } else {
                        FG
                    },
                ));
            }
            Event::Code(t) => spans.push((
                t.to_string(),
                font(13.0, bold > 0, true),
                Color::srgb(0.7, 0.85, 0.72),
            )),
            Event::End(TagEnd::Paragraph) => {
                if !in_table {
                    flush(parent, &mut spans)
                }
            }
            Event::SoftBreak => spans.push((" ".into(), font(size, false, false), FG)),
            _ => {}
        }
    }
    flush(parent, &mut spans);
}

/// `SPIKE_SHOT=<dir> cargo run --example text_spike` walks the four states,
/// screenshots each and exits. Beats driving the window from AppleScript.
fn shoot(
    mut frame: Local<u32>,
    mut zoom: ResMut<Zoom>,
    mut tz: ResMut<TransformZoom>,
    mut mode: ResMut<SourceMode>,
    mut commands: Commands,
    mut exit: MessageWriter<AppExit>,
) {
    let Ok(dir) = std::env::var("SPIKE_SHOT") else {
        return;
    };
    *frame += 1;
    let mut shot = |name: &str| {
        commands
            .spawn(Screenshot::primary_window())
            .observe(save_to_disk(format!("{dir}/{name}.png")));
    };
    match *frame {
        90 => shot("1_zoom1"),
        100 => zoom.0 = 4.0,
        150 => shot("2_zoom4_uiscale"),
        160 => tz.0 = true,
        210 => shot("3_zoom4_uitransform"),
        220 => {
            tz.0 = false;
            zoom.0 = 1.0;
            mode.0 = true;
        }
        270 => shot("4_editable_source"),
        320 => {
            exit.write(AppExit::Success);
        }
        _ => {}
    }
}
