use bevy::prelude::*;
use bevy::time::common_conditions::on_timer;
use extboard_core::{Canvas, rev};
use std::sync::{Arc, Mutex, PoisonError};
use std::time::Duration;

use crate::client::{Document, Notice, Rev, etag_rev, space_url};

// A save is the whole document, so the only thing that makes it cheap is not
// making many: half a second of quiet, however the document was changed.
const IDLE_SECS: f32 = 0.5;
const CHECK: Duration = Duration::from_millis(100);
// A dropped tailnet is not a reason to hammer the server.
const BACKOFF_MIN: f32 = 1.0;
const BACKOFF_MAX: f32 = 30.0;
const DROPPED: &str = "the file changed on disk: your last edit was dropped";

pub struct SyncPlugin;

#[derive(Debug)]
enum Reply {
    Saved(String),
    Conflict(Canvas, String),
    Failed(String),
}

type Inbox = Arc<Mutex<Vec<Reply>>>;

// A change no human made. Every client computes a timer's value from the same
// slot, so saving it would be N clients racing to write what they already
// agree on -- and the loser of that race is whoever is mid-edit, because an
// edit in progress is the one save being held back by the debounce. So a
// document that departs from the server's by tick writes alone is not saved,
// and the value lives in the open clients rather than in the file.
#[derive(Resource, Default)]
pub struct Ticked {
    // The rev before the first tick write that is still unsaved.
    clean: String,
    // The rev after the last one, so an edit on top of it is detectable.
    after: String,
}

impl Ticked {
    // Called by `script::tick` with the revs either side of its write.
    pub fn wrote(&mut self, before: &str, after: &str) {
        // A write that did not land on the last one starts the run again: the
        // document moved in between, and that move is somebody's edit.
        if self.after != before {
            self.clean = before.to_owned();
        }
        self.after = after.to_owned();
    }

    // `true` when the document differs from the server by tick writes alone.
    fn only(&self, local: &str, base: &str) -> bool {
        !self.after.is_empty() && self.after == local && self.clean == base
    }
}

#[derive(Resource)]
struct Save {
    inbox: Inbox,
    // The local rev at the last check, and when it last differed from the one
    // before it: a gesture is a run of changes, and this is where it went quiet.
    seen: String,
    settled: f32,
    in_flight: bool,
    retry_at: f32,
    backoff: f32,
}

impl Default for Save {
    fn default() -> Self {
        Self {
            inbox: Inbox::default(),
            seen: String::new(),
            settled: 0.0,
            in_flight: false,
            retry_at: 0.0,
            backoff: BACKOFF_MIN,
        }
    }
}

impl Plugin for SyncPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<Save>()
            .init_resource::<Ticked>()
            .add_systems(
                Update,
                (adopt, send.run_if(on_timer(CHECK)))
                    .chain()
                    .run_if(resource_exists::<Document>),
            );
    }
}

impl Save {
    // The one decision: a change that has stopped moving, with nothing in the
    // air and no backoff left to wait out. Hashing the document beats a dirty
    // flag every mutation site has to remember to set, and a drag that bypasses
    // change detection is caught all the same.
    fn due(&mut self, now: f32, local: &str, base: &str) -> bool {
        if self.seen != local {
            self.seen = local.to_owned();
            self.settled = now;
            return false;
        }
        !self.in_flight && local != base && now >= self.retry_at && now - self.settled >= IDLE_SECS
    }
}

fn send(
    time: Res<Time>,
    document: Res<Document>,
    base: Res<Rev>,
    ticked: Res<Ticked>,
    mut save: ResMut<Save>,
) {
    // Without a rev there is nothing to compare and swap against, so the first
    // load has to land before the first save.
    if base.0.is_empty() {
        return;
    }
    let local = rev(&document.0);
    // Checked before `due`, so a clock ticking away never counts as a gesture
    // that has settled and never holds up the save of a real edit.
    if ticked.only(&local, &base.0) {
        return;
    }
    if !save.due(time.elapsed_secs(), &local, &base.0) {
        return;
    }
    save.in_flight = true;
    put(&save.inbox, &document.0, &base.0);
}

