use clap::{Parser, Subcommand};
use extboard_core::{Canvas, validate};
use std::error::Error;
use std::fs;
use std::io::{self, Read};
use std::path::{Path, PathBuf};

#[derive(Parser)]
#[command(name = "extd", version, about = "The extboard server")]
pub struct Cli {
    #[command(subcommand)]
    pub command: Command,
}

#[derive(Subcommand)]
pub enum Command {
    // Run the HTTP server on loopback.
    Serve {
        // Port to bind on 127.0.0.1.
        #[arg(long, default_value_t = 7777)]
        port: u16,

        // Where `trunk build` left the wasm bundle. Not embedded in the binary:
        // a 70MB asset in the crate would make every `cargo build` wear it.
        #[arg(long, default_value = "dist")]
        dist: PathBuf,
    },

    // Print the LLM view of a .canvas file on stdout.
    Project {
        file: PathBuf,

        // Node ids to scope the projection to, comma separated.
        #[arg(long, value_delimiter = ',')]
        selection: Vec<String>,
    },

    // Read a projection on stdin and write it back over a .canvas file.
    Unproject {
        file: PathBuf,
    },

    // Parse, validate and rewrite a .canvas file in our format.
    Fmt {
        file: PathBuf,

        // Report instead of writing.
        #[arg(long)]
        check: bool,
    },

    Pull {
        file: PathBuf,

        #[arg(long)]
        space: Option<String>,

        #[arg(long)]
        server: Option<String>,
    },

    Push {
        file: PathBuf,

        #[arg(long)]
        space: Option<String>,

        #[arg(long)]
        server: Option<String>,

        // The rev `pull` printed. Required: without a base there is no
        // compare-and-swap, only a clobber.
        #[arg(long)]
        base: String,
    },
}

pub fn project(file: &Path, selection: &[String]) -> Result<(), Box<dyn Error>> {
    let selection = (!selection.is_empty()).then_some(selection);
    let projected = crate::project::render(&load(file)?.1, selection)
        .map_err(|e| format!("{}: {e}", file.display()))?;
    print!("{projected}");
    Ok(())
}

pub fn unproject(file: &Path) -> Result<(), Box<dyn Error>> {
    let mut projected = String::new();
    io::stdin().read_to_string(&mut projected)?;

    let (_, original) = load(file)?;
    let canvas = crate::project::unproject(&projected, &original)
        .map_err(|e| format!("{}: {e}", file.display()))?;

    Ok(fs::write(file, canvas.to_pretty_string())?)
}

pub fn fmt(file: &Path, check: bool) -> Result<(), Box<dyn Error>> {
    let (source, canvas) = load(file)?;
    validated(file, &canvas)?;

    let formatted = canvas.to_pretty_string();
    if source == formatted {
        return Ok(());
    }
    if check {
        return Err(format!("{}: not formatted", file.display()).into());
    }
    Ok(fs::write(file, formatted)?)
}

pub fn pull(file: &Path, space: Option<&str>, server: Option<&str>) -> Result<(), Box<dyn Error>> {
    let (server, id) = (base_url(server), space_id(file, space)?);
    let url = format!("{server}/api/spaces/{id}");

    let mut response = agent()
        .get(&url)
        .call()
        .map_err(|e| format!("{url}: {e}"))?;
    let (rev, status) = (rev_of(&response), response.status().as_u16());
    let body = response.body_mut().read_to_string()?;

    match status {
        200 => {}
        404 => return Err(format!("no space {id} on {server}").into()),
        _ => return Err(format!("{url}: {status} {body}").into()),
    }

    fs::write(file, &body)?;
    println!("pulled {id} @ {rev}");
    Ok(())
}

pub fn push(
    file: &Path,
    space: Option<&str>,
    server: Option<&str>,
    base: &str,
) -> Result<(), Box<dyn Error>> {
    let (server, id) = (base_url(server), space_id(file, space)?);

    let (_, canvas) = load(file)?;
    validated(file, &canvas)?;

    let url = format!("{server}/api/spaces/{id}");
    let mut response = agent()
        .put(&url)
        .header("Content-Type", "application/json")
        .header("If-Match", format!("\"{base}\""))
        .send(canvas.to_pretty_string())
        .map_err(|e| format!("{url}: {e}"))?;

    let (rev, status) = (rev_of(&response), response.status().as_u16());
    let body = response.body_mut().read_to_string()?;

    match status {
        204 => {
            println!("pushed {id} @ {rev}");
            Ok(())
        }
        409 => Err(format!("{id}: server moved to {rev} since {base}; re-pull and redo").into()),
        422 => Err(refusal(&body).into()),
        _ => Err(format!("{url}: {status} {body}").into()),
    }
}

fn agent() -> ureq::Agent {
    ureq::Agent::config_builder()
        .http_status_as_error(false)
        .build()
        .into()
}

