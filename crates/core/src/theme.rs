use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::{Canvas, object_mut};

// Five roles have names, so a space always has a background and three colours
// to tell its nodes apart. Past that they are extras, drawn on in order.
pub const MIN_COLORS: usize = 5;
pub const MAX_COLORS: usize = 20;
pub const ROLES: [&str; MIN_COLORS] = ["background", "primary", "secondary", "accent", "text"];

pub const BACKGROUND: usize = 0;
pub const PRIMARY: usize = 1;
pub const SECONDARY: usize = 2;
pub const ACCENT: usize = 3;
pub const TEXT: usize = 4;

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Theme {
    #[serde(default)]
    pub name: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub colors: Vec<String>,
}

// The palettes that ship. The first is what a space with no theme gets, and
// every one of them is eight colours: the five roles and three extra accents.
pub const PRESETS: [(&str, [&str; 8]); 4] = [
    (
        "tokyo-night-dark",
        [
            "#1a1b26", "#7aa2f7", "#414868", "#f7768e", "#c0caf5", "#e0af68", "#bb9af7", "#9ece6a",
        ],
    ),
    (
        "tokyo-night-light",
        [
            "#e6e7ed", "#2959aa", "#c9cbd1", "#8c4351", "#212b42", "#5a3e8e", "#385f0d", "#b15c00",
        ],
    ),
    (
        "EverForest",
        [
            "#282e31", "#8da06e", "#363e43", "#494841", "#8da06e", "#52796f", "#84a98c", "#6a994e",
        ],
    ),
    (
        "dracula-dark",
        [
            "#282a36", "#bd93f9", "#44475a", "#ff79c6", "#f8f8f2", "#8be9fd", "#50fa7b", "#ff5555",
        ],
    ),
];

impl Theme {
    pub fn preset_named(name: &str) -> Option<Self> {
        let name = alias(name);
        PRESETS
            .iter()
            .find(|(id, _)| *id == name)
            .map(|(id, colors)| Self {
                name: (*id).to_owned(),
                colors: colors.iter().map(|hex| (*hex).to_owned()).collect(),
            })
    }

    // Without building a `Theme`: the panel asks this on every open frame.
    pub fn preset_colors(name: &str) -> Option<&'static [&'static str; 8]> {
        let name = alias(name);
        PRESETS
            .iter()
            .find(|(id, _)| *id == name)
            .map(|(_, colors)| colors)
    }

    // A name we do not know falls back: a space can say anything and still draws.
    pub fn preset(name: &str) -> Self {
        Self::preset_named(name)
            .unwrap_or_else(|| Self::preset_named(PRESETS[0].0).expect("the first preset"))
    }

    // The mismatch rule: a theme with fewer colours than the space asks for
    // wraps. `get` over `%` so an empty palette answers `None` instead of
    // dividing by zero.
    pub fn color(&self, index: usize) -> Option<&str> {
        if self.colors.is_empty() {
            return None;
        }
        self.colors
            .get(index % self.colors.len())
            .map(String::as_str)
    }

    // The label a colour carries in the panel; the extras are more accents.
    pub fn role(index: usize) -> String {
        ROLES.get(index).map_or_else(
            || format!("accent-{}", (b'b' + (index - ROLES.len()) as u8) as char),
            |role| (*role).to_owned(),
        )
    }
}

// `set_theme("dark")` predates the presets and still has to mean something.
fn alias(name: &str) -> &str {
    match name {
        "" | "dark" => "tokyo-night-dark",
        "light" => "tokyo-night-light",
        name => name,
    }
}

impl Canvas {
    // The space's own colours when it carries any, the named preset's otherwise.
    pub fn theme(&self) -> Theme {
        let stored = self.stored_theme();
        if stored.colors.is_empty() {
            return Theme::preset(&stored.name);
        }
        stored
    }

    pub fn stored_theme(&self) -> Theme {
        let mut theme: Theme = self
            .extra
            .get("theme")
            .cloned()
            .and_then(|value| serde_json::from_value(value).ok())
            .unwrap_or_default();
        theme.colors.truncate(MAX_COLORS);
        theme
    }

    // Merged rather than replaced: a theme block can carry keys that are not
    // ours, and an empty palette means "follow the preset" rather than "black".
    pub fn set_theme(&mut self, theme: &Theme) {
        let block = object_mut(&mut self.extra, "theme");
        block.insert("name".to_owned(), Value::String(theme.name.clone()));
        match theme.colors.as_slice() {
            [] => block.remove("colors"),
            colors => block.insert(
                "colors".to_owned(),
                colors
                    .iter()
                    .take(MAX_COLORS)
                    .map(|hex| Value::String(hex.clone()))
                    .collect(),
            ),
        };
    }

    // Beside the scripts under the key already ours. `serde_json`'s map is
    // sorted, which is what keeps the list stable.
    pub fn themes(&self) -> Vec<Theme> {
        self.extra
            .get("extboard")
            .and_then(|extboard| extboard.get("themes"))
            .and_then(Value::as_object)
            .map(|saved| {
                saved
                    .iter()
                    .filter_map(|(name, colors)| {
                        Some(Theme {
                            name: name.clone(),
                            colors: serde_json::from_value(colors.clone()).ok()?,
                        })
                    })
                    .collect()
            })
            .unwrap_or_default()
    }

    // A preset first: the shipped names are not available to be taken.
    pub fn saved_theme(&self, name: &str) -> Option<Theme> {
        Theme::preset_named(name)
            .or_else(|| self.themes().into_iter().find(|theme| theme.name == name))
    }

    // A theme with no name is one nothing can select again.
    pub fn save_theme(&mut self, theme: &Theme) -> bool {
        if theme.name.is_empty()
            || theme.colors.is_empty()
            || Theme::preset_named(&theme.name).is_some()
        {
            return false;
        }
        let colors = theme
            .colors
            .iter()
            .take(MAX_COLORS)
            .map(|hex| Value::String(hex.clone()))
            .collect();
        object_mut(object_mut(&mut self.extra, "extboard"), "themes")
            .insert(theme.name.clone(), colors);
        true
    }

    pub fn fresh_theme_name(&self, from: &str) -> String {
        let stem = if from.is_empty() { "theme" } else { from };
        (2..)
            .map(|n| format!("{stem} {n}"))
            .find(|name| self.saved_theme(name).is_none())
            .expect("the integers run out after the names do")
    }
}
