use ehttp::streaming::Part;
use ehttp::{Request, Response};
use extboard_core::{Canvas, url_path};
use std::fmt::Display;
use std::ops::ControlFlow;
use std::sync::{Mutex, PoisonError};

use super::{Backend, Event, Loaded, Reply, Saved};
use crate::client::base_url;
use crate::lock::keyed;

type Answer = ehttp::Result<Response>;

// extd, at `base_url`.
pub struct Http;

impl Backend for Http {
    fn get(&self, space: &str, rev: &str, reply: Reply<Result<Loaded, String>>) {
        let mut request = keyed(Request::get(space_url(space)));
        if !rev.is_empty() {
            request
                .headers
                .insert("If-None-Match", format!("\"{rev}\""));
        }
        let space = space.to_owned();
        ehttp::fetch(request, move |answer| reply(loaded(&space, answer)));
    }

    fn subscribe(&self, on: Box<dyn Fn(Event) + Send>) {
        let mut request = Request::get(format!("{}/api/events", base_url()));
        request.timeout = None; // the stream is meant to stay open

        let buffer = Mutex::new(String::new());
        ehttp::streaming::fetch(request, move |part| {
            let chunk = match part {
                Ok(Part::Response(response)) if response.ok => return ControlFlow::Continue(()),
                Ok(Part::Chunk(chunk)) if !chunk.is_empty() => chunk,
                // An empty chunk is the end of the stream; the rest is extd gone.
                Ok(_) | Err(_) => {
                    on(Event::Disconnected);
                    return ControlFlow::Break(());
                }
            };

            let mut buffer = buffer.lock().unwrap_or_else(PoisonError::into_inner);
            buffer.push_str(&String::from_utf8_lossy(&chunk));
            for frame in drain_frames(&mut buffer) {
                if let Some((space, rev)) = changed_rev(&frame) {
                    on(Event::Changed(space, rev));
                }
            }
            ControlFlow::Continue(())
        });
    }

    fn put(&self, space: &str, canvas: &Canvas, base: &str, reply: Reply<Result<Saved, String>>) {
        ehttp::fetch(keyed(put(canvas, base, space)), move |answer| {
            reply(saved(answer));
        });
    }

    fn list(&self, reply: Reply<Result<Vec<String>, String>>) {
        let request = Request::get(format!("{}/api/spaces", base_url()));
        ehttp::fetch(request, move |answer| reply(listed(answer)));
    }

    fn path(&self, space: &str, reply: Reply<Option<Vec<String>>>) {
        let request = keyed(Request::get(format!("{}/path", space_url(space))));
        ehttp::fetch(request, move |answer| {
            reply(
                answer
                    .ok()
                    .filter(|response| response.ok)
                    .and_then(|response| serde_json::from_slice(&response.bytes).ok()),
            );
        });
    }

    // extd names a new space: the list does not show the inner ones, so only
    // it knows which `untitled` is free.
    fn create(&self, reply: Reply<Result<String, String>>) {
        let request = Request::post(format!("{}/api/spaces", base_url()), Vec::new());
        ehttp::fetch(request, move |answer| reply(created(answer)));
    }

    // extd makes the id, `<parent>_<name>`, and answers with it.
    fn create_inner(&self, parent: &str, name: &str, reply: Reply<Result<String, String>>) {
        let request = keyed(Request::post(space_url(name), parent.into()));
        let name = name.to_owned();
        ehttp::fetch(request, move |answer| reply(made_inner(&name, answer)));
    }

    fn rename(&self, from: &str, to: &str, reply: Reply<Result<(), String>>) {
        let request = keyed(Request::post(
            format!("{}/rename", space_url(from)),
            to.into(),
        ));
        let to = to.to_owned();
        ehttp::fetch(request, move |answer| reply(renamed(&to, answer)));
    }

    fn read(&self, file: &str, reply: Reply<Result<Vec<u8>, String>>) {
        let request = Request::get(format!("{}/f/{}", base_url(), url_path(file)));
        ehttp::fetch(request, move |answer| {
            reply(match answer {
                Err(e) => Err(e),
                Ok(response) if !response.ok => Err(format!("extd returned {}", response.status)),
                Ok(response) => Ok(response.bytes),
            });
        });
    }

    fn upload(&self, name: &str, bytes: Vec<u8>, reply: Reply<Result<String, String>>) {
        let request = Request::post(
            format!("{}/api/files/{}", base_url(), url_path(name)),
            bytes,
        );
        ehttp::fetch(request, move |answer| {
            reply(match answer {
                Ok(response) if response.ok => {
                    Ok(String::from_utf8_lossy(&response.bytes).into_owned())
                }
                Ok(response) => Err(format!("extd refused the upload: {}", response.status)),
                Err(e) => Err(format!("upload failed: {e}")),
            });
        });
    }
}

fn space_url(space: &str) -> String {
    format!("{}/api/spaces/{space}", base_url())
}

