mod http;

use bevy::prelude::*;
use extboard_core::Canvas;
use std::sync::Arc;

// Natively a reply lands on ehttp's thread, so each one pushes into the inbox a
// system already drains.
pub type Reply<T> = Box<dyn FnOnce(T) + Send>;

#[derive(Debug)]
pub enum Loaded {
    Canvas(Canvas, String),
    // The rev asked about is still current.
    Unchanged,
    // Refused, and why: locked, or the password sent was wrong.
    Locked(String),
}

#[derive(Debug)]
pub enum Saved {
    Saved(String),
    // Someone else's save landed first: theirs, and its rev.
    Conflict(Canvas, String),
}

pub enum Event {
    // A space and its new rev; both empty when the stream lagged.
    Changed(String, String),
    Disconnected,
}

pub trait Backend {
    fn get(&self, space: &str, rev: &str, reply: Reply<Result<Loaded, String>>);
    fn subscribe(&self, on: Box<dyn Fn(Event) + Send>);
    fn put(&self, space: &str, canvas: &Canvas, base: &str, reply: Reply<Result<Saved, String>>);
    fn list(&self, reply: Reply<Result<Vec<String>, String>>);
    // The open space and the ones it was made inside, root first.
    fn path(&self, space: &str, reply: Reply<Option<Vec<String>>>);
    // Both answer with the id the backend settled on.
    fn create(&self, reply: Reply<Result<String, String>>);
    fn create_inner(&self, parent: &str, name: &str, reply: Reply<Result<String, String>>);
    fn rename(&self, from: &str, to: &str, reply: Reply<Result<(), String>>);
    // A file in the spaces dir, such as a `data` block's JSON.
    fn read(&self, file: &str, reply: Reply<Result<Vec<u8>, String>>);
    // Answers with the path written, which is not always `name`: nothing is overwritten.
    fn upload(&self, name: &str, bytes: Vec<u8>, reply: Reply<Result<String, String>>);
}

// An `Arc` rather than a `Box`: the tab's drop and paste listeners hold one.
#[derive(Resource, Clone, Deref)]
pub struct Io(Arc<dyn Backend + Send + Sync>);

impl Default for Io {
    fn default() -> Self {
        Self(Arc::new(http::Http))
    }
}