fn adopt(
    time: Res<Time>,
    mut save: ResMut<Save>,
    mut document: ResMut<Document>,
    mut base: ResMut<Rev>,
    mut notice: ResMut<Notice>,
) {
    let batch = std::mem::take(&mut *lock(&save.inbox));
    for reply in batch {
        save.in_flight = false;
        match reply {
            // The body is ours; only the rev is news. Adopting it is also what
            // makes the SSE echo of this save recognisable as our own.
            Reply::Saved(rev) => {
                base.0 = rev;
                save.backoff = BACKOFF_MIN;
                clear(&mut notice);
            }
            // Whole-document saves cannot merge, so the server wins and the
            // dropped edit is said out loud rather than replayed. A handler's
            // write is no different: the lambda already ran, and re-running it
            // would double whatever else it did, so the click is lost too.
            Reply::Conflict(canvas, etag) => {
                let lost = dropped(&document.0, &canvas);
                document.0 = canvas;
                base.0 = etag;
                save.backoff = BACKOFF_MIN;
                if lost {
                    notice.0 = Some(DROPPED.to_owned());
                } else {
                    clear(&mut notice);
                }
            }
            Reply::Failed(message) => {
                error!("{message}");
                save.retry_at = time.elapsed_secs() + save.backoff;
                save.backoff = (save.backoff * 2.0).min(BACKOFF_MAX);
                notice.0 = Some(message);
            }
        }
    }
}

// A timer fires in every open client at once, so the clients that lose the race
// get a 409 carrying the document they already hold. Nothing was dropped, and
// saying so every minute would be the clock crying wolf.
fn dropped(ours: &Canvas, theirs: &Canvas) -> bool {
    rev(ours) != rev(theirs)
}

fn clear(notice: &mut ResMut<Notice>) {
    if notice.0.is_some() {
        notice.0 = None;
    }
}

fn put(inbox: &Inbox, canvas: &Canvas, base: &str) {
    let inbox = inbox.clone();
    ehttp::fetch(request(canvas, base), move |result| {
        lock(&inbox).push(replied(result));
    });
}

// The headers are built, not amended: `Headers::insert` appends, and extd reads
// the first Content-Type, which on a `put` is ehttp's own text/plain.
fn request(canvas: &Canvas, base: &str) -> ehttp::Request {
    let if_match = format!("\"{base}\"");
    ehttp::Request {
        headers: ehttp::Headers::new(&[
            ("Accept", "*/*"),
            ("Content-Type", "application/json"),
            ("If-Match", &if_match),
        ]),
        ..ehttp::Request::put(space_url(), canvas.to_pretty_string().into_bytes())
    }
}

fn replied(result: ehttp::Result<ehttp::Response>) -> Reply {
    let response = match result {
        Ok(response) => response,
        Err(e) => return Reply::Failed(format!("save failed: {e}")),
    };
    let rev = etag_rev(&response);
    match response.status {
        200 | 204 => Reply::Saved(rev),
        409 => match serde_json::from_slice(&response.bytes) {
            Ok(canvas) => Reply::Conflict(canvas, rev),
            Err(e) => Reply::Failed(format!("bad canvas from extd: {e}")),
        },
        status => Reply::Failed(format!(
            "extd refused the save: {status} {}",
            String::from_utf8_lossy(&response.bytes)
        )),
    }
}

fn lock(inbox: &Inbox) -> std::sync::MutexGuard<'_, Vec<Reply>> {
    inbox.lock().unwrap_or_else(PoisonError::into_inner)
}

#[cfg(test)]
mod tests {
    use super::*;

    // One tick of the check timer, in the units `due` works in.
    const TICK: f32 = 0.1;

    fn response(status: u16, body: &str) -> ehttp::Response {
        ehttp::Response {
            url: String::new(),
            ok: (200..300).contains(&status),
            status,
            status_text: String::new(),
            headers: ehttp::Headers::new(&[("etag", "\"r2\"")]),
            bytes: body.as_bytes().to_vec(),
        }
    }

