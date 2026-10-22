//! `pwrroute-links`: PwrRoute's link helper on macOS.
//!
//! LaunchServices activates whichever app it hands a link to. The app that
//! draws PwrRoute's windows is a regular app while one is open, so if it
//! were the `http`/`https` handler every link would bring its window over
//! the app the reader clicked in before the browser came forward. This
//! helper is the handler instead: a windowless (`LSUIElement`) app nested in
//! `PwrRoute.app/Contents/Helpers`, that forwards each link to the running
//! router over its socket, as a second launch of the app does, and stays
//! resident for the next one. When no router answers it launches the app
//! in the background with the links on its command line ([`forward`]).
//!
//! It never logs a link. It holds no engine and no window.
//!
//! Elsewhere there is nothing for it to do: Windows and Linux start the app
//! itself for each link, in the background.

// Everything but the tests is the macOS side's.
#[cfg_attr(not(target_os = "macos"), allow(dead_code))]
mod forward;
#[cfg(target_os = "macos")]
mod macos;

use std::process::ExitCode;

fn main() -> ExitCode {
    #[cfg(target_os = "macos")]
    {
        macos::run();
        ExitCode::SUCCESS
    }
    #[cfg(not(target_os = "macos"))]
    {
        eprintln!("pwrroute-links: the link helper is for macOS; run pwrroute instead");
        ExitCode::from(2)
    }
}
