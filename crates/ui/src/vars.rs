use bevy::prelude::*;
use extboard_core::{plain_file, url_path};
use serde_json::{Map, Value};
use std::collections::HashMap;
use std::sync::{Arc, Mutex, PoisonError};

use crate::client::{Document, Notice, base_url};

// The board's `vars` with each loaded `data` file merged in under its name.
#[derive(Resource, Default, PartialEq)]
pub struct Vars(pub extboard_core::Vars);

type Arrival = (String, String, Result<Value, String>);

#[derive(Resource, Default)]
pub struct Data {
    loaded: Map<String, Value>,
    requested: Vec<(String, String)>,
    inbox: Arc<Mutex<Vec<Arrival>>>,
}

pub struct VarsPlugin;

impl Plugin for VarsPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<Vars>()
            .init_resource::<Data>()
            .add_systems(
                Update,
                (
                    request.run_if(resource_exists_and_changed::<Document>),
                    receive,
                    merge.run_if(resource_exists::<Document>),
                )
                    .chain(),
            );
    }
}

fn request(document: Res<Document>, mut data: ResMut<Data>, mut notice: ResMut<Notice>) {
    let declared: Vec<(String, String)> = document
        .0
        .data_files()
        .into_iter()
        .map(|(name, file)| (name.to_owned(), file.to_owned()))
        .collect();
    if declared == data.requested {
        return;
    }
    data.loaded.clear();
    for (name, file) in &declared {
        if let Err(refused) = fetchable(file) {
            notice.0 = Some(format!("data {name}: {refused}"));
            continue;
        }
        let inbox = data.inbox.clone();
        let (name, file) = (name.clone(), file.clone());
        let url = format!("{}/f/{}", base_url(), url_path(&file));
        ehttp::fetch(ehttp::Request::get(url), move |result| {
            let parsed = parse(result);
            inbox
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .push((name, file, parsed));
        });
    }
    data.requested = declared;
}

fn fetchable(file: &str) -> Result<(), String> {
    if !plain_file(file) {
        return Err(format!("{file} is not a file in the spaces dir"));
    }
    if !file.to_ascii_lowercase().ends_with(".json") {
        return Err(format!("{file} is not .json"));
    }
    Ok(())
}

fn parse(result: ehttp::Result<ehttp::Response>) -> Result<Value, String> {
    let response = result?;
    if !response.ok {
        return Err(format!("extd returned {}", response.status));
    }
    serde_json::from_slice(&response.bytes).map_err(|e| e.to_string())
}

fn receive(mut data: ResMut<Data>, mut notice: ResMut<Notice>) {
    let arrivals = std::mem::take(&mut *data.inbox.lock().unwrap_or_else(PoisonError::into_inner));
    for (name, file, parsed) in arrivals {
        // A reply to a `data` block since edited or left behind.
        if !data.requested.contains(&(name.clone(), file.clone())) {
            continue;
        }
        match parsed {
            Ok(value) => {
                data.loaded.insert(name, value);
            }
            Err(e) => notice.0 = Some(format!("data {name} ({file}): {e}")),
        }
    }
}

impl Data {
    pub fn by_file(&self) -> HashMap<String, Option<Value>> {
        self.requested
            .iter()
            .map(|(name, file)| (file.clone(), self.loaded.get(name).cloned()))
            .collect()
    }
}

fn merge(document: Res<Document>, data: Res<Data>, mut vars: ResMut<Vars>) {
    if !document.is_changed() && !data.is_changed() {
        return;
    }
    vars.set_if_neq(Vars(merged(document.0.vars(), &data.loaded)));
}

fn merged(vars: &extboard_core::Vars, loaded: &Map<String, Value>) -> extboard_core::Vars {
    let mut out = vars.clone();
    out.extend(
        loaded
            .iter()
            .map(|(name, value)| (name.clone(), value.clone())),
    );
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn a_data_entry_shadows_a_var_of_the_same_name() {
        let vars = json!({ "Monday": "rainy", "City": "Porto" });
        let loaded = json!({ "Monday": { "temperature": 21 } });
        let got = merged(vars.as_object().unwrap(), loaded.as_object().unwrap());
        assert_eq!(
            extboard_core::interpolate("{{Monday.temperature}} in {{City}}", &got),
            "21 in Porto"
        );
    }

    #[test]
    fn only_a_plain_json_file_is_asked_for() {
        assert!(fetchable("monday.json").is_ok());
        assert!(fetchable("../../.ssh/id_rsa").is_err());
        assert!(fetchable("notes.md").is_err());
    }
}
