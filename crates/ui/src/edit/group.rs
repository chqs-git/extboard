use bevy::math::{IRect, IVec2};
use extboard_core::{Canvas, Node, NodeKind};

use super::keys::GROUP_PAD;

const PAD: i32 = GROUP_PAD as i32;

// Each group and what sat wholly inside it when a gesture began, innermost
// first, so a group is refitted before the one holding it measures it.
pub type Members = Vec<(String, Vec<String>)>;

pub fn members(canvas: &Canvas) -> Members {
    let mut groups: Vec<(i64, String, Vec<String>)> = canvas
        .nodes
        .iter()
        .filter(|node| is_group(node))
        .map(|group| {
            let bounds = rect(group);
            let inside = canvas
                .nodes
                .iter()
                .filter(|node| node.id != group.id && holds(bounds, rect(node)))
                .map(|node| node.id.clone())
                .collect();
            (group.width * group.height, group.id.clone(), inside)
        })
        .collect();
    groups.sort_by_key(|(area, ..)| *area);
    groups
        .into_iter()
        .map(|(_, id, inside)| (id, inside))
        .collect()
}

// Every group the gesture touched, wrapped round what it holds plus the pad,
// growing or shrinking. Only those: an untouched group keeps its hand-made room.
pub fn fitted(canvas: &mut Canvas, members: &Members, mut touched: Vec<String>) {
    for (group, inside) in members {
        if !touched.contains(group) && !inside.iter().any(|member| touched.contains(member)) {
            continue;
        }
        touched.push(group.clone());
        // An emptied group stays where it was rather than collapsing to nothing.
        if let Some(want) = inside
            .iter()
            .filter_map(|id| rect_of(canvas, id))
            .map(|member| member.inflate(PAD))
            .reduce(|all, member| all.union(member))
        {
            placed(canvas, group, want);
        }
    }
}

// The innermost group under the node's centre, when it is not in that one yet.
// Groups do not join by dropping: they carry contents a tile cannot.
pub fn target<'a>(canvas: &Canvas, members: &'a Members, id: &str) -> Option<&'a str> {
    let node = canvas
        .nodes
        .iter()
        .find(|node| node.id == id && !is_group(node))?;
    let center = rect(node).center();
    let (group, inside) = members
        .iter()
        .find(|(group, _)| rect_of(canvas, group).is_some_and(|r| r.contains(center)))?;
    (!inside.iter().any(|member| member == id)).then_some(group.as_str())
}

// Out of whatever held it and into `group`, at the open spot nearest the drop.
// It, and every group it left, are what has to be refitted.
pub fn joined(canvas: &mut Canvas, members: &mut Members, id: &str, group: &str) -> Vec<String> {
    let mut touched = vec![id.to_owned()];
    for (left, inside) in members.iter_mut() {
        if inside.iter().any(|member| member == id) {
            inside.retain(|member| member != id);
            touched.push(left.clone());
        }
    }
    let Some((_, inside)) = members.iter_mut().find(|(g, _)| g == group) else {
        return touched;
    };
    let others: Vec<IRect> = inside.iter().filter_map(|m| rect_of(canvas, m)).collect();
    if let Some(at) = rect_of(canvas, id) {
        placed(canvas, id, tiled(at, &others));
    }
    inside.push(id.to_owned());
    touched
}

// The spot nearest the drop that clears every member by the pad. The nearest
// one keeps each axis where it was or butts it against a member's side.
fn tiled(at: IRect, others: &[IRect]) -> IRect {
    let size = at.size();
    let xs: Vec<i32> = std::iter::once(at.min.x)
        .chain(
            others
                .iter()
                .flat_map(|o| [o.max.x + PAD, o.min.x - PAD - size.x]),
        )
        .collect();
    let ys: Vec<i32> = std::iter::once(at.min.y)
        .chain(
            others
                .iter()
                .flat_map(|o| [o.max.y + PAD, o.min.y - PAD - size.y]),
        )
        .collect();
    xs.iter()
        .flat_map(|&x| ys.iter().map(move |&y| IVec2::new(x, y)))
        .map(|min| IRect::from_corners(min, min + size))
        .filter(|spot| {
            others
                .iter()
                .all(|o| spot.inflate(PAD).intersect(*o).is_empty())
        })
        .min_by_key(|spot| (spot.min - at.min).as_i64vec2().length_squared())
        .unwrap_or(at)
}

// The shortest move that clears `wall` by the pad; none if it already does.
fn outside(at: IRect, wall: IRect) -> IVec2 {
    if wall.inflate(PAD).intersect(at).is_empty() {
        return IVec2::ZERO;
    }
    [
        IVec2::new(wall.min.x - PAD - at.max.x, 0),
        IVec2::new(wall.max.x + PAD - at.min.x, 0),
        IVec2::new(0, wall.min.y - PAD - at.max.y),
        IVec2::new(0, wall.max.y + PAD - at.min.y),
    ]
    .into_iter()
    .min_by_key(|shift| shift.abs().element_sum())
    .expect("four sides")
}

pub fn in_group(canvas: &Canvas, id: &str) -> bool {
    members(canvas)
        .iter()
        .any(|(_, inside)| inside.iter().any(|member| member == id))
}

