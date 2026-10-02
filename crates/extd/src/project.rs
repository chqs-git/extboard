use extboard_core::{Canvas, Edge, Node, NodeKind, Side};
use serde_json::Value;

pub const CELL: i64 = 20;

pub const SCRIPT_PLACEHOLDER: &str = "<kept>";

pub fn render(canvas: &Canvas) -> String {
    let mut out = format!("grid {CELL}\n");

    for name in script_names(canvas) {
        out.push_str(&format!("script {name} {SCRIPT_PLACEHOLDER}\n"));
    }

    let mut nodes: Vec<&Node> = canvas.nodes.iter().collect();
    nodes.sort_by_key(|node| (cells(node.y), cells(node.x), node.id.clone()));
    for node in nodes {
        out.push_str(&node_line(node));
        out.push('\n');
    }

    for edge in &canvas.edges {
        out.push_str(&edge_line(edge));
        out.push('\n');
    }

    out
}

pub fn cells(units: i64) -> i64 {
    let half = CELL / 2;
    if units >= 0 {
        (units + half) / CELL
    } else {
        (units - half) / CELL
    }
}

fn node_line(node: &Node) -> String {
    let mut line = format!(
        "{} {} @{},{} {}x{}",
        kind_name(&node.kind),
        node.id,
        cells(node.x),
        cells(node.y),
        cells(node.width).max(1),
        cells(node.height).max(1),
    );
    if let Some(color) = &node.color {
        line.push_str(&format!(" c={color}"));
    }
    let payload = payload(&node.kind);
    if !payload.is_empty() {
        line.push_str(&format!(" | {}", escape(&payload)));
    }
    line
}

fn kind_name(kind: &NodeKind) -> &'static str {
    match kind {
        NodeKind::Text { .. } => "text",
        NodeKind::File { .. } => "file",
        NodeKind::Link { .. } => "link",
        NodeKind::Group { .. } => "group",
    }
}

fn payload(kind: &NodeKind) -> String {
    match kind {
        NodeKind::Text { text } => text.clone(),
        // The caption, never the bytes: a file node's path is all the text it
        // has until E8-T4 gives it one.
        NodeKind::File { file, subpath } => match subpath {
            Some(subpath) => format!("{file}{subpath}"),
            None => file.clone(),
        },
        NodeKind::Link { url } => url.clone(),
        NodeKind::Group { label } => label.clone().unwrap_or_default(),
    }
}

fn edge_line(edge: &Edge) -> String {
    let mut line = format!(
        "edge {} {}{} -> {}{}",
        edge.id,
        edge.from_node,
        side(edge.from_side),
        edge.to_node,
        side(edge.to_side),
    );
    if let Some(label) = &edge.label {
        line.push_str(&format!(" | {}", escape(label)));
    }
    line
}

fn side(side: Option<Side>) -> String {
    match side {
        Some(Side::Top) => ":top".to_owned(),
        Some(Side::Right) => ":right".to_owned(),
        Some(Side::Bottom) => ":bottom".to_owned(),
        Some(Side::Left) => ":left".to_owned(),
        None => String::new(),
    }
}

// One node per line, so a newline in a text node has to travel escaped.
fn escape(text: &str) -> String {
    text.replace('\\', "\\\\")
        .replace('\n', "\\n")
        .replace('\r', "\\r")
}

// `extboard.scripts`, plus the single `extboard.script` E6-T4 wrote.
fn script_names(canvas: &Canvas) -> Vec<String> {
    let Some(extboard) = canvas.extra.get("extboard") else {
        return Vec::new();
    };
    if let Some(scripts) = extboard.get("scripts").and_then(Value::as_object) {
        return scripts.keys().cloned().collect();
    }
    match extboard.get("script") {
        Some(Value::String(_)) => vec!["main".to_owned()],
        _ => Vec::new(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn canvas() -> Canvas {
        serde_json::from_str(include_str!(
            "../../core/tests/fixtures/kitchen-sink.canvas"
        ))
        .unwrap()
    }

    #[test]
    fn quantises_snaps_and_reads_top_to_bottom() {
        let mut canvas = canvas();
        // Two units out of line, one cell wide: the same row, never 0 tall.
        canvas.nodes[0].x = 398;
        canvas.nodes[0].y = 1_002;
        canvas.nodes[0].height = 3;
        canvas.nodes[1].x = 802;
        canvas.nodes[1].y = 998;

        let projected = render(&canvas);
        let lines: Vec<&str> = projected.lines().collect();
        assert_eq!(lines[0], "grid 20");

        let rows: Vec<i64> = lines[1..]
            .iter()
            .filter(|line| !line.starts_with("edge"))
            .map(|line| {
                let at = line.split(" @").nth(1).unwrap();
                at.split(',').next().unwrap().parse().unwrap()
            })
            .collect();
        assert!(rows.windows(2).all(|w| w[0] <= w[1]), "{rows:?}");

        let moved: Vec<&&str> = lines
            .iter()
            .filter(|line| line.contains("@20,50") || line.contains("@40,50"))
            .collect();
        assert_eq!(moved.len(), 2, "near-aligned nodes left the row: {lines:?}");
        assert!(moved[0].contains("x1 "), "a node projected to no height");
    }

    #[test]
    fn a_script_travels_as_a_placeholder() {
        let mut canvas = canvas();
        canvas.extra.insert(
            "extboard".to_owned(),
            serde_json::json!({"scripts": {"main": "fn on_click(id) { 1 }"}}),
        );

        let projected = render(&canvas);
        assert!(projected.contains("script main <kept>"), "{projected}");
        assert!(!projected.contains("on_click"), "{projected}");
    }

    // The point of the whole exercise: a board worth reading, cheaply. Real
    // boards land near 3x; node text is the rest of the payload and no
    // projection can shrink it.
    #[test]
    fn costs_a_fraction_of_the_raw_json() {
        let canvas = canvas();
        let projected = render(&canvas);
        let raw = canvas.to_pretty_string();
        assert!(
            projected.len() * 5 < raw.len() * 2,
            "projection {} vs raw {}",
            projected.len(),
            raw.len()
        );
    }
}
