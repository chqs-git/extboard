use crate::model::{Canvas, object_mut};

// Path traversal boundary: an id is one filename component, never a path.
pub fn is_space_id(id: &str) -> bool {
    !id.is_empty() && !id.starts_with('.') && !id.contains(['/', '\\'])
}

// The ids a new top-level space tries in turn; the first one free is its id.
pub fn untitled(n: usize) -> String {
    match n {
        0 | 1 => "untitled".to_owned(),
        n => format!("untitled-{n}"),
    }
}

pub fn nth_name(name: &str, n: u32) -> String {
    match (n, name.rsplit_once('.')) {
        (0, _) => name.to_owned(),
        (n, Some((stem, ext))) => format!("{stem}-{n}.{ext}"),
        (n, None) => format!("{name}-{n}"),
    }
}

// `<parent>_<name>`, so two parents can each have a `notes`.
pub fn inner_id(parent: &str, name: &str) -> String {
    match parent {
        "" => name.to_owned(),
        parent => format!("{parent}_{name}"),
    }
}

// An empty canvas inside `parent`, starting out in its theme and saved themes.
pub fn child_of(parent_id: &str, parent: &Canvas) -> Canvas {
    let mut canvas = Canvas::default();
    if let Some(theme) = parent.extra.get("theme") {
        canvas.extra.insert("theme".into(), theme.clone());
    }
    if let Some(themes) = parent
        .extra
        .get("extboard")
        .and_then(|extboard| extboard.get("themes"))
    {
        object_mut(&mut canvas.extra, "extboard").insert("themes".into(), themes.clone());
    }
    canvas.set_parent(parent_id);
    canvas
}

// A space made inside `from` follows it to `to`; `None` when it was not made there.
// A link to `from` elsewhere is left dangling.
pub fn reparented(mut canvas: Canvas, from: &str, to: &str) -> Option<Canvas> {
    if canvas.parent() != Some(from) {
        return None;
    }
    canvas.set_parent(to);
    Some(canvas)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_id_is_one_filename_and_never_a_path() {
        assert!(is_space_id("untitled_notes"));
        for bad in ["", ".hidden", "..", "a/b", "a\\b"] {
            assert!(!is_space_id(bad), "{bad}");
        }
    }

    #[test]
    fn new_ids_count_up_and_inner_ones_are_prefixed() {
        assert_eq!(
            (1..=3).map(untitled).collect::<Vec<_>>(),
            ["untitled", "untitled-2", "untitled-3"]
        );
        assert_eq!(inner_id("", "notes"), "notes");
        assert_eq!(
            (0..3).map(|n| nth_name("a.b.jpg", n)).collect::<Vec<_>>(),
            ["a.b.jpg", "a.b-1.jpg", "a.b-2.jpg"]
        );
        assert_eq!(nth_name("README", 2), "README-2");
        assert_eq!(inner_id("untitled-2", "notes"), "untitled-2_notes");
    }

    #[test]
    fn a_child_starts_in_its_parents_themes_and_nothing_else() {
        let parent: Canvas = serde_json::from_str(
            r#"{"nodes":[{"id":"n1","type":"text","x":0,"y":0,"width":9,"height":9,"text":"hi"}],
                "edges":[],"theme":"dusk",
                "extboard":{"themes":[{"name":"dusk"}],"vars":{"a":1}}}"#,
        )
        .expect("fixture");
        let child = child_of("trip", &parent);
        assert!(child.nodes.is_empty());
        assert_eq!(child.parent(), Some("trip"));
        assert_eq!(child.extra["theme"], "dusk");
        assert_eq!(
            child.extra["extboard"]["themes"],
            parent.extra["extboard"]["themes"]
        );
        assert!(child.extra["extboard"].get("vars").is_none());
    }

    #[test]
    fn a_rename_moves_only_the_spaces_made_inside_it() {
        let mut inner = Canvas::default();
        inner.set_parent("a");
        let moved = reparented(inner.clone(), "a", "b").expect("made inside a");
        assert_eq!(moved.parent(), Some("b"));
        assert!(reparented(inner, "c", "d").is_none());
        assert!(reparented(Canvas::default(), "a", "b").is_none());
    }
}
