---
name: extboard-canvas
description: Edit an extboard space, a JSONCanvas `.canvas` file, from the console. Use when asked to add, move, connect, recolour or delete cards, nodes, arrows or edges on a board or canvas, or when a `.canvas` file is named or attached. Carries the format rules, the id convention, `extd project --selection` for a board too big to read whole, and `extd fmt` as the closing gate.
---

# extboard canvas

A space is one `.canvas` file in `~/extboard` (`EXTBOARD_DIR` overrides). Edit
it with ordinary file tools. The server watches the directory and broadcasts the
write, so every open client reloads on its own: there is no API call to make and
no pipe to run.

```sh
cd ~/extboard
# read trip.canvas, edit it, then:
extd fmt trip.canvas
```

`extd fmt` is the last step of every edit, never optional. It validates, then
rewrites the file in the canonical form. A dangling edge or a duplicate id fails
there, names what is wrong, and leaves the file untouched, which is the whole
reason it runs before you report done.

## The format

Top level is `{"nodes": [...], "edges": [...]}`. Any other top-level key is the
board's own and survives a round trip: `extboard.scripts` holds the Rhai
handlers, so never rewrite or drop it.

Every node has `id`, `x`, `y`, `width`, `height`, a `type`, and the fields that
type needs:

| `type` | its field |
|---|---|
| `text` | `text`, markdown |
| `file` | `file`, a path in the spaces dir, plus optional `subpath` starting `#` |
| `link` | `url` |
| `group` | `label`, optional |

`color` is optional on a node: `"1"` to `"6"` for the presets, or `"#rrggbb"`.
Anything else draws as no colour at all, so a typo is a silent no-op.

An edge has `id`, `fromNode`, `toNode`, and optionally `fromSide`/`toSide`
(`top`, `right`, `bottom`, `left`), `fromEnd`/`toEnd` (`none`, `arrow`) and
`label`. Leave the sides out and the board picks the pair that face each other,
which is usually what you want. Both endpoints must name a node that exists.

## Ids

16 lowercase hex characters, and unique across nodes *and* edges in one file,
because they share a namespace. Any string works, but matching the convention
keeps a hand-made node indistinguishable from a board-made one.

```sh
openssl rand -hex 8
```

Never reuse an id, and never invent one for a node you did not add: an edge
pointing at an id that is not in the file is the error `extd fmt` exists to
catch.

## Geometry

Scene units, and **`+y` is down**: a node below another has a larger `y`. A node
"below Day 2" sits at Day 2's `x`, with `y` past the bottom of Day 2's box.

Read the neighbours before placing anything. Match their `width` and `height`
rather than inventing a size, and reuse the gap they already have between them
instead of picking a new one. Two cards 60 tall with 40 between them want the
third at `y + 100`, not at a round number.

## A board too big to read whole

`extd project` prints a reduced view of the document: one line per node, a fixed
20 unit grid, scripts replaced by a placeholder. It costs about a third of the
raw JSON and reads in a terminal.

```sh
extd project trip.canvas                     # the whole board
extd project trip.canvas --selection d1,d2   # those nodes, plus one hop
```

`--selection` is the lever worth reaching for: it emits the named nodes, the
edges touching them and the node at each edge's far end, which is all most
prompts are about.

Edit the projected text and hand it back:

```sh
extd project trip.canvas --selection d1,d2 > /tmp/p
# edit /tmp/p
extd unproject trip.canvas < /tmp/p
extd fmt trip.canvas
```

The header line carries the scope, so a selection merges into the full document
and the other nodes are not disturbed. Inside the scope, omitting a node or an
edge deletes it; a neighbour shown for context is never in the scope, so leaving
it out changes nothing.

Coordinates come back quantised to the 20 unit grid, deliberately. For an edit
where the exact pixel matters, edit the `.canvas` directly instead.

## Mistakes this format invites

- Writing an edge before the node it points at. `extd fmt` rejects the file, by
  edge id and node id.
- Dropping `extboard.scripts` by rebuilding the file from the nodes you read.
- Placing a node "above" with a larger `y`.
- A `color` of `"red"` or `"#f00"`. Neither is a canvas colour.
- Reporting done without running `extd fmt`.