    // 415 from extd is what a second Content-Type looks like from the outside.
    #[test]
    fn a_save_is_one_json_content_type_and_the_rev_it_is_based_on() {
        let canvas: Canvas = serde_json::from_str(r#"{"nodes":[],"edges":[]}"#).unwrap();
        let got = request(&canvas, "r1");
        assert_eq!(
            got.headers.get_all("content-type").collect::<Vec<_>>(),
            ["application/json"]
        );
        assert_eq!(got.headers.get("if-match"), Some("\"r1\""));
        assert_eq!(got.method, ehttp::Method::PUT);
    }

    // The ticket's done-when: ten seconds of dragging is one PUT, not six hundred.
    #[test]
    fn a_long_gesture_saves_once_when_it_stops() {
        let mut save = Save::default();
        let mut now = 0.0;
        let mut saves = 0;

        for frame in 0..100 {
            now += TICK;
            // Every check sees a different document: the drag is still moving.
            if save.due(now, &format!("drag{frame}"), "base") {
                saves += 1;
            }
        }
        assert_eq!(saves, 0, "saved mid-gesture");

        // Let go. The first quiet check only records the rev as settled.
        for _ in 0..8 {
            now += TICK;
            if save.due(now, "dragged", "base") {
                saves += 1;
                save.in_flight = true;
            }
        }
        assert_eq!(saves, 1, "{now}s of quiet produced {saves} saves");
    }

    #[test]
    fn an_edit_while_a_save_flies_waits_for_the_reply() {
        let mut save = Save::default();
        save.due(0.0, "second", "first");
        save.in_flight = true;

        // Single-flight: due in every other respect, and still nothing goes out.
        assert!(!save.due(1.0, "second", "first"));

        // The reply lands. Its rev is the one we sent, not the one we now hold,
        // so the next quiet check schedules the follow-up save.
        save.in_flight = false;
        assert!(!save.due(1.1, "third", "second"), "a fresh edit debounces");
        assert!(save.due(1.7, "third", "second"));
    }

    #[test]
    fn a_failure_backs_off_and_a_save_stays_due_until_it_lands() {
        let mut save = Save::default();
        save.due(0.0, "local", "base");
        assert!(save.due(1.0, "local", "base"));

        // The PUT failed at t=1: nothing goes out again until the backoff is up.
        save.retry_at = 1.0 + BACKOFF_MIN;
        assert!(!save.due(1.5, "local", "base"));
        assert!(save.due(2.5, "local", "base"));
    }

    #[test]
    fn nothing_is_due_when_the_server_holds_what_we_hold() {
        let mut save = Save::default();
        save.due(0.0, "same", "same");
        assert!(!save.due(9.0, "same", "same"));
    }

    // A timer's write stays in the open clients: the file is not where a value
    // every client can compute for itself belongs, and saving it is what put
    // the person mid-edit on the losing side of a conflict.
    #[test]
    fn a_tick_only_change_is_not_saved() {
        let mut ticked = Ticked::default();
        assert!(!ticked.only("local", "base"), "nothing has ticked yet");

        // One tick, taking the document from the server's version to r1.
        ticked.wrote("base", "r1");
        assert!(ticked.only("r1", "base"));

        // The next slot, landing on the first: still nothing but ticks.
        ticked.wrote("r1", "r2");
        assert!(ticked.only("r2", "base"));

        // The person types. The document is no longer what the tick left.
        assert!(!ticked.only("r3", "base"));
        // And a tick on top of that edit does not make the edit unsavable.
        ticked.wrote("r3", "r4");
        assert!(!ticked.only("r4", "base"));

        // A remote change moved the server on, so ours is a real difference.
        ticked.wrote("base", "r5");
        assert!(!ticked.only("r5", "r9"));
    }

    // The clock on two tabs: the loser's 409 body is what it already holds.
    #[test]
    fn a_conflict_that_changes_nothing_is_not_a_dropped_edit() {
        let canvas: Canvas = serde_json::from_str(
            r#"{"nodes":[{"id":"n7","type":"text","x":0,"y":0,"width":9,"height":9,"text":"14:32"}],"edges":[]}"#,
        )
        .expect("fixture");
        assert!(!dropped(&canvas, &canvas.clone()));

        let mut theirs = canvas.clone();
        theirs
            .set_text("n7", "14:33".to_owned())
            .expect("a text node");
        assert!(dropped(&canvas, &theirs));
    }

    #[test]
    fn a_reply_is_the_new_rev_a_conflict_or_a_refusal() {
        assert!(matches!(
            replied(Ok(response(204, ""))),
            Reply::Saved(rev) if rev == "r2"
        ));
        let conflict = replied(Ok(response(409, r#"{"nodes":[],"edges":[]}"#)));
        assert!(
            matches!(&conflict, Reply::Conflict(canvas, rev) if canvas.nodes.is_empty() && rev == "r2"),
            "{conflict:?}"
        );
        // A validation refusal carries what was wrong, and is not a conflict.
        assert!(matches!(
            replied(Ok(response(422, r#"{"errors":["edge e1: node nope does not exist"]}"#))),
            Reply::Failed(message) if message.contains("does not exist")
        ));
        assert!(matches!(
            replied(Err("connection refused".to_owned())),
            Reply::Failed(_)
        ));
    }
}