fn unreachable(e: impl Display) -> String {
    format!("extd unreachable at {}: {e}", base_url())
}

fn etag_rev(response: &Response) -> String {
    response
        .headers
        .get("etag")
        .unwrap_or_default()
        .trim_matches('"')
        .to_owned()
}

fn loaded(space: &str, answer: Answer) -> Result<Loaded, String> {
    // A dead server is a transport error; a live one can still answer 404/500,
    // which ehttp reports as Ok. Both have to read as a failure.
    let response = answer.map_err(unreachable)?;
    if response.status == 304 {
        return Ok(Loaded::Unchanged);
    }
    if response.status == 401 {
        let reason = serde_json::from_slice::<serde_json::Value>(&response.bytes)
            .ok()
            .and_then(|body| Some(body.get("error")?.as_str()?.to_owned()))
            .unwrap_or_else(|| format!("{space} is locked"));
        return Ok(Loaded::Locked(reason));
    }
    if !response.ok {
        return Err(format!("extd returned {}", response.status));
    }
    serde_json::from_slice(&response.bytes)
        .map(|canvas| Loaded::Canvas(canvas, etag_rev(&response)))
        .map_err(|e| format!("bad canvas from extd: {e}"))
}

// The headers are built, not amended: `Headers::insert` appends, and extd reads
// the first Content-Type, which on a `put` is ehttp's own text/plain.
fn put(canvas: &Canvas, base: &str, space: &str) -> Request {
    let if_match = format!("\"{base}\"");
    Request {
        headers: ehttp::Headers::new(&[
            ("Accept", "*/*"),
            ("Content-Type", "application/json"),
            ("If-Match", &if_match),
        ]),
        ..Request::put(space_url(space), canvas.to_pretty_string().into_bytes())
    }
}

fn saved(answer: Answer) -> Result<Saved, String> {
    let response = answer.map_err(|e| format!("save failed: {e}"))?;
    let rev = etag_rev(&response);
    match response.status {
        200 | 204 => Ok(Saved::Saved(rev)),
        409 => serde_json::from_slice(&response.bytes)
            .map(|canvas| Saved::Conflict(canvas, rev))
            .map_err(|e| format!("bad canvas from extd: {e}")),
        status => Err(format!(
            "extd refused the save: {status} {}",
            String::from_utf8_lossy(&response.bytes)
        )),
    }
}

fn listed(answer: Answer) -> Result<Vec<String>, String> {
    let response = answer.map_err(unreachable)?;
    if !response.ok {
        return Err(format!("extd returned {}", response.status));
    }
    serde_json::from_slice(&response.bytes).map_err(|e| format!("bad space list from extd: {e}"))
}

fn created(answer: Answer) -> Result<String, String> {
    match answer {
        Err(e) => Err(unreachable(e)),
        Ok(response) if response.status == 201 => String::from_utf8(response.bytes)
            .map_err(|_| "extd named the new space in something other than UTF-8".to_owned()),
        Ok(response) => Err(format!("extd refused a new space: {}", response.status)),
    }
}

fn made_inner(name: &str, answer: Answer) -> Result<String, String> {
    match &answer {
        Ok(response) if response.status == 409 => Err(format!("{name} already exists here")),
        Ok(response) if response.status == 400 => Err(format!("{name} is not a usable name")),
        _ => created(answer),
    }
}

fn renamed(to: &str, answer: Answer) -> Result<(), String> {
    match answer {
        Err(e) => Err(unreachable(e)),
        Ok(response) if response.ok => Ok(()),
        Ok(response) if response.status == 409 => Err(format!("{to} already exists")),
        Ok(response) if response.status == 400 => Err(format!("{to} is not a usable name")),
        Ok(response) => Err(format!("extd refused the rename: {}", response.status)),
    }
}

fn drain_frames(buffer: &mut String) -> Vec<String> {
    let mut frames = Vec::new();
    while let Some(end) = buffer.find("\n\n") {
        frames.push(buffer[..end].to_owned());
        buffer.replace_range(..end + 2, "");
    }
    frames
}

