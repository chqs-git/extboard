//! `GET /v/<space>` — server-rendered read-only HTML, no JS. Phase 3.
//!
//! The phone path: positions, markdown, images, native text selection. Folded
//! into extd rather than living in its own crate — it is one handler with no
//! second consumer.

use extboard_core::{
    CIRCLE_SIDES, Canvas, End, Node, NodeKind, edge_ends, is_image, sides_inset, sides_polygon,
};
use pulldown_cmark::{Event, Options, Parser};
use std::fmt::Write;

// Room for the arrowheads and the group labels that sit on the bounding box.
const MARGIN: f32 = 40.0;

/// The whole page for one space.
pub fn page(space: &str, canvas: &Canvas) -> String {
    let (origin, size) = extent(canvas);
    let at = |x: f32, y: f32| (x - origin.0, y - origin.1);

    let (lines, labels) = edges(canvas, at);

    let mut body = String::new();
    // Edges first: an arrow crossing a node should pass behind it.
    let _ = write!(
        body,
        r#"<svg width="{w}" height="{h}" aria-hidden="true">{lines}</svg>"#,
        w = size.0,
        h = size.1
    );
    // Big rects first, so a group never covers what sits inside it.
    let mut nodes: Vec<&Node> = canvas.nodes.iter().collect();
    nodes.sort_by_key(|node| std::cmp::Reverse(node.width * node.height));
    for node in nodes {
        let (x, y) = at(node.x as f32, node.y as f32);
        let _ = write!(
            body,
            r#"<div class="node {kind}" style="left:{x}px;top:{y}px;width:{w}px;height:{h}px{shape}">{}</div>"#,
            contents(node),
            kind = kind_class(&node.kind),
            shape = shape_style(node),
            w = node.width,
            h = node.height,
        );
    }
    // Last, so a label stays readable over whatever it crosses.
    body.push_str(&labels);

    // `width=<board>` alone hands the phone a board-sized page: it shrinks it to
    // fit on load and pinch-zoom is the browser's, not ours. An `initial-scale`
    // here would pin it at 1 and land the phone in the top-left corner.
    format!(
        r#"<!doctype html>
<html lang="en">
<head>
<meta charset="utf-8">
<meta name="viewport" content="width={w}">
<meta name="color-scheme" content="dark">
<title>{title}</title>
<style>{STYLE}</style>
</head>
<body>
<main style="width:{w}px;height:{h}px">
{body}</main>
</body>
</html>
"#,
        title = esc(space),
        w = size.0,
        h = size.1,
    )
}

const STYLE: &str = "\
html,body{margin:0;background:#12121a;color:#e0e4ea;\
font:15px/1.45 ui-sans-serif,system-ui,sans-serif}\
main{position:relative}\
svg{position:absolute;inset:0;overflow:visible}\
line{stroke:#737f8c;stroke-width:3}\
polygon{fill:#737f8c}\
.node{position:absolute;box-sizing:border-box;padding:12px;border-radius:6px;\
overflow:hidden}\
.text{background:#5a8cc4}\
.file{background:#49a87c}\
.link{background:#a878c4}\
.group{background:#3a3f4b33;border:2px solid #3a3f4b;overflow:visible}\
.group>.label{position:absolute;top:-28px;left:0;color:#9aa4b2;font-size:13px}\
.node h1{font-size:26px;margin:0 0 8px}\
.node h2{font-size:21px;margin:0 0 8px}\
.node h3,.node h4,.node h5,.node h6{font-size:17px;margin:0 0 8px}\
.node p,.node ul,.node ol,.node table{margin:0 0 8px}\
.node code{background:#0f1116;border-radius:3px;padding:1px 4px}\
.node pre{background:#0f1116;border-radius:4px;padding:8px;overflow:auto}\
.node pre code{background:none;padding:0}\
.node a{color:#dce8ff}\
.node img{max-width:100%;max-height:100%;object-fit:contain}\
.node table{border-collapse:collapse}\
.node th,.node td{border:1px solid #444b57;padding:4px 8px}\
.edge-label{position:absolute;transform:translate(-50%,-50%);color:#e0e4ea;\
font-size:13px;background:#12121ac0;padding:0 4px;border-radius:3px}";

