//! `extd project` / `unproject` — the LLM view. Phase 6.
//!
//! A projection, not a format: fixed 20-unit grid so both directions are
//! stateless, coordinates quantised and snapped, file-node bytes replaced by
//! their caption, nodes in reading order, and `--selection` to scope to a
//! subgraph. Scripts get the real lossless JSON; only `claude` sees this.
