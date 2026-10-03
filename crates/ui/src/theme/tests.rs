use super::*;
use extboard_core::{ACCENT, BACKGROUND, Canvas, PRESETS};

fn canvas() -> Canvas {
    serde_json::from_str(r#"{"nodes":[],"edges":[]}"#).expect("fixture")
}

// `AssetPlugin` because the panel's shader is an embedded asset, `Font` because
// a loaded font is an asset too, and the task pool because loading one reads a
// file. `DefaultPlugins` brings all three for real.
fn app() -> App {
    rooted(AssetPlugin::default())
}

// A real asset root, for the one test that loads a real font off the disk.
fn app_rooted(dir: &std::path::Path) -> App {
    rooted(AssetPlugin {
        file_path: dir.to_string_lossy().into_owned(),
        meta_check: bevy::asset::AssetMetaCheck::Never,
        ..default()
    })
}

fn rooted(assets: AssetPlugin) -> App {
    let mut app = App::new();
    app.add_plugins((TaskPoolPlugin::default(), assets, ThemePlugin))
        .init_asset::<bevy::text::Font>()
        // `TextPlugin`'s, both: without the loader a `.ttf` never arrives, and
        // without `FontCx` nothing can name it.
        .register_asset_loader(bevy::text::FontLoader)
        .init_resource::<bevy::text::FontCx>()
        .init_resource::<InputFocus>()
        .init_resource::<ClearColor>()
        .init_resource::<ButtonInput<KeyCode>>()
        .insert_resource(Document(canvas()));
    app
}

#[test]
fn hex_is_the_one_form_a_slot_is_written_in() {
    assert_eq!(hex("#1a2b3c"), Some(Color::srgb_u8(0x1a, 0x2b, 0x3c)));
    assert_eq!(hex("#ffffff"), Some(Color::srgb_u8(255, 255, 255)));
    // Case is not part of it.
    assert_eq!(hex("#FFFFFF"), hex("#ffffff"));
    for junk in ["#abc", "#nothex", "1a2b3c", "4", "", "#1a2b3c7"] {
        assert_eq!(hex(junk), None, "{junk}");
    }
}

// Nothing may ask for a `Document` before the first load.
#[test]
fn the_frames_before_the_first_load_are_quiet() {
    let mut app = App::new();
    app.add_plugins((AssetPlugin::default(), ThemePlugin));
    app.world_mut().run_schedule(Update);
}

#[test]
fn the_background_follows_the_spaces_theme() {
    let mut app = app();
    app.world_mut().run_schedule(Update);
    let background = |app: &App| app.world().resource::<ClearColor>().0;
    assert_eq!(
        background(&app),
        hex(&Block::preset("").colors[BACKGROUND]).expect("a preset colour")
    );

    app.world_mut()
        .resource_mut::<Document>()
        .0
        .set_theme(&Block::preset("dracula-dark"));
    app.world_mut().run_schedule(Update);
    assert_eq!(
        background(&app),
        hex(&Block::preset("dracula-dark").colors[BACKGROUND]).expect("a preset colour")
    );
}

// Ctrl-T and nothing else: a bare `t` is a character somewhere.
#[test]
fn control_t_opens_the_panel_and_closes_it_again() {
    let mut app = app();
    let mut press = |keys: &[KeyCode]| {
        let mut input = ButtonInput::default();
        for key in keys {
            input.press(*key);
        }
        app.world_mut().insert_resource(input);
        app.world_mut().run_schedule(Update);
        app.world().resource::<Open>().0
    };
    assert!(!press(&[KeyCode::KeyT]), "a bare t is not the panel");
    assert!(press(&[KeyCode::ControlLeft, KeyCode::KeyT]));
    assert!(!press(&[KeyCode::ControlLeft, KeyCode::KeyT]));
}

// Both panels live on the right edge, and the script sidebar is the full height
// of it: opening one is what puts the other away.
#[test]
fn opening_this_panel_closes_the_script_sidebar() {
    let mut app = app();
    app.world_mut().resource_mut::<Sidebar>().open = true;
    let mut input = ButtonInput::default();
    input.press(KeyCode::ControlLeft);
    input.press(KeyCode::KeyT);
    app.world_mut().insert_resource(input);
    app.world_mut().run_schedule(Update);

    assert!(app.world().resource::<Open>().0, "the panel is up");
    assert!(!app.world().resource::<Sidebar>().open, "and it is alone");
}

fn opened() -> App {
    let mut app = app();
    app.world_mut().resource_mut::<Open>().0 = true;
    // Three: the panel is spawned by `Commands`, so its fields exist on the
    // frame after, and `typed` sees them as changed on the one after that.
    for _ in 0..3 {
        app.world_mut().run_schedule(Update);
    }
    app
}

fn field(app: &mut App, which: Field) -> Entity {
    app.world_mut()
        .query::<(Entity, &Field)>()
        .iter(app.world())
        .find(|(_, field)| **field == which)
        .map(|(entity, _)| entity)
        .expect("a field for every slot")
}

// Reading the theme is not editing it: a space that never carried a `theme` key
// still does not, however long the panel sits open.
#[test]
fn opening_the_panel_does_not_write_the_space() {
    let app = opened();
    assert_eq!(
        app.world().resource::<Document>().0.stored_theme(),
        Block::default()
    );
}

// Editing one slot of a preset writes the whole palette: a preset is where a
// theme of your own starts, not a mode to live in.
#[test]
fn typing_a_colour_writes_the_palette_as_the_spaces_own() {
    let mut app = opened();
    let entity = field(&mut app, Field::Color(ACCENT));
    app.world_mut()
        .insert_resource(InputFocus::from_entity(entity));
    set_text(
        &mut app
            .world_mut()
            .get_mut::<EditableText>(entity)
            .expect("the field's editor"),
        "#abcdef",
    );
    app.world_mut().run_schedule(Update);

    let stored = app.world().resource::<Document>().0.stored_theme();
    assert_eq!(stored.name, Block::preset("").name);
    assert_eq!(stored.colors[ACCENT], "#abcdef");
    assert_eq!(stored.colors.len(), Block::preset("").colors.len());
    // And the rest of the preset came with it, rather than being lost.
    assert_eq!(
        stored.colors[BACKGROUND],
        Block::preset("").colors[BACKGROUND]
    );
}

// Never below the four named roles, and never past twenty.
#[test]
fn the_palette_grows_to_the_cap_and_shrinks_to_the_four_roles() {
    let mut theme = Block::preset("");
    while let Some(grown) = resized(&theme, 1) {
        theme = grown;
    }
    assert_eq!(theme.colors.len(), MAX_COLORS);
    // The slot a `+` adds is visible, or it cannot be found to be edited.
    assert_eq!(
        theme.colors[MAX_COLORS - 1],
        *Block::preset("").colors.last().expect("a preset colour")
    );

    while let Some(shrunk) = resized(&theme, -1) {
        theme = shrunk;
    }
    assert_eq!(theme.colors.len(), MIN_COLORS);
    assert_eq!(theme.colors, Block::preset("").colors[..MIN_COLORS]);
}

// Whatever is typed, the box holds a colour.
#[test]
fn a_colour_box_is_a_hash_and_six_hex_digits() {
    assert_eq!(clamped("#1a2b3c"), "#1a2b3c");
    assert_eq!(clamped(""), "#");
    assert_eq!(clamped("#"), "#");
    assert_eq!(clamped("1a2b3c"), "#1a2b3c");
    assert_eq!(clamped("#1a2b3cd"), "#1a2b3c");
    assert_eq!(clamped("##1a2b3c99"), "#1a2b3c");
    assert_eq!(clamped("rebeccapurple"), "#ebecca");
    assert_eq!(clamped("hello"), "#e");
    // Whatever comes out is either a colour or on its way to being one.
    for typed in ["#1a2b3c", "", "zzz", "#1a2b3cd", "0"] {
        let code = clamped(typed);
        assert!(
            code.len() <= 7 && code.starts_with('#'),
            "{typed} -> {code}"
        );
        assert_eq!(clamped(&code), code, "clamping twice has to settle");
    }
}

#[test]
fn the_picker_shader_compiles() {
    crate::wgsl::compiles(include_str!("picker.wgsl"));
}

// What the picker writes has to be what the box would accept, or dragging a
// dial produces a colour the panel then refuses to show.
#[test]
fn a_dial_writes_a_colour_the_box_would_take() {
    for hue in [0.0, 0.33, 0.99] {
        for (saturation, value) in [(0.0, 0.0), (1.0, 1.0), (0.4, 0.7), (0.0, 1.0)] {
            let written = code(Color::from(Hsva::new(hue * 360.0, saturation, value, 1.0)));
            assert_eq!(clamped(&written), written, "{hue} {saturation} {value}");
            assert!(hex(&written).is_some(), "{written}");
        }
    }
}

// A round trip through a code lands on the same square. Black and grey are the
// exception, and the reason `Picking` carries the hue itself.
#[test]
fn a_colour_round_trips_through_the_plane_except_for_its_hue() {
    for original in [
        Hsva::new(210.0, 0.6, 0.8, 1.0),
        Hsva::new(40.0, 1.0, 1.0, 1.0),
        Hsva::new(0.0, 0.0, 0.5, 1.0),
    ] {
        let back = Hsva::from(hex(&code(Color::from(original))).expect("a colour"));
        assert!(
            (back.saturation - original.saturation).abs() < 0.01,
            "{back:?}"
        );
        assert!((back.value - original.value).abs() < 0.01, "{back:?}");
    }
    // Grey has no hue to come back with: this is what the resource holds.
    assert_eq!(
        Hsva::from(hex("#808080").expect("a colour")).saturation,
        0.0
    );
}

// Two frames: `resolve` runs ahead of every write, and the second has the keys
// released or `just_pressed` fires the shortcut twice.
fn press(app: &mut App, keys: &[KeyCode]) {
    let mut input = ButtonInput::default();
    for key in keys {
        input.press(*key);
    }
    app.world_mut().insert_resource(input);
    app.world_mut().run_schedule(Update);
    app.world_mut()
        .insert_resource(ButtonInput::<KeyCode>::default());
    app.world_mut().run_schedule(Update);
}

fn edit(app: &mut App, slot: usize, code: &str) {
    let entity = field(app, Field::Color(slot));
    app.world_mut()
        .insert_resource(InputFocus::from_entity(entity));
    set_text(
        &mut app
            .world_mut()
            .get_mut::<EditableText>(entity)
            .expect("the field's editor"),
        code,
    );
    app.world_mut().run_schedule(Update);
    app.world_mut().run_schedule(Update);
}

fn themes(app: &App) -> Vec<Block> {
    app.world().resource::<Document>().0.themes()
}

fn answer(app: &mut App, name: &str) {
    let entity = field(app, Field::New);
    set_text(
        &mut app
            .world_mut()
            .get_mut::<EditableText>(entity)
            .expect("the naming box"),
        name,
    );
    press(app, &[KeyCode::Enter]);
}

fn asking(app: &App) -> bool {
    app.world().resource::<Naming>().0.is_some()
}

// An edit to a shipped palette is a copy, and a copy needs a name first.
#[test]
fn a_preset_is_copied_rather_than_changed_and_the_copy_is_named_first() {
    let mut app = opened();
    assert!(
        !app.world().resource::<Theme>().dirty(),
        "a preset is saved"
    );

    edit(&mut app, ACCENT, "#abcdef");
    assert!(app.world().resource::<Theme>().dirty());
    assert!(themes(&app).is_empty(), "an edit saves nothing by itself");

    // The save asks: the window goes up and nothing is written yet.
    press(&mut app, &[KeyCode::ControlLeft, KeyCode::KeyS]);
    assert!(asking(&app), "the window has to be up");
    assert!(themes(&app).is_empty(), "the question is not the save");
    // And it offers a free name rather than an empty box.
    assert_eq!(
        app.world().resource::<Naming>().0.as_deref(),
        Some("tokyo-night-dark 2")
    );

    answer(&mut app, "studio");
    assert!(!asking(&app), "answered, so the window goes away");
    let saved = themes(&app);
    assert_eq!(saved.len(), 1);
    assert_eq!(saved[0].name, "studio");
    assert_eq!(saved[0].colors[ACCENT], "#abcdef");
    assert!(!app.world().resource::<Theme>().dirty(), "and it is clean");
    assert_eq!(
        Block::preset("tokyo-night-dark").colors[ACCENT],
        PRESETS[0].1[ACCENT]
    );
}

// Escape is the answer that creates nothing -- and it may not throw the edit
// away either, because the board is still drawing it.
#[test]
fn escape_cancels_the_naming_and_keeps_the_edit() {
    let mut app = opened();
    edit(&mut app, ACCENT, "#abcdef");
    press(&mut app, &[KeyCode::ControlLeft, KeyCode::KeyS]);
    press(&mut app, &[KeyCode::Escape]);

    assert!(!asking(&app));
    assert!(themes(&app).is_empty(), "escape saves nothing");
    let theme = app.world().resource::<Theme>();
    assert_eq!(
        theme.live.colors[ACCENT], "#abcdef",
        "the edit is still live"
    );
    assert!(theme.dirty(), "and still uncommitted");
}

// A name nothing can select again is refused, and the window stays up.
#[test]
fn a_blank_name_is_not_an_answer() {
    let mut app = opened();
    edit(&mut app, ACCENT, "#abcdef");
    press(&mut app, &[KeyCode::ControlLeft, KeyCode::KeyS]);

    answer(&mut app, "   ");
    assert!(asking(&app), "still asking");
    assert!(themes(&app).is_empty());
    // And the box still has the caret, or there is no way to answer again.
    let box_entity = field(&mut app, Field::New);
    assert_eq!(app.world().resource::<InputFocus>().get(), Some(box_entity));

    // A preset's name is taken, so that is not an answer either.
    answer(&mut app, "dracula-dark");
    assert!(asking(&app));
    assert!(themes(&app).is_empty());

    answer(&mut app, "studio");
    assert!(!asking(&app));
    assert_eq!(themes(&app).len(), 1);
}

// Saving a theme of your own writes straight back into it.
#[test]
fn a_theme_of_your_own_saves_in_place() {
    let mut app = opened();
    edit(&mut app, ACCENT, "#abcdef");
    press(&mut app, &[KeyCode::ControlLeft, KeyCode::KeyS]);
    answer(&mut app, "studio");

    edit(&mut app, BACKGROUND, "#010203");
    assert!(app.world().resource::<Theme>().dirty());
    press(&mut app, &[KeyCode::ControlLeft, KeyCode::KeyS]);

    assert!(!asking(&app), "a theme of your own has nothing to ask");
    let saved = themes(&app);
    assert_eq!(saved.len(), 1, "one theme, not a copy of a copy");
    assert_eq!(saved[0].name, "studio");
    assert_eq!(saved[0].colors[BACKGROUND], "#010203");
}

// Nothing to keep is nothing to do: a save on a clean theme may not fork it.
#[test]
fn saving_a_clean_theme_does_nothing() {
    let mut app = opened();
    press(&mut app, &[KeyCode::ControlLeft, KeyCode::KeyS]);
    assert!(!asking(&app), "nothing to keep is nothing to name");
    assert!(themes(&app).is_empty());
    assert_eq!(
        app.world().resource::<Theme>().live.name,
        Block::preset("").name
    );
}

// The buttons say what they would do, and go dark when they would do nothing.
#[test]
fn every_button_reads_what_it_would_do() {
    let mut app = opened();
    let read = |app: &App, which| {
        let theme = app.world().resource::<Theme>();
        legend(theme, which)
    };
    // Blocked until there is something to keep, and amber once there is.
    assert_eq!(read(&app, Button::Save), ("save", LABEL));
    assert_eq!(read(&app, Button::Clone).1, LABEL, "a preset is not cloned");

    edit(&mut app, ACCENT, "#abcdef");
    assert_eq!(read(&app, Button::Save), ("save", UNSAVED));

    press(&mut app, &[KeyCode::ControlLeft, KeyCode::KeyS]);
    answer(&mut app, "studio");
    assert_eq!(read(&app, Button::Save), ("save", LABEL));
    assert_eq!(read(&app, Button::Clone).1, FG, "and now it is yours");

    // The palette's own two ends, which `resized` is what refuses.
    while app.world().resource::<Theme>().live.colors.len() > MIN_COLORS {
        let block = resized(&app.world().resource::<Theme>().live, -1).expect("a slot to drop");
        app.world_mut()
            .resource_mut::<Document>()
            .0
            .set_theme(&block);
        app.world_mut().run_schedule(Update);
    }
    assert_eq!(read(&app, Button::Drop).1, LABEL);
    assert_eq!(read(&app, Button::Add).1, FG);
}

// A row has 236px and a library holds paths.
#[test]
fn a_font_row_shows_the_file_and_not_the_folder_it_is_in() {
    assert_eq!(font_label("fonts/Inter.ttf"), "Inter");
    assert_eq!(font_label("fonts/Fira Code.otf"), "Fira Code");
    // A family name has neither a folder nor an extension.
    assert_eq!(font_label("Menlo"), "Menlo");
    assert_eq!(font_label(DEFAULT_FONT), DEFAULT_FONT);
    assert_eq!(font_label(""), "");
}

// A file is held as an asset and spent as a family; a name is the system's.
#[test]
fn a_font_file_is_held_as_an_asset_and_anything_else_is_a_family() {
    let mut app = app();
    let assets = app.world_mut().resource::<AssetServer>().clone();

    let file = load("fonts/Inter.ttf", &assets);
    assert!(file.file.is_some(), "the asset is held, or it unloads");
    assert!(file.pending(), "and it has no family until it lands");
    // Never the handle: a handle for an asset still on its way draws nothing.
    assert_eq!(file.source, FontSource::default());

    let family = load("Menlo", &assets);
    assert!(family.file.is_none());
    assert!(!family.pending());
    assert_eq!(family.source, FontSource::from("Menlo"));

    for embedded in ["", DEFAULT_FONT] {
        assert_eq!(load(embedded, &assets).source, FontSource::default());
    }
}

// The bug behind E8's fonts: a `FontSource::Handle` is resolved through the
// asset's alias, and the dev build's file watcher fires for the `.ttf` a drop
// has just written -- replacing the asset with one whose alias is empty, which
// bevy never fills in again. The family name is what survives that.
#[test]
fn a_font_file_is_spent_as_the_family_it_registers() {
    // The embedded font's bytes under a name of our own: a family lives in the
    // file and not in the file name, which is the whole point of the fix.
    let dir = std::env::temp_dir().join("extboard-font-test");
    std::fs::create_dir_all(dir.join("fonts")).unwrap();
    std::fs::write(dir.join("fonts/dropped.ttf"), bevy::text::DEFAULT_FONT_DATA).unwrap();

    let mut app = app_rooted(&dir);
    let mut fonts = Fonts::default();
    fonts.add("fonts/dropped.ttf");
    fonts.set(PRIMARY_TEXT, "fonts/dropped.ttf");
    app.world_mut()
        .resource_mut::<Document>()
        .0
        .set_fonts(&fonts);

    // The file is read off a task pool thread, so this is a wait, not a frame.
    for _ in 0..200 {
        app.update();
        if !app.world().resource::<Theme>().pending() {
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(5));
    }
    assert_eq!(
        app.world().resource::<Theme>().text_font(PRIMARY_TEXT),
        FontSource::from("Fira Mono"),
        "the family the file carries, not the asset it arrived as"
    );
    // That the weight reaches the classifier at all, off a real file. This
    // fixture is bevy's *subset* FiraMono, light enough to read as a pixel font.
    assert_eq!(
        app.world().resource::<Theme>().smoothing(),
        FontSmoothing::None
    );

    std::fs::remove_dir_all(&dir).unwrap();
}

// Every text on the board follows the primary one, and that one is the
// embedded font until a space says otherwise.
#[test]
fn the_primary_text_is_the_embedded_font_until_a_font_is_loaded() {
    let mut app = app();
    app.world_mut().run_schedule(Update);
    let font = |app: &App| app.world().resource::<Theme>().text_font(PRIMARY_TEXT);
    assert_eq!(font(&app), FontSource::default());

    let mut fonts = Fonts::default();
    fonts.add("fonts/Inter.ttf");
    fonts.set(PRIMARY_TEXT, "fonts/Inter.ttf");
    app.world_mut()
        .resource_mut::<Document>()
        .0
        .set_fonts(&fonts);
    app.world_mut().run_schedule(Update);
    assert!(matches!(font(&app), FontSource::Handle(_)));
    // And the two nothing reads yet stay unset.
    assert_eq!(
        app.world().resource::<Theme>().text_font(1),
        FontSource::default()
    );
}

// Three rows, never more and never fewer: a text cannot be added or removed.
#[test]
fn the_panel_shows_three_texts_and_one_list_at_a_time() {
    let mut app = opened();
    let count = |app: &mut App| {
        (
            app.world_mut().query::<&TextRow>().iter(app.world()).len(),
            app.world_mut()
                .query::<&Candidate>()
                .iter(app.world())
                .len(),
        )
    };
    assert_eq!(count(&mut app), (FONT_ROLES.len(), 0));

    // The secondary text: the embedded font, and `none`.
    app.world_mut().resource_mut::<Targeting>().0 = Some(1);
    app.world_mut().run_schedule(Update);
    assert_eq!(count(&mut app), (FONT_ROLES.len(), 2));

    // The primary text has no `none`, because every node's text follows it.
    app.world_mut().resource_mut::<Targeting>().0 = Some(PRIMARY_TEXT);
    app.world_mut().run_schedule(Update);
    assert_eq!(count(&mut app), (FONT_ROLES.len(), 1));
}

// A pixel font needs no grey edge and a nearest sampler on the atlas behind it,
// which is what `FontSmoothing::None` carries -- and the file's weight is what
// says the primary text is one. The threshold is the only interesting line.
#[test]
fn a_light_enough_font_file_is_a_pixel_font_and_stops_the_smoothing() {
    let mut theme = Theme::default();
    assert_eq!(theme.smoothing(), FontSmoothing::AntiAliased);

    theme.fonts.add("fonts/tiny.ttf");
    theme.fonts.set(PRIMARY_TEXT, "fonts/tiny.ttf");
    theme.loaded = vec![Loaded {
        name: "fonts/tiny.ttf".to_owned(),
        bytes: None,
        file: None,
        source: FontSource::from("Tiny"),
    }];
    // Still on its way: nothing to weigh, so nothing to classify.
    assert_eq!(theme.smoothing(), FontSmoothing::AntiAliased);

    theme.loaded[0].bytes = Some(PIXEL_BYTES - 1);
    assert_eq!(theme.smoothing(), FontSmoothing::None);
    theme.loaded[0].bytes = Some(PIXEL_BYTES);
    assert_eq!(theme.smoothing(), FontSmoothing::AntiAliased);
}