// `(space, rev)`. A lagged stream names neither, and an empty rev is one no
// client can be holding, so it reads as a change to whatever is open.
fn changed_rev(frame: &str) -> Option<(String, String)> {
    let mut changed = false;
    let mut data = None;
    for line in frame.lines() {
        if let Some(name) = line.strip_prefix("event:") {
            changed = name.trim() == "changed";
        }
        if let Some(payload) = line.strip_prefix("data:") {
            data = Some(payload.trim().to_owned());
        }
    }
    if !changed {
        return None;
    }

    let data: serde_json::Value = serde_json::from_str(&data?).ok()?;
    match (data.get("space"), data.get("rev")) {
        (Some(space), Some(rev)) => Some((space.as_str()?.to_owned(), rev.as_str()?.to_owned())),
        (None, None) => Some((String::new(), String::new())),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn response(status: u16, body: &str) -> Answer {
        Ok(Response {
            url: String::new(),
            ok: (200..300).contains(&status),
            status,
            status_text: String::new(),
            headers: ehttp::Headers::new(&[("etag", "\"r1\"")]),
            bytes: body.as_bytes().to_vec(),
        })
    }

    #[test]
    fn every_failure_carries_a_message() {
        // Server down: this is the case that used to panic off-thread.
        assert!(loaded("a", Err("connection refused".to_owned())).is_err());
        // Server up, space missing: ehttp calls this Ok.
        assert!(loaded("a", response(404, "not found")).is_err());
        assert!(loaded("a", response(200, "<html>")).is_err());
    }

    #[test]
    fn a_load_carries_the_etags_rev_and_304_carries_nothing() {
        let got = loaded("a", response(200, r#"{"nodes":[],"edges":[]}"#));
        assert!(
            matches!(&got, Ok(Loaded::Canvas(_, rev)) if rev == "r1"),
            "{got:?}"
        );
        assert!(matches!(
            loaded("a", response(304, "")),
            Ok(Loaded::Unchanged)
        ));
        let locked = loaded("a", response(401, r#"{"error":"wrong password for a"}"#));
        assert!(
            matches!(&locked, Ok(Loaded::Locked(why)) if why == "wrong password for a"),
            "{locked:?}"
        );
    }

    // 415 from extd is what a second Content-Type looks like from the outside.
    #[test]
    fn a_save_is_one_json_content_type_and_the_rev_it_is_based_on() {
        let canvas: Canvas = serde_json::from_str(r#"{"nodes":[],"edges":[]}"#).unwrap();
        let got = put(&canvas, "r1", "a");
        assert_eq!(
            got.headers.get_all("content-type").collect::<Vec<_>>(),
            ["application/json"]
        );
        assert_eq!(got.headers.get("if-match"), Some("\"r1\""));
        assert_eq!(got.method, ehttp::Method::PUT);
    }

    #[test]
    fn a_reply_is_the_new_rev_a_conflict_or_a_refusal() {
        assert!(matches!(
            saved(response(204, "")),
            Ok(Saved::Saved(rev)) if rev == "r1"
        ));
        let conflict = saved(response(409, r#"{"nodes":[],"edges":[]}"#));
        assert!(
            matches!(&conflict, Ok(Saved::Conflict(canvas, rev)) if canvas.nodes.is_empty() && rev == "r1"),
            "{conflict:?}"
        );
        // A validation refusal carries what was wrong, and is not a conflict.
        assert!(matches!(
            saved(response(422, r#"{"errors":["edge e1: node nope does not exist"]}"#)),
            Err(message) if message.contains("does not exist")
        ));
        assert!(saved(Err("connection refused".to_owned())).is_err());
    }

    #[test]
    fn a_list_that_is_not_a_list_is_a_notice_not_a_panic() {
        assert_eq!(
            listed(response(200, r#"["a","b"]"#)),
            Ok(vec!["a".to_owned(), "b".to_owned()])
        );
        assert!(listed(response(200, "<html>")).is_err());
        assert!(listed(response(500, "")).is_err());
        assert!(listed(Err("connection refused".to_owned())).is_err());
        // The server names the space, in the body.
        assert_eq!(
            created(response(201, "untitled-2")),
            Ok("untitled-2".to_owned())
        );
        assert!(created(response(404, "")).is_err());
        assert_eq!(
            made_inner("b", response(409, "")),
            Err("b already exists here".to_owned())
        );
        assert_eq!(renamed("b", response(204, "")), Ok(()));
        assert_eq!(
            renamed("b", response(409, "")),
            Err("b already exists".to_owned())
        );
    }

    #[test]
    fn frames_survive_being_split_across_chunks() {
        let mut buffer = String::new();
        buffer.push_str(":ping\n\nevent: changed\ndata: {\"space\":\"a\",\"re");
        assert_eq!(drain_frames(&mut buffer), [":ping"]);

        buffer.push_str("v\":\"r2\"}\n\n");
        let frames = drain_frames(&mut buffer);
        assert_eq!(frames.len(), 1);
        assert_eq!(
            changed_rev(&frames[0]),
            Some(("a".to_owned(), "r2".to_owned()))
        );
        assert!(buffer.is_empty());
    }

    #[test]
    fn a_change_carries_its_space() {
        let frame = |data: &str| format!("event: changed\ndata: {data}");
        assert_eq!(
            changed_rev(&frame(r#"{"space":"b","rev":"r2"}"#)),
            Some(("b".to_owned(), "r2".to_owned()))
        );
        assert_eq!(changed_rev(":ping"), None);
        assert_eq!(changed_rev("event: hello\ndata: {}"), None);
        // Lagged: no space and no rev to compare, so it is a change to ours.
        assert_eq!(
            changed_rev(&frame("{}")),
            Some((String::new(), String::new()))
        );
    }
}