// The shortest way out of what its innermost group shrinks to, contents and
// all. A group around that one still holds it, and refits to keep it.
pub fn ungrouped(canvas: &mut Canvas, id: &str) -> bool {
    let mut members = members(canvas);
    let mut carried: Vec<String> = members
        .iter()
        .find(|(group, _)| group == id)
        .map(|(_, inside)| inside.clone())
        .unwrap_or_default();
    carried.push(id.to_owned());
    let Some((group, inside)) = members
        .iter_mut()
        .find(|(_, inside)| inside.iter().any(|member| member == id))
    else {
        return false;
    };
    inside.retain(|member| !carried.contains(member));
    let left = group.clone();
    let (Some(bounds), Some(at)) = (rect_of(canvas, group), rect_of(canvas, id)) else {
        return false;
    };
    // An emptied group keeps its size, so then it is the group itself to clear.
    let wall = inside
        .iter()
        .filter_map(|member| rect_of(canvas, member))
        .map(|member| member.inflate(PAD))
        .reduce(|all, member| all.union(member))
        .unwrap_or(bounds);
    let shift = outside(at, wall).as_i64vec2();
    for node in canvas.nodes.iter_mut().filter(|n| carried.contains(&n.id)) {
        (node.x, node.y) = (node.x + shift.x, node.y + shift.y);
    }
    carried.push(left);
    fitted(canvas, &members, carried);
    true
}

fn is_group(node: &Node) -> bool {
    matches!(node.kind, NodeKind::Group { .. })
}

// Canvas space, y down. Boards are nowhere near i32's reach.
fn rect(node: &Node) -> IRect {
    IRect::new(
        node.x as i32,
        node.y as i32,
        (node.x + node.width) as i32,
        (node.y + node.height) as i32,
    )
}

fn rect_of(canvas: &Canvas, id: &str) -> Option<IRect> {
    canvas.nodes.iter().find(|node| node.id == id).map(rect)
}

fn placed(canvas: &mut Canvas, id: &str, to: IRect) {
    if let Some(node) = canvas.nodes.iter_mut().find(|node| node.id == id) {
        (node.x, node.y) = (to.min.x.into(), to.min.y.into());
        (node.width, node.height) = (to.width().into(), to.height().into());
    }
}

fn holds(outer: IRect, inner: IRect) -> bool {
    outer.contains(inner.min) && outer.contains(inner.max)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn node(id: &str, x: i64, y: i64, w: i64, h: i64, kind: NodeKind) -> Node {
        Node {
            id: id.into(),
            x,
            y,
            width: w,
            height: h,
            color: None,
            sides: None,
            kind,
            extra: Default::default(),
        }
    }

    fn card(id: &str, x: i64, y: i64) -> Node {
        let text = NodeKind::Text {
            text: String::new(),
        };
        node(id, x, y, 100, 100, text)
    }

    fn board() -> Canvas {
        Canvas {
            nodes: vec![
                node("g", 0, 0, 400, 200, NodeKind::Group { label: None }),
                card("a", 24, 24),
                card("loose", 1000, 0),
            ],
            ..Default::default()
        }
    }

    fn at(canvas: &Canvas, id: &str) -> IRect {
        rect_of(canvas, id).unwrap()
    }

    #[test]
    fn a_group_follows_its_member_and_ignores_a_stranger() {
        let mut canvas = board();
        let before = members(&canvas);
        canvas.nodes[1].x = 600;
        canvas.nodes[2].x = 500;
        fitted(&mut canvas, &before, vec!["a".into(), "loose".into()]);
        assert_eq!(at(&canvas, "g"), IRect::new(576, 0, 724, 148));
        // And back, shrinking round it rather than keeping the room it had.
        canvas.nodes[1].x = 24;
        fitted(&mut canvas, &before, vec!["a".into()]);
        assert_eq!(at(&canvas, "g"), IRect::new(0, 0, 148, 148));
    }

    #[test]
    fn a_group_nothing_touched_keeps_its_room() {
        let mut canvas = board();
        let before = members(&canvas);
        fitted(&mut canvas, &before, vec!["loose".into()]);
        assert_eq!(at(&canvas, "g"), IRect::new(0, 0, 400, 200));
    }

    #[test]
    fn a_node_dropped_on_a_member_slides_off_it_the_shortest_way() {
        let mut canvas = board();
        let mut before = members(&canvas);
        // Over the right half of `a`, and lower than it.
        (canvas.nodes[2].x, canvas.nodes[2].y) = (100, 30);
        let group = target(&canvas, &before, "loose").unwrap().to_owned();
        let touched = joined(&mut canvas, &mut before, "loose", &group);
        assert_eq!(at(&canvas, "loose"), IRect::new(148, 30, 248, 130));
        fitted(&mut canvas, &before, touched);
        assert_eq!(at(&canvas, "g"), IRect::new(0, 0, 272, 154));
        assert_eq!(target(&canvas, &before, "loose"), None, "already a member");
    }

    #[test]
    fn a_dropped_node_on_open_ground_stays_where_it_landed() {
        let others = [IRect::new(0, 0, 100, 100)];
        let at = IRect::new(300, 300, 400, 400);
        assert_eq!(tiled(at, &others), at);
    }

    #[test]
    fn removing_a_node_steps_it_just_clear_of_what_is_left() {
        let mut canvas = board();
        canvas.nodes.push(card("b", 148, 24));
        assert!(ungrouped(&mut canvas, "b"));
        assert_eq!(at(&canvas, "b"), IRect::new(172, 24, 272, 124));
        assert_eq!(at(&canvas, "g"), IRect::new(0, 0, 148, 148));
        assert!(!in_group(&canvas, "b"));
    }

    #[test]
    fn removing_the_last_node_puts_it_outside_and_leaves_the_group_be() {
        let mut canvas = board();
        assert!(in_group(&canvas, "a"));
        assert!(ungrouped(&mut canvas, "a"));
        assert_eq!(at(&canvas, "a"), IRect::new(-124, 24, -24, 124));
        assert!(!in_group(&canvas, "a"));
        assert_eq!(at(&canvas, "g"), IRect::new(0, 0, 400, 200));
        assert!(!ungrouped(&mut canvas, "loose"));
    }
}
