use clap::{Parser, Subcommand};
use extboard_core::{Canvas, validate};
use std::error::Error;
use std::fs;
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
    },

    // Parse, validate and rewrite a .canvas file in our format.
    Fmt {
        file: PathBuf,

        // Report instead of writing.
        #[arg(long)]
        check: bool,
    },
}

pub fn fmt(file: &Path, check: bool) -> Result<(), Box<dyn Error>> {
    let source = fs::read_to_string(file).map_err(|e| format!("{}: {e}", file.display()))?;
    let canvas: Canvas =
        serde_json::from_str(&source).map_err(|e| format!("{}: {e}", file.display()))?;

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
        assert!(fmt(&path, true).is_ok(), "settled file still reports unformatted");
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
