use super::*;

// Nothing may run or ask for a resource before the first document lands.
#[test]
fn the_frames_before_the_first_load_are_quiet() {
    let mut app = App::new();
    app.add_plugins(MenuPlugin);
    app.world_mut().run_schedule(Update);
}

// The bug this closes: `swallow` consumed the very press `dismiss` was
// reading, so the menu never went away at all. Clicking a row and clicking
// away are one gesture to it, and both have to end with the menu gone.
#[test]
fn a_press_closes_the_menu_without_reaching_the_canvas() {
    let mut app = App::new();
    app.add_plugins(MenuPlugin)
        .init_resource::<ButtonInput<KeyCode>>()
        .init_resource::<ButtonInput<MouseButton>>();
    let menu = app.world_mut().spawn(ContextMenu).id();

    app.world_mut()
        .resource_mut::<ButtonInput<MouseButton>>()
        .press(MouseButton::Left);
    app.world_mut().run_schedule(PreUpdate);

    // Every canvas system reads this after `swallow` and must miss the press.
    assert!(
        !app.world()
            .resource::<ButtonInput<MouseButton>>()
            .just_pressed(MouseButton::Left)
    );

    app.world_mut().run_schedule(Update);
    assert!(app.world().get_entity(menu).is_err(), "the menu is gone");
}

// And a frame with no menu up leaves the press for the canvas to act on.
#[test]
fn a_press_with_no_menu_is_left_alone() {
    let mut app = App::new();
    app.add_plugins(MenuPlugin)
        .init_resource::<ButtonInput<KeyCode>>()
        .init_resource::<ButtonInput<MouseButton>>();
    app.world_mut()
        .resource_mut::<ButtonInput<MouseButton>>()
        .press(MouseButton::Left);
    app.world_mut().run_schedule(PreUpdate);

    assert!(
        app.world()
            .resource::<ButtonInput<MouseButton>>()
            .just_pressed(MouseButton::Left)
    );
}

#[test]
fn every_action_is_labelled() {
    for action in [Action::Copy, Action::Duplicate, Action::CopyId] {
        assert!(!action.label().is_empty());
    }
}
