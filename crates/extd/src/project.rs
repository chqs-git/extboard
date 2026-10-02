use extboard_core::{Canvas, Edge, Node, NodeKind, Side, validate};
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

pub fn unproject(projected: &str, original: &Canvas) -> Result<Canvas, String> {
    let mut canvas = Canvas {
        nodes: Vec::new(),
        edges: Vec::new(),
        extra: original.extra.clone(),
    };

    for (number, line) in projected.lines().enumerate() {
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        let at = |e: String| format!("line {}: {e}", number + 1);
        match line.split_whitespace().next() {
            Some("grid") => grid(line).map_err(at)?,
            Some("script") => {}
            Some("edge") => canvas.edges.push(edge(line, original).map_err(at)?),
            Some(_) => canvas.nodes.push(node(line, original).map_err(at)?),
            None => {}
        }
    }

    validate(&canvas).map_err(|errors| {
        errors
            .iter()
            .map(ToString::to_string)
            .collect::<Vec<_>>()
            .join("\n")
    })?;

    Ok(canvas)
}

pub fn units(cells: i64) -> i64 {
    cells * CELL
}

// The grid is fixed, so a header naming another one is a projection from a
// different reference frame and nothing here can place it.
fn grid(line: &str) -> Result<(), String> {
    match line.split_whitespace().nth(1).map(str::parse::<i64>) {
        Some(Ok(CELL)) => Ok(()),
        _ => Err(format!("grid must be {CELL}: {line}")),
    }
}

fn node(line: &str, original: &Canvas) -> Result<Node, String> {
    let (head, payload) = split_payload(line);
    let mut words = head.split_whitespace();
    let (Some(kind), Some(id), Some(at), Some(size)) =
        (words.next(), words.next(), words.next(), words.next())
    else {
        return Err(format!("not a node: {head}"));
    };

    let (x, y) = pair(at.strip_prefix('@').unwrap_or(at), ',', "@col,row")?;
    let (width, height) = pair(size, 'x', "WxH")?;

    let was = original.nodes.iter().find(|node| node.id == id);
    let kind = kind_of(kind, &payload, was)?;

    Ok(Node {
        id: id.to_owned(),
        x: units(x),
        y: units(y),
        width: units(width),
        height: units(height),
        color: words
            .next()
            .and_then(|word| word.strip_prefix("c="))
            .map(str::to_owned),
        kind,
        extra: was.map(|node| node.extra.clone()).unwrap_or_default(),
    })
}

// An untouched payload keeps the node it came from, which is how a file node's
// `subpath` and a text node's exact bytes survive the trip.
fn kind_of(kind: &str, payload: &str, was: Option<&Node>) -> Result<NodeKind, String> {
    if let Some(node) = was
        && kind_name(&node.kind) == kind
        && self::payload(&node.kind) == payload
    {
        return Ok(node.kind.clone());
    }
    match kind {
        "text" => Ok(NodeKind::Text {
            text: payload.to_owned(),
        }),
        "file" => {
            let (file, subpath) = match payload.split_once('#') {
                Some((file, subpath)) => (file.to_owned(), Some(format!("#{subpath}"))),
                None => (payload.to_owned(), None),
            };
            Ok(NodeKind::File { file, subpath })
        }
        "link" => Ok(NodeKind::Link {
            url: payload.to_owned(),
        }),
        "group" => Ok(NodeKind::Group {
            label: (!payload.is_empty()).then(|| payload.to_owned()),
        }),
        other => Err(format!("{other} is not a node type")),
    }
}

fn edge(line: &str, original: &Canvas) -> Result<Edge, String> {
    let (head, label) = split_payload(line);
    let mut words = head.split_whitespace();
    let (Some(_), Some(id), Some(from), Some("->"), Some(to)) = (
        words.next(),
        words.next(),
        words.next(),
        words.next(),
        words.next(),
    ) else {
        return Err(format!("not an edge: {head}"));
    };

    let (from_node, from_side) = endpoint(from)?;
    let (to_node, to_side) = endpoint(to)?;
    let was = original.edges.iter().find(|edge| edge.id == id);

    Ok(Edge {
        id: id.to_owned(),
        from_node,
        from_side,
        from_end: was.and_then(|edge| edge.from_end),
        to_node,
        to_side,
        to_end: was.and_then(|edge| edge.to_end),
        label: (!label.is_empty()).then(|| label.clone()),
        extra: was.map(|edge| edge.extra.clone()).unwrap_or_default(),
    })
}

