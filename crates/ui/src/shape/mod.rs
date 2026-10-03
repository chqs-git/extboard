use bevy::input_focus::InputFocus;
use bevy::prelude::*;
use extboard_core::{CIRCLE_SIDES, MIN_SIDES, sides_of};

use crate::client::Document;

mod material;
mod panel;

pub use material::{Palette, PolygonMaterial, paint};
use panel::{SidesField, SidesPanel};

const RECT_SIDES: u8 = 4;
const CIRCLE_WORDS: [&str; 4] = ["circle", "c", "o", "\u{221e}"];

pub struct ShapePlugin;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Sides {
    Ngon(u8),
    Circle,
}

impl Plugin for ShapePlugin {
    fn build(&self, app: &mut App) {
        material::register(app);
        app.add_systems(
            Update,
            (panel::sync, panel::typed)
                .chain()
                .run_if(resource_exists::<Document>),
        );
    }
}

impl Sides {
    fn of(sides: Option<u8>) -> Self {
        match sides_of(sides) {
            Some(CIRCLE_SIDES) => Self::Circle,
            Some(sides) => Self::Ngon(sides),
            None => Self::Ngon(RECT_SIDES),
        }
    }

    // Four goes back as no key: the spec already spells that rectangle.
    fn key(self) -> Option<u8> {
        match self {
            Self::Ngon(RECT_SIDES) => None,
            Self::Ngon(sides) => Some(sides),
            Self::Circle => Some(CIRCLE_SIDES),
        }
    }

    fn notch(self) -> f32 {
        f32::from(match self {
            Self::Ngon(sides) => sides,
            Self::Circle => CIRCLE_SIDES,
        })
    }

    fn from_notch(notch: f32) -> Self {
        Self::of(Some(
            notch
                .round()
                .clamp(f32::from(MIN_SIDES), f32::from(CIRCLE_SIDES)) as u8,
        ))
    }

    fn label(self) -> String {
        match self {
            Self::Ngon(sides) => sides.to_string(),
            Self::Circle => "circle".to_owned(),
        }
    }

    fn parse(text: &str) -> Option<Self> {
        let text = text.trim().to_lowercase();
        if CIRCLE_WORDS.contains(&text.as_str()) {
            return Some(Self::Circle);
        }
        match text.parse::<u32>() {
            Ok(count) => Some(Self::from_notch(count.min(u32::from(CIRCLE_SIDES)) as f32)),
            Err(_) => None,
        }
    }
}

// Bypassed on purpose: waking the document respawns every node, panel included.
// sync.rs and undo.rs poll `rev`, so the edit still saves and still undoes.
fn set_sides(document: &mut ResMut<Document>, id: &str, sides: Sides) {
    let canvas = &mut document.bypass_change_detection().0;
    let Some(node) = canvas.nodes.iter_mut().find(|node| node.id == id) else {
        return;
    };
    if node.sides != sides.key() {
        node.sides = sides.key();
    }
}

// A press that reaches the canvas clears the selection, which is this panel's
// own node.
pub fn over_panel(window: Single<&Window>, panels: Query<(), With<SidesPanel>>) -> bool {
    if panels.is_empty() {
        return false;
    }
    let Some(at) = window.cursor_position() else {
        return false;
    };
    panel::holds(&window, at)
}

pub fn typing(focus: Res<InputFocus>, fields: Query<(), With<SidesField>>) -> bool {
    focus.get().is_some_and(|entity| fields.contains(entity))
}

#[cfg(test)]
mod tests;
