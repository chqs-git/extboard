use bevy::prelude::*;
use bevy::time::common_conditions::on_timer;
use extboard_core::{Canvas, rev};
use std::collections::VecDeque;
use std::time::Duration;

use crate::client::Document;
use crate::edit::command;
use crate::script;

// Whole documents, which are kilobytes: fifty of them is cheaper than the code
// an operation log would take to get right.
const DEPTH: usize = 50;
// The same debounce a save uses, and for the same reason: a gesture is a run of
// changes, and one drag has to be one entry.
const IDLE_SECS: f32 = 0.5;
const CHECK: Duration = Duration::from_millis(100);

pub struct UndoPlugin;

#[derive(Resource, Default)]
struct History {
    // The document as it stood when the last gesture finished, and its rev. This
    // is what the next gesture pushes onto `past`; `None` until the first one
    // arrives, which is not a state to undo past.
    current: Option<Canvas>,
    seen: String,
    // The rev last seen live, and when it last differed: the settle window.
    pending: String,
    quiet_at: f32,
    past: VecDeque<Canvas>,
    future: Vec<Canvas>,
}

impl Plugin for UndoPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<History>().add_systems(
            Update,
            (
                // Typing owns the keyboard, and the buffer is not in the document
                // until the edit ends.
                restore.run_if(not(crate::text::editing)),
                record.run_if(on_timer(CHECK)),
            )
                .chain()
                .run_if(resource_exists::<Document>),
        );
    }
}

impl History {
    // `true` once the document holds something new that has stopped changing.
    fn settled(&mut self, now: f32, live: &str) -> bool {
        if self.pending != live {
            self.pending = live.to_owned();
            self.quiet_at = now;
            return false;
        }
        self.pending != self.seen && now - self.quiet_at >= IDLE_SECS
    }

    // The gesture is over: what came before it is the step to go back to, and a
    // new edit is what makes the future unreachable.
    fn commit(&mut self, canvas: Canvas, at: String) {
        if let Some(previous) = self.current.replace(canvas) {
            self.past.push_back(previous);
            if self.past.len() > DEPTH {
                self.past.pop_front();
            }
            self.future.clear();
        }
        self.seen = at;
    }

    fn undo(&mut self, live: &Canvas) -> Option<Canvas> {
        let previous = self.past.pop_back()?;
        self.future.push(live.clone());
        Some(self.adopt(live, previous))
    }

    fn redo(&mut self, live: &Canvas) -> Option<Canvas> {
        let next = self.future.pop()?;
        self.past.push_back(live.clone());
        Some(self.adopt(live, next))
    }

    // A step this stack took is not a change to record: adopting it as the
    // settled state is what keeps `record` from pushing it straight back.
    fn adopt(&mut self, live: &Canvas, mut canvas: Canvas) -> Canvas {
        // The scripts belong to the sidebar's history, so a step moves the
        // canvas and leaves the buffers where the person left them.
        script::carry_scripts(live, &mut canvas);
        self.seen = step_rev(&canvas);
        self.current = Some(canvas.clone());
        canvas
    }
}

// The sidebar keeps the scripts' history (E6-T5), so this stack is blind to them:
// a script edit is not a step back, and a step back is not a way to lose one.
// A clone per check, which is kilobytes — `record` clones the document anyway.
fn step_rev(canvas: &Canvas) -> String {
    rev(&script::without_script(canvas))
}

fn record(time: Res<Time>, document: Res<Document>, mut history: ResMut<History>) {
    let live = step_rev(&document.0);
    if !history.settled(time.elapsed_secs(), &live) {
        return;
    }
    history.commit(document.0.clone(), live);
}

