//! `GET /api/commands` · `POST /api/commands/:name`. Phase 5.
//!
//! Every command is an executable in `~/extboard/commands/` that reads the
//! document as JSON on stdin and writes the new one to stdout. Spawn, pipe,
//! validate, bump rev, broadcast. Nonzero exit or unparseable stdout shows
//! stderr and changes nothing.
