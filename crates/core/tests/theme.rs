use extboard_core::{BACKGROUND, Canvas, MAX_COLORS, MIN_COLORS, PRESETS, Theme};

fn canvas(json: &str) -> Canvas {
    serde_json::from_str(json).expect("fixture")
}

#[test]
fn a_space_with_no_theme_draws_with_the_first_preset() {
    let canvas = canvas(r#"{"nodes":[],"edges":[]}"#);
    assert_eq!(canvas.stored_theme(), Theme::default());
    assert_eq!(canvas.theme(), Theme::preset(PRESETS[0].0));
    assert_eq!(canvas.theme().color(BACKGROUND), Some(PRESETS[0].1[0]));
}

#[test]
fn every_preset_carries_the_four_named_roles() {
    for (name, colors) in PRESETS {
        assert!(colors.len() >= MIN_COLORS, "{name}");
        assert_eq!(Theme::preset(name).colors, colors, "{name}");
        for hex in colors {
            assert!(extboard_core::is_canvas_color(hex), "{name}: {hex}");
        }
    }
}

// The names the Rhai `set_theme` has always taken have to keep meaning a theme.
#[test]
fn dark_and_light_are_the_tokyo_night_pair() {
    assert_eq!(Theme::preset("dark").name, "tokyo-night-dark");
    assert_eq!(Theme::preset("light").name, "tokyo-night-light");
    assert_eq!(Theme::preset("nonesuch").colors, PRESETS[0].1);
}

// What would be `undefined` without `Option`: slot 9 of a four-colour theme.
#[test]
fn a_theme_shorter_than_the_space_wraps_by_index() {
    let theme = Theme {
        name: "mine".to_owned(),
        colors: ["#000000", "#111111", "#222222", "#333333"]
            .map(str::to_owned)
            .to_vec(),
    };
    assert_eq!(theme.color(0), Some("#000000"));
    assert_eq!(theme.color(3), Some("#333333"));
    assert_eq!(theme.color(4), Some("#000000"));
    assert_eq!(theme.color(9), Some("#111111"));
    assert_eq!(theme.color(usize::MAX), theme.color(usize::MAX % 4));
    // No colours at all is the one case that cannot wrap.
    assert_eq!(Theme::default().color(0), None);
}

#[test]
fn the_roles_run_background_primary_secondary_accent_text_then_extra_accents() {
    assert_eq!(Theme::role(0), "background");
    assert_eq!(Theme::role(1), "primary");
    assert_eq!(Theme::role(2), "secondary");
    assert_eq!(Theme::role(3), "accent");
    assert_eq!(Theme::role(4), "text");
    assert_eq!(Theme::role(5), "accent-b");
    assert_eq!(Theme::role(6), "accent-c");
    assert_eq!(Theme::role(MAX_COLORS - 1), "accent-p");
}

#[test]
fn a_theme_write_keeps_the_rest_of_the_block_and_the_cap() {
    let mut canvas = canvas(r#"{"theme":{"author":"me"},"nodes":[],"edges":[]}"#);
    let long = Theme {
        name: "mine".to_owned(),
        colors: (0..MAX_COLORS + 5).map(|i| format!("#{i:06x}")).collect(),
    };
    canvas.set_theme(&long);

    let theme = canvas.stored_theme();
    assert_eq!(theme.name, "mine");
    assert_eq!(theme.colors.len(), MAX_COLORS, "the cap holds on write");
    assert_eq!(
        canvas.extra["theme"]["author"],
        serde_json::json!("me"),
        "a key that is not ours survives the write"
    );

    // An empty palette means "follow the preset", which is the key going away.
    canvas.set_theme(&Theme {
        name: "dracula-dark".to_owned(),
        colors: Vec::new(),
    });
    assert_eq!(canvas.stored_theme().colors, Vec::<String>::new());
    assert_eq!(canvas.theme(), Theme::preset("dracula-dark"));
}

// A hand-written file can say anything; none of it may panic on the way in.
#[test]
fn a_theme_block_that_is_not_one_reads_as_no_theme() {
    for json in [
        r#"{"theme":"dark","nodes":[],"edges":[]}"#,
        r#"{"theme":{"colors":[1,2]},"nodes":[],"edges":[]}"#,
        r#"{"theme":[],"nodes":[],"edges":[]}"#,
    ] {
        let mut canvas = canvas(json);
        assert_eq!(canvas.stored_theme(), Theme::default(), "{json}");
        assert_eq!(canvas.theme(), Theme::preset(""), "{json}");
        // And it is still writable: the block is replaced, not merged into.
        canvas.set_theme(&Theme::preset("dracula-dark"));
        assert_eq!(canvas.theme(), Theme::preset("dracula-dark"), "{json}");
    }
}

#[test]
fn a_saved_theme_lives_beside_the_scripts_and_comes_back_by_name() {
    let mut canvas = canvas(r#"{"extboard":{"scripts":{"main":"// hi"}},"nodes":[],"edges":[]}"#);
    let mine = Theme {
        name: "studio".to_owned(),
        colors: ["#111111", "#222222", "#333333", "#444444"]
            .map(str::to_owned)
            .to_vec(),
    };
    assert!(canvas.save_theme(&mine));

    assert_eq!(canvas.themes(), vec![mine.clone()]);
    assert_eq!(canvas.saved_theme("studio"), Some(mine));
    assert_eq!(canvas.saved_theme("nonesuch"), None);
    // The scripts under the same key are not what a theme write replaces.
    assert_eq!(canvas.extra["extboard"]["scripts"]["main"], "// hi");
}

// The shipped palettes are read-only: that is what makes an edit to one a copy.
#[test]
fn a_preset_can_be_read_by_name_and_never_written_over() {
    assert_eq!(
        Theme::preset_named("dracula-dark")
            .map(|t| t.name)
            .as_deref(),
        Some("dracula-dark")
    );
    assert_eq!(Theme::preset_named("studio"), None);
    // The old script names still resolve, and still to a preset.
    assert_eq!(
        Theme::preset_named("dark").map(|t| t.name).as_deref(),
        Some("tokyo-night-dark")
    );

    let mut canvas = canvas(r#"{"nodes":[],"edges":[]}"#);
    for name in ["dracula-dark", "dark", ""] {
        assert!(
            !canvas.save_theme(&Theme {
                name: name.to_owned(),
                colors: vec!["#111111".to_owned()],
            }),
            "{name}"
        );
    }
    assert!(!canvas.save_theme(&Theme {
        name: "empty".to_owned(),
        colors: Vec::new(),
    }));
    assert!(canvas.themes().is_empty());
}

#[test]
fn a_fresh_name_steps_past_every_name_already_taken() {
    let mut canvas = canvas(r#"{"nodes":[],"edges":[]}"#);
    assert_eq!(canvas.fresh_theme_name("dracula-dark"), "dracula-dark 2");
    assert_eq!(canvas.fresh_theme_name(""), "theme 2");

    for expected in ["dracula-dark 2", "dracula-dark 3", "dracula-dark 4"] {
        let name = canvas.fresh_theme_name("dracula-dark");
        assert_eq!(name, expected);
        assert!(canvas.save_theme(&Theme {
            name,
            colors: vec!["#111111".to_owned()],
        }));
    }
}
