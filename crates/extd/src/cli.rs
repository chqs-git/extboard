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

    if let Err(errors) = validate(&canvas) {
        for error in &errors {
            eprintln!("{}: {error}", file.display());
        }
        return Err(format!("{}: {} validation errors", file.display(), errors.len()).into());
    }

    let formatted = canvas.to_pretty_string();
    if source == formatted {
        return Ok(());
    }
    if check {
        return Err(format!("{}: not formatted", file.display()).into());
    }
    Ok(fs::write(file, formatted)?)
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
    fn a_dangling_edge_fails_and_leaves_the_file_alone() {
        let body = r#"{"nodes":[],"edges":[{"id":"e1","fromNode":"nope","toNode":"gone"}]}"#;
        let path = write("extboard-fmt-dangling.canvas", body);

        let err = fmt(&path, false).unwrap_err().to_string();
        assert!(err.contains("2 validation errors"), "{err}"); // both ends dangle
        assert_eq!(fs::read_to_string(&path).unwrap(), body);
        fs::remove_file(&path).unwrap();
    }
}
