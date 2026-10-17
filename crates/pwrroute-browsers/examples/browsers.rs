//! Look at the catalog and this machine's browsers, and dry-run a launch.
//! Never starts a browser.
//!
//! ```text
//! cargo run -p pwrroute-browsers --example browsers -- catalog
//! cargo run -p pwrroute-browsers --example browsers -- detect
//! cargo run -p pwrroute-browsers --example browsers -- profiles "Google Chrome"
//! cargo run -p pwrroute-browsers --example browsers -- plan "Google Chrome" URL \
//!     [--profile NAME] [--bundle-id | --path] [--background] [-- ARGS...]
//! ```

use std::process::ExitCode;

use pwrroute_browsers::{
    Catalog, Env, LaunchOptions, Launcher, Platform, detect_installed, list_profiles,
};
use pwrroute_engine::{AppType, Browser};

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let env = Env::system();
    match args.first().map(String::as_str) {
        Some("catalog") => {
            for b in Catalog::builtin().browsers() {
                let platforms: Vec<&str> = [
                    (Platform::MacOs, "macOS"),
                    (Platform::Windows, "Windows"),
                    (Platform::Linux, "Linux"),
                ]
                .into_iter()
                .filter(|(p, _)| b.supports(*p))
                .map(|(_, n)| n)
                .collect();
                println!("{:<26} {:<9?} {}", b.name, b.engine, platforms.join(", "));
            }
        }
        Some("detect") => {
            for b in detect_installed(&env) {
                let id = b.browser.map_or("-", |e| e.id.as_str());
                println!("{:<28} {:<20} {}", b.name, id, b.location.display());
            }
        }
        Some("profiles") if args.len() == 2 => {
            let Some(b) = Catalog::builtin().find(&args[1]) else {
                eprintln!("{:?} is not in the catalog", args[1]);
                return ExitCode::FAILURE;
            };
            for p in list_profiles(b, &env) {
                let star = if p.default { "*" } else { " " };
                println!("{star} {:<24} {}", p.name, p.directory);
            }
        }
        Some("plan") if args.len() >= 3 => return plan(env, &args[1], &args[2], &args[3..]),
        _ => {
            eprintln!("usage: browsers catalog | detect | profiles NAME | plan NAME URL [options]");
            return ExitCode::FAILURE;
        }
    }
    ExitCode::SUCCESS
}

fn plan(env: Env, name: &str, url: &str, rest: &[String]) -> ExitCode {
    let mut browser = Browser {
        name: name.into(),
        app_type: AppType::AppName,
        url: url.into(),
        profile: String::new(),
        args: Vec::new(),
        open_in_background: None,
    };
    let mut it = rest.iter();
    while let Some(arg) = it.next() {
        match arg.as_str() {
            "--profile" => browser.profile = it.next().cloned().unwrap_or_default(),
            "--bundle-id" => browser.app_type = AppType::BundleId,
            "--path" => browser.app_type = AppType::Path,
            "--background" => browser.open_in_background = Some(true),
            "--" => browser.args = it.by_ref().cloned().collect(),
            other => {
                eprintln!("unknown option {other}");
                return ExitCode::FAILURE;
            }
        }
    }
    match Launcher::with_env(env, LaunchOptions::default()).plan(&browser) {
        Ok(Some(plan)) => {
            println!("{}", plan.command_line());
            for note in &plan.notes {
                println!("note: {note:?}");
            }
            ExitCode::SUCCESS
        }
        Ok(None) => {
            println!("(not opened)");
            ExitCode::SUCCESS
        }
        Err(e) => {
            eprintln!("{e}");
            ExitCode::FAILURE
        }
    }
}