/// Top-left corner and size of everything on the board, plus a margin.
fn extent(canvas: &Canvas) -> ((f32, f32), (f32, f32)) {
    let Some(first) = canvas.nodes.first() else {
        return ((0.0, 0.0), (320.0, 240.0));
    };
    let (mut min_x, mut min_y) = (first.x, first.y);
    let (mut max_x, mut max_y) = (first.x + first.width, first.y + first.height);
    for node in &canvas.nodes {
        min_x = min_x.min(node.x);
        min_y = min_y.min(node.y);
        max_x = max_x.max(node.x + node.width);
        max_y = max_y.max(node.y + node.height);
    }
    (
        (min_x as f32 - MARGIN, min_y as f32 - MARGIN),
        (
            (max_x - min_x) as f32 + 2.0 * MARGIN,
            (max_y - min_y) as f32 + 2.0 * MARGIN,
        ),
    )
}

/// The `<line>`/`<polygon>` markup, and the labels as HTML: `<foreignObject>`
/// is a worse way to get text a phone will happily select.
fn edges(canvas: &Canvas, at: impl Fn(f32, f32) -> (f32, f32)) -> (String, String) {
    let find = |id: &str| canvas.nodes.iter().find(|node| node.id == id);
    let mut svg = String::new();
    let mut labels = String::new();

    for edge in &canvas.edges {
        // extd validates both ends on write, but a file edited between reads
        // can still dangle: skip it rather than panic.
        let (Some(from), Some(to)) = (find(&edge.from_node), find(&edge.to_node)) else {
            continue;
        };
        let (a, b) = edge_ends(from, edge.from_side, to, edge.to_side);
        let (a, b) = (at(a.0, a.1), at(b.0, b.1));
        let _ = write!(
            svg,
            r#"<line x1="{}" y1="{}" x2="{}" y2="{}"/>"#,
            a.0, a.1, b.0, b.1
        );
        if edge.to_end.unwrap_or(End::Arrow) == End::Arrow {
            svg.push_str(&arrow_head(b, a));
        }
        if edge.from_end.unwrap_or(End::None) == End::Arrow {
            svg.push_str(&arrow_head(a, b));
        }
        if let Some(label) = &edge.label {
            let _ = write!(
                labels,
                r#"<div class="edge-label" style="left:{}px;top:{}px">{}</div>"#,
                (a.0 + b.0) / 2.0,
                (a.1 + b.1) / 2.0,
                esc(label)
            );
        }
    }
    (svg, labels)
}

// A filled triangle at `tip`, pointing away from `tail`.
fn arrow_head(tip: (f32, f32), tail: (f32, f32)) -> String {
    const LEN: f32 = 18.0;
    const HALF: f32 = 8.0;

    let (dx, dy) = (tip.0 - tail.0, tip.1 - tail.1);
    let len = dx.hypot(dy);
    if len == 0.0 {
        return String::new();
    }
    let (ux, uy) = (dx / len, dy / len);
    let base = (tip.0 - ux * LEN, tip.1 - uy * LEN);
    // Perpendicular, for the two back corners.
    let (px, py) = (-uy * HALF, ux * HALF);
    format!(
        r#"<polygon points="{},{} {},{} {},{}"/>"#,
        tip.0,
        tip.1,
        base.0 + px,
        base.1 + py,
        base.0 - px,
        base.1 - py
    )
}

fn kind_class(kind: &NodeKind) -> &'static str {
    match kind {
        NodeKind::Text { .. } => "text",
        NodeKind::File { .. } => "file",
        NodeKind::Link { .. } => "link",
        NodeKind::Group { .. } => "group",
    }
}