fn endpoint(word: &str) -> Result<(String, Option<Side>), String> {
    let Some((id, side)) = word.split_once(':') else {
        return Ok((word.to_owned(), None));
    };
    let side = match side {
        "top" => Side::Top,
        "right" => Side::Right,
        "bottom" => Side::Bottom,
        "left" => Side::Left,
        other => return Err(format!("{other} is not a side")),
    };
    Ok((id.to_owned(), Some(side)))
}

// The payload runs to the end of the line, so only the first separator counts.
fn split_payload(line: &str) -> (&str, String) {
    match line.split_once(" | ") {
        Some((head, payload)) => (head, unescape(payload)),
        None => (line, String::new()),
    }
}

fn pair(word: &str, between: char, shape: &str) -> Result<(i64, i64), String> {
    let parsed = word
        .split_once(between)
        .and_then(|(a, b)| Some((a.parse().ok()?, b.parse().ok()?)));
    parsed.ok_or_else(|| format!("{word} is not {shape}"))
}

fn unescape(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut chars = text.chars();
    while let Some(c) = chars.next() {
        if c != '\\' {
            out.push(c);
            continue;
        }
        match chars.next() {
            Some('n') => out.push('\n'),
            Some('r') => out.push('\r'),
            Some('\\') => out.push('\\'),
            Some(other) => {
                out.push('\\');
                out.push(other);
            }
            None => out.push('\\'),
        }
    }
    out
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

    // Quantisation is lossy by design, so the invariant is "quantised equals
    // quantised", never "equals the original".
    #[test]
    fn a_round_trip_through_the_grid_settles() {
        let original = canvas();
        let projected = render(&original);
        let back = unproject(&projected, &original).unwrap();

        assert_eq!(render(&back), projected);
        assert_eq!(back.nodes.len(), original.nodes.len());
        assert_eq!(
            render(&unproject(&render(&back), &back).unwrap()),
            projected
        );
    }

    #[test]
    fn the_script_and_the_extras_come_back() {
        let mut original = canvas();
        original.extra.insert(
            "extboard".to_owned(),
            serde_json::json!({"scripts": {"main": "fn on_click(id) { 1 }"}}),
        );
        original.nodes[0]
            .extra
            .insert("mine".to_owned(), serde_json::json!(7));
        original.edges[0].to_end = Some(extboard_core::End::None);

        let back = unproject(&render(&original), &original).unwrap();

        assert_eq!(back.extra, original.extra);
        let node = back
            .nodes
            .iter()
            .find(|node| node.id == original.nodes[0].id)
            .unwrap();
        assert_eq!(node.extra.get("mine"), Some(&serde_json::json!(7)));
        assert_eq!(back.edges[0].to_end, Some(extboard_core::End::None));
    }

    #[test]
    fn an_invented_id_is_rejected_by_id() {
        let original = canvas();
        let mut projected = render(&original);
        projected.push_str("edge e9 nope -> alsonope\n");

        let error = unproject(&projected, &original).unwrap_err();
        assert!(error.contains("nope"), "{error}");
        assert!(error.contains("does not exist"), "{error}");
    }

    #[test]
    fn a_broken_line_names_its_number() {
        let original = canvas();

        for (line, expected) in [
            ("grid 7", "grid must be 20"),
            ("text n1 @zero,0 2x2", "is not @col,row"),
            ("blob n1 @0,0 2x2", "is not a node type"),
            ("edge e1 a -> b:sideways", "is not a side"),
        ] {
            let error = unproject(&format!("grid 20\n{line}\n"), &original).unwrap_err();
            assert!(error.starts_with("line 2:"), "{line}: {error}");
            assert!(error.contains(expected), "{line}: {error}");
        }
    }

    #[test]
    fn a_newline_in_a_text_node_survives() {
        let mut original = canvas();
        original.nodes[1].kind = NodeKind::Text {
            text: "## Day 1\n\na back\\slash and a | pipe".to_owned(),
        };

        let back = unproject(&render(&original), &original).unwrap();
        let node = back
            .nodes
            .iter()
            .find(|node| node.id == original.nodes[1].id)
            .unwrap();
        assert_eq!(node.kind, original.nodes[1].kind);
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
