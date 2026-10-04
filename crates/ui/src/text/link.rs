use bevy::ecs::system::SystemParam;
use bevy::prelude::*;
use bevy::text::TextLayoutInfo;
use bevy::ui::{ComputedNode, UiGlobalTransform};

#[derive(Component)]
pub(super) struct Doors {
    node: String,
    spans: Vec<(usize, String)>,
}

impl Doors {
    pub(super) fn new(node: &str, spans: Vec<(usize, String)>) -> Option<Self> {
        (!spans.is_empty()).then(|| Self {
            node: node.to_owned(),
            spans,
        })
    }

    fn space(&self, span: usize) -> Option<&str> {
        self.spans
            .iter()
            .find(|(at, _)| *at == span)
            .map(|(_, space)| space.as_str())
    }
}

#[derive(SystemParam)]
pub struct Links<'w, 's> {
    lines: Query<
        'w,
        's,
        (
            &'static Doors,
            &'static ComputedNode,
            &'static UiGlobalTransform,
            &'static TextLayoutInfo,
        ),
    >,
}

impl Links<'_, '_> {
    pub fn under(&self, node: &str, cursor: Vec2) -> Option<String> {
        self.lines
            .iter()
            .filter(|(doors, ..)| doors.node == node)
            .find_map(|(doors, computed, transform, layout)| {
                let point = laid_out(computed, transform, cursor)?;
                doors.space(span_at(layout, point)?).map(str::to_owned)
            })
    }
}

// Runs are measured from the content box, where bevy's own renderer hangs them.
fn laid_out(node: &ComputedNode, transform: &UiGlobalTransform, cursor: Vec2) -> Option<Vec2> {
    let inverse = transform.try_inverse()?;
    // The pointer is in logical pixels and a laid-out node in physical ones.
    let at = inverse.transform_point2(cursor / node.inverse_scale_factor());
    Some(at - node.content_box().min)
}

fn span_at(layout: &TextLayoutInfo, point: Vec2) -> Option<usize> {
    layout
        .run_geometry
        .iter()
        .find(|run| run.bounds.contains(point))
        .map(|run| run.section_index)
}

#[cfg(test)]
mod tests {
    use super::*;
    use bevy::text::RunGeometry;

    fn run(section_index: usize, min: Vec2, max: Vec2) -> RunGeometry {
        RunGeometry {
            section_index,
            bounds: Rect::from_corners(min, max),
            strikethrough_y: 0.0,
            strikethrough_thickness: 0.0,
            underline_y: 0.0,
            underline_thickness: 0.0,
        }
    }

    fn line() -> TextLayoutInfo {
        TextLayoutInfo {
            run_geometry: vec![
                run(0, Vec2::new(0.0, 0.0), Vec2::new(40.0, 18.0)),
                run(1, Vec2::new(40.0, 0.0), Vec2::new(70.0, 18.0)),
                run(2, Vec2::new(70.0, 0.0), Vec2::new(110.0, 18.0)),
            ],
            ..default()
        }
    }

    #[test]
    fn a_point_lands_in_the_span_whose_run_covers_it() {
        let layout = line();
        assert_eq!(span_at(&layout, Vec2::new(10.0, 9.0)), Some(0));
        assert_eq!(span_at(&layout, Vec2::new(55.0, 9.0)), Some(1));
        assert_eq!(span_at(&layout, Vec2::new(90.0, 9.0)), Some(2));
        assert_eq!(span_at(&layout, Vec2::new(200.0, 9.0)), None);
        assert_eq!(span_at(&layout, Vec2::new(55.0, 40.0)), None);
    }

    #[test]
    fn only_the_linked_span_is_a_door() {
        let doors = Doors::new("n7", vec![(1, "trip".to_owned())]).expect("a door");
        let span = |point| span_at(&line(), point).and_then(|span| doors.space(span));
        assert_eq!(span(Vec2::new(55.0, 9.0)), Some("trip"));
        assert_eq!(span(Vec2::new(10.0, 9.0)), None);
        assert_eq!(span(Vec2::new(90.0, 9.0)), None);
        assert_eq!(span(Vec2::new(200.0, 9.0)), None);
    }

    #[test]
    fn a_line_without_a_link_is_not_a_door() {
        assert!(Doors::new("n7", Vec::new()).is_none());
    }
}