fn shape_style(node: &Node) -> String {
    let Some(sides) = node.sides else {
        return String::new();
    };
    // Pixels, not percent: a CSS percentage padding is the width on all sides.
    let inset = sides_inset(node.sides);
    let pad = format!(
        ";padding:{:.0}px {:.0}px",
        inset * node.height as f32,
        inset * node.width as f32
    );
    match sides {
        CIRCLE_SIDES => format!(";border-radius:50%{pad}"),
        sides => format!(";clip-path:polygon({}){pad}", ngon_points(sides)),
    }
}

fn ngon_points(sides: u8) -> String {
    sides_polygon(sides)
        .iter()
        .map(|(x, y)| format!("{:.1}% {:.1}%", 50.0 + 100.0 * x, 50.0 + 100.0 * y))
        .collect::<Vec<_>>()
        .join(",")
}

fn contents(node: &Node) -> String {
    match &node.kind {
        NodeKind::Text { text } => markdown(text),
        NodeKind::File { file, .. } => file_node(file),
        NodeKind::Link { url } => match safe_url(url) {
            Some(href) => format!(r#"<a href="{}">{}</a>"#, esc(&href), esc(url)),
            None => esc(url),
        },
        NodeKind::Group { label } => match label {
            Some(label) => format!(r#"<div class="label">{}</div>"#, esc(label)),
            None => String::new(),
        },
    }
}

fn file_node(file: &str) -> String {
    let src = format!("/f/{}", url_path(file));
    if is_image(file) {
        format!(r#"<img src="{}" alt="{}">"#, esc(&src), esc(file))
    } else {
        format!(r#"<a href="{}">{}</a>"#, esc(&src), esc(file))
    }
}

/// Markdown to HTML, with every raw-HTML event dropped. A canvas can arrive by
/// `cp` from someone else, so its text is untrusted input, not our markup.
fn markdown(text: &str) -> String {
    let parser = Parser::new_ext(text, Options::ENABLE_TABLES | Options::ENABLE_STRIKETHROUGH).map(
        |event| match event {
            Event::Html(raw) | Event::InlineHtml(raw) => Event::Text(raw),
            other => other,
        },
    );
    let mut out = String::new();
    pulldown_cmark::html::push_html(&mut out, parser);
    out
}

/// `javascript:` and friends never reach an `href`. Relative links stay: they
/// resolve against `/v/`, which is this page.
fn safe_url(url: &str) -> Option<String> {
    match url.split_once(':') {
        None => Some(url.to_owned()),
        Some((scheme, _)) if scheme.contains('/') => Some(url.to_owned()),
        Some((scheme, _)) => matches!(
            scheme.to_ascii_lowercase().as_str(),
            "http" | "https" | "mailto" | "tel"
        )
        .then(|| url.to_owned()),
    }
}

// Enough percent-encoding for a filename in a URL path; `/` stays a separator.
fn url_path(path: &str) -> String {
    let mut out = String::new();
    for byte in path.as_bytes() {
        match byte {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'.' | b'_' | b'~' | b'/' => {
                out.push(*byte as char);
            }
            _ => out.push_str(&format!("%{byte:02X}")),
        }
    }
    out
}

/// The trust boundary: everything that came out of a canvas file goes through
/// here before it reaches the page.
fn esc(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for c in text.chars() {
        match c {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            '\'' => out.push_str("&#39;"),
            _ => out.push(c),
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use extboard_core::Edge;

    fn text_node(id: &str, x: i64, y: i64, text: &str) -> Node {
        Node {
            id: id.to_owned(),
            x,
            y,
            width: 200,
            height: 100,
            color: None,
            sides: None,
            kind: NodeKind::Text {
                text: text.to_owned(),
            },
            extra: Default::default(),
        }
    }

    fn canvas(nodes: Vec<Node>, edges: Vec<Edge>) -> Canvas {
        Canvas {
            nodes,
            edges,
            extra: Default::default(),
        }
    }

    // A pinned `initial-scale` beats `width=<board>` and strands the phone in the
    // top-left corner of an unscrollable page.
    #[test]
    fn viewport_lets_the_phone_shrink_to_fit() {
        let html = page("s", &canvas(vec![text_node("a", 0, 0, "hi")], vec![]));
        assert!(!html.contains("initial-scale"), "{html}");
    }

    #[test]
    fn a_node_is_clipped_to_its_side_count() {
        let shaped = |sides| Node {
            sides,
            ..text_node("a", 0, 0, "hi")
        };
        let hex = shape_style(&shaped(Some(6)));
        assert_eq!(hex.matches('%').count(), 12, "{hex}");
        assert!(hex.contains("padding:"), "text would cross an edge: {hex}");
        assert!(shape_style(&shaped(Some(CIRCLE_SIDES))).contains("border-radius:50%"));
        assert!(shape_style(&shaped(None)).is_empty());
        let square = shape_style(&shaped(Some(4)));
        assert!(!square.contains("50.0% 0.0%"), "{square}");
    }

    // The one that matters: a board is untrusted input, and this file writes
    // HTML by hand.
    #[test]
    fn canvas_text_cannot_inject_markup() {
        let evil = canvas(
            vec![
                text_node("a", 0, 0, "<script>alert(1)</script> and <img onerror=x>"),
                Node {
                    kind: NodeKind::Link {
                        url: "javascript:alert(1)".to_owned(),
                    },
                    ..text_node("b", 300, 0, "")
                },
            ],
            vec![Edge {
                id: "e".to_owned(),
                from_node: "a".to_owned(),
                from_side: None,
                from_end: None,
                to_node: "b".to_owned(),
                to_side: None,
                to_end: None,
                label: Some("</svg><script>alert(1)</script>".to_owned()),
                color: None,
                extra: Default::default(),
            }],
        );

        let html = page("evil", &evil);
        // No tag the board asked for ever opens: this canvas has no file node,
        // so a real `<img` in the output could only have come from its text.
        assert!(!html.contains("<script"), "{html}");
        assert!(!html.contains("<img"), "{html}");
        assert!(!html.contains("href=\"javascript:"), "{html}");
        // The text survives, escaped, so the page still reads as written.
        assert!(
            html.contains("&lt;script&gt;alert(1)&lt;/script&gt;"),
            "{html}"
        );
        assert!(html.contains("&lt;img onerror=x&gt;"), "{html}");
        // Including the edge label, which is written straight into the body.
        assert!(html.contains("&lt;/svg&gt;"), "{html}");
    }

    #[test]
    fn markdown_becomes_html() {
        let html = page(
            "s",
            &canvas(
                vec![text_node("a", 0, 0, "# Title\n\n- one\n- **two**")],
                vec![],
            ),
        );
        assert!(html.contains("<h1>Title</h1>"), "{html}");
        assert!(html.contains("<strong>two</strong>"), "{html}");
    }

    #[test]
    fn the_board_is_shifted_to_the_page_origin() {
        let html = page(
            "s",
            &canvas(vec![text_node("a", -500, -300, "far from origin")], vec![]),
        );
        // MARGIN in from the top-left corner, wherever the board sat.
        assert!(html.contains("left:40px;top:40px"), "{html}");
        assert!(
            html.contains(r#"width=280""#),
            "200 wide + two margins: {html}"
        );
    }

    #[test]
    fn an_image_file_node_renders_an_img_from_the_spaces_dir() {
        let node = Node {
            kind: NodeKind::File {
                file: "pics/a b.png".to_owned(),
                subpath: None,
            },
            ..text_node("a", 0, 0, "")
        };
        let html = page("s", &canvas(vec![node], vec![]));
        assert!(html.contains(r#"<img src="/f/pics/a%20b.png""#), "{html}");
    }

    #[test]
    fn a_dangling_edge_is_skipped_not_panicked_on() {
        let html = page(
            "s",
            &canvas(
                vec![text_node("a", 0, 0, "only node")],
                vec![Edge {
                    id: "e".to_owned(),
                    from_node: "a".to_owned(),
                    from_side: None,
                    from_end: None,
                    to_node: "gone".to_owned(),
                    to_side: None,
                    to_end: None,
                    label: None,
                    color: None,
                    extra: Default::default(),
                }],
            ),
        );
        assert!(!html.contains("<line"), "{html}");
    }
}
