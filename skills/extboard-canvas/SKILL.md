---
name: extboard-canvas
description: Edit an extboard space, a JSONCanvas `.canvas` file, from the console. Use when asked to add, move, connect, recolour or delete cards, nodes, arrows or edges on a board or canvas, or when a `.canvas` file is named or attached. Settle dev or prod first: a local space is edited in place, a remote one goes through `extd pull`/`push`. Carries the format rules, the id convention, `extd project --selection` for a board too big to read whole, and `extd fmt` as the closing gate.
---

# extboard canvas

A space is one `.canvas` file. Which machine holds it is the first thing to
settle, because getting it wrong fails quietly: the file changes and the board
on screen does not.

Every snippet below writes `extd`, which is not on `PATH`: run it as `cargo run
-q -p extd -- <command>` from the repo root, or `cargo install --path
crates/extd` once and have the real binary.

## Dev or prod

**Dev** is a local `extd serve` with its spaces in `~/extboard` (`EXTBOARD_DIR`
overrides). Edit the file with ordinary file tools. The server watches the
directory and broadcasts the write, so every open client reloads on its own:
there is no API call to make and no pipe to run.

**Prod** is a remote `extd`. Its spaces live on that host, so a local edit
reaches nothing — `~/extboard` is a separate copy that drifts from it.

`EXTBOARD_SERVER` is the tell: when it is set it names the server the client
talks to, and the change belongs there. The `extboard-prod` alias sets it, and
so does any mention of prod, the VPS or the server. Only the UI's *images* come
off the local disk on a remote board, never the document. When it is genuinely
unclear which board is meant, ask: the two copies never converge on their own.

### Dev

```sh
cd ~/extboard
# read trip.canvas, edit it, then:
extd fmt trip.canvas
```

### Prod

`pull`, edit, `fmt`, `push`, carrying the rev `pull` printed as `--base`:

```sh
export EXTBOARD_SERVER=https://host    # whatever extboard-prod points at
extd pull /tmp/trip.canvas             # prints: pulled trip @ <rev>
# edit /tmp/trip.canvas
extd fmt /tmp/trip.canvas
extd push /tmp/trip.canvas --base <rev>
```

The space id defaults to the file stem, so `untitled.canvas` is the `untitled`
space; `--space` is only for a scratch file named something else.

`--base` is a compare-and-swap and it is required. A `409` means the board moved
while you were editing: re-pull and redo the edit on the new bytes. Never answer
a `409` by pushing again with a freshly read rev, which is the clobber `--base`
exists to prevent — there is no `--force`.

`pull` writes canonical bytes, so `fmt` straight after one is a no-op and any
diff afterwards is the edit rather than the formatting. `push` validates
locally before it opens a socket, and extd validates again on arrival.

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

`sides` is optional on any node, and makes it a polygon: 3 to 9 for a regular
polygon, 10 for a circle. The polygon fills the node's box, so 4 is exactly the
rectangle a node is by default — leave the key out for that, which is also what
under 3 means. Not in the spec, so Obsidian keeps the key and draws the rect.

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
- Editing `~/extboard` for a change that belongs on prod. Nothing errors; the
  board just never changes.
- Re-reading the rev to get past a `409` instead of pulling and redoing the
  edit. It overwrites whatever moved.
