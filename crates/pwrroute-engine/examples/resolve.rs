//! Resolve URLs against a config from the command line.
//!
//! ```text
//! cargo run --release -p pwrroute-engine --example resolve -- \
//!     --config path/to/config.ts [--opener NAME:BUNDLE_ID] [--timings] URL...
//! ```
//!
//! Prints one JSON line per URL in the shape the Go host's `--test` printed,
//! so the two can be diffed. Config `console` output goes to stderr.

use std::{path::PathBuf, rc::Rc, time::Instant};

use anyhow::{Context as _, Result};
use pwrroute_engine::{Engine, Host, Limits, LogLevel, Opener};

const USAGE: &str = "usage: resolve --config PATH [--opener NAME:BUNDLE_ID] [--timings] URL...";

struct StderrHost;

impl Host for StderrHost {
    fn log(&self, level: LogLevel, message: &str) {
        eprintln!("[config {level:?}] {message}");
    }
}

fn main() -> Result<()> {
    let mut args = std::env::args().skip(1);
    let mut config = None;
    let mut opener = None;
    let mut timings = false;
    let mut urls = Vec::new();
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--config" => config = args.next().map(PathBuf::from),
            "--opener" => {
                let spec = args.next().context(USAGE)?;
                let (name, bundle_id) = spec.split_once(':').unwrap_or((&spec, ""));
                opener = Some(Opener {
                    name: name.to_owned(),
                    bundle_id: bundle_id.to_owned(),
                    ..Opener::default()
                });
            }
            "--timings" => timings = true,
            "-h" | "--help" => {
                println!("{USAGE}");
                return Ok(());
            }
            _ => urls.push(arg),
        }
    }
    let config = config.context(USAGE)?;

    let started = Instant::now();
    let engine = Engine::load(&config, Rc::new(StderrHost), Limits::default())?;
    let loaded = started.elapsed();
    for url in &urls {
        let r = engine.resolve(url, opener.as_ref())?;
        let mut line = serde_json::json!({
            "url": r.browser.url,
            "browser": r.browser.name,
            "profile": r.browser.profile,
            "args": r.browser.args,
            "openInBackground": r.browser.open_in_background.unwrap_or(false),
        });
        if let Some(error) = r.error {
            line["error"] = error.into();
        }
        println!("{line}");
    }
    if timings {
        eprintln!(
            "load {:.2} ms, {} url(s) {:.2} ms",
            loaded.as_secs_f64() * 1000.0,
            urls.len(),
            (started.elapsed() - loaded).as_secs_f64() * 1000.0
        );
    }
    Ok(())
}
