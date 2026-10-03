use serde::{Deserialize, Deserializer, Serialize, de};
use serde_json::{Map, Value};

// main model
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Canvas {
    #[serde(default)]
    pub nodes: Vec<Node>,
    #[serde(default)]
    pub edges: Vec<Edge>,
    #[serde(flatten)]
    pub extra: Map<String, Value>,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Node {
    pub id: String,
    pub x: i64,
    pub y: i64,
    pub width: i64,
    pub height: i64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub color: Option<String>,
    // A key rather than a node type: an unknown key still draws as the rect it
    // was, where an unknown type is skipped.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sides: Option<u8>,
    #[serde(flatten)]
    pub kind: NodeKind,
    #[serde(flatten)]
    pub extra: Map<String, Value>,
}

// throw away struct for the custom Deserialize
#[derive(Deserialize)]
struct NodeRepr {
    id: String,
    x: i64,
    y: i64,
    width: i64,
    height: i64,
    #[serde(default)]
    color: Option<String>,
    #[serde(default)]
    sides: Option<u8>,
    #[serde(flatten)]
    rest: Map<String, Value>,
}

impl<'de> Deserialize<'de> for Node {
    fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        let NodeRepr {
            id,
            x,
            y,
            width,
            height,
            color,
            sides,
            mut rest,
        } = NodeRepr::deserialize(d)?;

        let kind = NodeKind::deserialize(Value::Object(rest.clone())).map_err(de::Error::custom)?;

        if let Value::Object(owned) = serde_json::to_value(&kind).map_err(de::Error::custom)? {
            for key in owned.keys() {
                rest.remove(key);
            }
        }

        Ok(Self {
            id,
            x,
            y,
            width,
            height,
            color,
            sides: sides_of(sides),
            kind,
            extra: rest,
        })
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "camelCase")]
pub enum NodeKind {
    Text {
        text: String,
    },
    File {
        file: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        subpath: Option<String>,
    },
    Link {
        url: String,
    },
    Group {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        label: Option<String>,
    },
}

// What a `file` node draws as a picture rather than as a link. One list, so
// the app, the phone view and the projection agree on what an image is.
pub fn is_image(file: &str) -> bool {
    let lower = file.to_ascii_lowercase();
    [".png", ".jpg", ".jpeg", ".gif", ".webp", ".svg", ".avif"]
        .iter()
        .any(|ext| lower.ends_with(ext))
}

pub const MIN_SIDES: u8 = 3;
pub const CIRCLE_SIDES: u8 = 10;

pub fn sides_of(sides: Option<u8>) -> Option<u8> {
    match sides? {
        ..MIN_SIDES => None,
        sides => Some(sides.min(CIRCLE_SIDES)),
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Edge {
    pub id: String,
    pub from_node: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub from_side: Option<Side>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub from_end: Option<End>,
    pub to_node: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub to_side: Option<Side>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub to_end: Option<End>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub label: Option<String>,
    #[serde(flatten)]
    pub extra: Map<String, Value>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Side {
    Top,
    Right,
    Bottom,
    Left,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum End {
    None,
    Arrow,
}

// A nested object in the extras, made if absent and replaced if it is not one.
// Every write to one of our own keys goes through here, so a hand-written file
// that put a string where we keep a block is overwritten rather than panicked on.
pub fn object_mut<'a>(extra: &'a mut Map<String, Value>, key: &str) -> &'a mut Map<String, Value> {
    let slot = extra
        .entry(key)
        .or_insert_with(|| Value::Object(Map::new()));
    if !slot.is_object() {
        *slot = Value::Object(Map::new());
    }
    slot.as_object_mut().expect("an object either way")
}

impl Canvas {
    // serialize into bytes
    pub fn canonical_bytes(&self) -> Vec<u8> {
        serde_json::to_vec(self).expect("Canvas always serializes")
    }

    // serialize in pretty string format
    pub fn to_pretty_string(&self) -> String {
        serde_json::to_string_pretty(self).expect("Canvas always serializes")
    }
}