// `--server`, else EXTBOARD_SERVER, resolved the way ui/src/client.rs does it.
fn base_url(flag: Option<&str>) -> String {
    flag.map(ToOwned::to_owned)
        .unwrap_or_else(|| {
            std::env::var("EXTBOARD_SERVER").unwrap_or_else(|_| "http://127.0.0.1:7777".to_owned())
        })
        .trim_end_matches('/')
        .to_owned()
}

fn space_id(file: &Path, flag: Option<&str>) -> Result<String, Box<dyn Error>> {
    if let Some(id) = flag {
        return Ok(id.to_owned());
    }
    file.file_stem()
        .and_then(|stem| stem.to_str())
        .map(ToOwned::to_owned)
        .ok_or_else(|| format!("{}: no file stem to use as a space id", file.display()).into())
}

fn rev_of(response: &ureq::http::Response<ureq::Body>) -> String {
    response
        .headers()
        .get("etag")
        .and_then(|etag| etag.to_str().ok())
        .unwrap_or_default()
        .trim_matches('"')
        .to_owned()
}

fn refusal(body: &str) -> String {
    serde_json::from_str::<serde_json::Value>(body)
        .ok()
        .and_then(|parsed| {
            let errors: Vec<&str> = parsed
                .get("errors")?
                .as_array()?
                .iter()
                .filter_map(serde_json::Value::as_str)
                .collect();
            (!errors.is_empty()).then(|| errors.join("\n"))
        })
        .unwrap_or_else(|| body.to_owned())
}

fn validated(file: &Path, canvas: &Canvas) -> Result<(), Box<dyn Error>> {
    let Err(errors) = validate(canvas) else {
        return Ok(());
    };
    for error in &errors {
        eprintln!("{}: {error}", file.display());
    }
    Err(format!("{}: {} validation errors", file.display(), errors.len()).into())
}

// The file's own bytes come back too, because `fmt` compares against them.
fn load(file: &Path) -> Result<(String, Canvas), Box<dyn Error>> {
    let source = fs::read_to_string(file).map_err(|e| format!("{}: {e}", file.display()))?;
    let canvas = serde_json::from_str(&source).map_err(|e| format!("{}: {e}", file.display()))?;
    Ok((source, canvas))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn write(name: &str, body: &str) -> PathBuf {
        let path = std::env::temp_dir().join(name);
        fs::write(&path, body).unwrap();
        path
    }

    // Obsidian's bytes are its own; ours settle after one pass.
    #[test]
    fn formatting_is_idempotent() {
        let path = write(
            "extboard-fmt-test.canvas",
            include_str!("../../core/tests/fixtures/kitchen-sink.canvas"),
        );

        fmt(&path, false).unwrap();
        let once = fs::read_to_string(&path).unwrap();
        assert!(
            fmt(&path, true).is_ok(),
            "settled file still reports unformatted"
        );
        fmt(&path, false).unwrap();

        assert_eq!(once, fs::read_to_string(&path).unwrap());
        fs::remove_file(&path).unwrap();
    }

    #[test]
    fn push_refuses_an_invalid_canvas_before_the_network() {
        let body = r#"{"nodes":[],"edges":[{"id":"e1","fromNode":"nope","toNode":"gone"}]}"#;
        let path = write("extboard-push-dangling.canvas", body);

        let err = push(&path, None, Some("http://127.0.0.1:1"), "deadbeef")
            .unwrap_err()
            .to_string();

        assert!(err.contains("2 validation errors"), "{err}");
        fs::remove_file(&path).unwrap();
    }

    #[test]
    fn a_refusal_reports_every_error_the_server_listed() {
        let body = r#"{"errors":["e1: fromNode nope is not a node","e1: toNode gone"]}"#;
        let reported = refusal(body);

        assert!(reported.contains("fromNode nope"), "{reported}");
        assert!(reported.contains("toNode gone"), "{reported}");
        assert_eq!(refusal("502 Bad Gateway"), "502 Bad Gateway");
    }

    #[test]
    fn a_space_id_falls_back_to_the_file_stem() {
        let path = PathBuf::from("/tmp/untitled.canvas");

        assert_eq!(space_id(&path, None).unwrap(), "untitled");
        assert_eq!(space_id(&path, Some("other")).unwrap(), "other");
        assert_eq!(base_url(Some("http://h:1/")), "http://h:1");
    }

    #[test]
    fn a_dangling_edge_fails_and_leaves_the_file_alone() {
        let body = r#"{"nodes":[],"edges":[{"id":"e1","fromNode":"nope","toNode":"gone"}]}"#;
        let path = write("extboard-fmt-dangling.canvas", body);

        let err = fmt(&path, false).unwrap_err().to_string();
        assert!(err.contains("2 validation errors"), "{err}"); // both ends dangle
        assert_eq!(fs::read_to_string(&path).unwrap(), body);
        fs::remove_file(&path).unwrap();
    }
}