fn restore(
    keys: Res<ButtonInput<KeyCode>>,
    mut document: ResMut<Document>,
    mut history: ResMut<History>,
) {
    if !command(&keys) {
        return;
    }
    let shift = keys.any_pressed([KeyCode::ShiftLeft, KeyCode::ShiftRight]);
    let step = if keys.just_pressed(KeyCode::KeyY) || (shift && keys.just_pressed(KeyCode::KeyZ)) {
        History::redo
    } else if keys.just_pressed(KeyCode::KeyZ) {
        History::undo
    } else {
        return;
    };

    if let Some(restored) = step(&mut history, &document.0) {
        document.0 = restored;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn canvas(width: i64) -> Canvas {
        let node = format!(
            r#"{{"id":"n","type":"text","x":0,"y":0,"width":{width},"height":10,"text":"hi"}}"#
        );
        serde_json::from_str(&format!(r#"{{"nodes":[{node}],"edges":[]}}"#)).expect("fixture")
    }

    // Each call is one finished gesture, the way `record` sees one.
    fn gesture(history: &mut History, canvas: Canvas) {
        let at = step_rev(&canvas);
        history.commit(canvas, at);
    }

    fn width(canvas: &Canvas) -> i64 {
        canvas.nodes[0].width
    }

    // The ticket's done-when: ten operations undo and redo correctly.
    #[test]
    fn ten_gestures_walk_back_and_forward_again() {
        let mut history = History::default();
        gesture(&mut history, canvas(0));
        for step in 1..=10 {
            gesture(&mut history, canvas(step));
        }

        let mut live = canvas(10);
        for step in (0..10).rev() {
            live = history.undo(&live).expect("a step back");
            assert_eq!(width(&live), step);
        }
        assert!(history.undo(&live).is_none(), "past the first document");

        for step in 1..=10 {
            live = history.redo(&live).expect("a step forward");
            assert_eq!(width(&live), step);
        }
        assert!(history.redo(&live).is_none());
    }

    #[test]
    fn a_new_edit_drops_the_future() {
        let mut history = History::default();
        gesture(&mut history, canvas(0));
        gesture(&mut history, canvas(1));

        let live = history.undo(&canvas(1)).expect("a step back");
        assert_eq!(width(&live), 0);

        gesture(&mut history, canvas(7));
        assert!(history.future.is_empty());
        assert!(history.redo(&canvas(7)).is_none());
    }

    // Holding the key down must not grow without bound.
    #[test]
    fn the_stack_is_capped_and_drops_its_oldest() {
        let mut history = History::default();
        let gestures = DEPTH as i64 * 2;
        for step in 0..gestures {
            gesture(&mut history, canvas(step));
        }
        assert_eq!(history.past.len(), DEPTH);

        let mut live = canvas(gestures);
        while let Some(previous) = history.undo(&live) {
            live = previous;
        }
        // The first gesture recorded nothing to go back to, so 99 steps were
        // pushed and all but the newest 50 are gone rather than replayed.
        assert_eq!(width(&live), gestures - 1 - DEPTH as i64);
        assert_eq!(history.future.len(), DEPTH);
    }

    #[test]
    fn one_gesture_is_one_entry_however_many_frames_it_takes() {
        let mut history = History::default();
        gesture(&mut history, canvas(0));
        let mut now = 0.0;

        // A drag: every check sees a different document.
        for step in 1..100 {
            now += CHECK.as_secs_f32();
            assert!(!history.settled(now, &step_rev(&canvas(step))));
        }

        // Let go, and the run becomes one step back.
        let quiet = step_rev(&canvas(99));
        assert!(
            !history.settled(now, &quiet),
            "the first quiet check settles"
        );
        now += IDLE_SECS;
        assert!(history.settled(now, &quiet));
        history.commit(canvas(99), quiet);
        assert_eq!(history.past.len(), 1);
    }

    // The sidebar's stack is its own: typing Rhai is not a step this one takes,
    // and a step it does take leaves every script where it was.
    #[test]
    fn the_script_is_not_part_of_a_canvas_step() {
        let mut history = History::default();
        let mut live = canvas(0);
        gesture(&mut history, live.clone());

        // A script edit settles into no new rev, so no step is ever recorded.
        script::set_script(
            &mut live,
            "themes",
            r#"on_click("n", || set_theme("dark"));"#,
        );
        assert_eq!(step_rev(&live), step_rev(&canvas(0)));

        // And a step back keeps the script the sidebar is holding.
        gesture(&mut history, canvas(1));
        let back = history.undo(&live).expect("a step back");
        assert_eq!(width(&back), 0);
        assert!(script::script(&back, "themes").contains("on_click"));
    }

    // Ctrl-Z over a theme edit needs nothing of its own: the palette is part of
    // the canvas, so this stack already carries it. `without_script` strips the
    // scripts and only the scripts, which is what makes that true.
    #[test]
    fn a_theme_edit_is_a_step_this_stack_takes() {
        let mut history = History::default();
        let mut live = canvas(0);
        gesture(&mut history, live.clone());

        let mine = extboard_core::Theme {
            name: "mine".to_owned(),
            colors: vec!["#111111".to_owned()],
        };
        live.set_theme(&mine);
        assert!(live.save_theme(&mine), "and the library with it");
        assert_ne!(step_rev(&live), step_rev(&canvas(0)));

        gesture(&mut history, live.clone());
        let back = history.undo(&live).expect("a step back");
        assert_eq!(back.stored_theme(), extboard_core::Theme::default());
        assert!(back.themes().is_empty());
    }

    // An undo writes the document too, and that write is not a new step.
    #[test]
    fn a_step_this_stack_took_is_not_recorded_as_an_edit() {
        let mut history = History::default();
        gesture(&mut history, canvas(0));
        gesture(&mut history, canvas(1));

        let live = history.undo(&canvas(1)).expect("a step back");
        let back = step_rev(&live);
        // However long it sits there, it stays one step back.
        assert!(!history.settled(0.0, &back));
        assert!(!history.settled(9.0, &back));
        assert_eq!(history.past.len(), 0);
        assert_eq!(history.future.len(), 1);
    }
}
