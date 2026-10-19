//! `pwrroute mcp`: the stdio end an MCP client launches. It copies bytes
//! between its stdin/stdout and the running app's link, and nothing else:
//! the app is the MCP server.
//!
//! ```text
//! claude mcp add --scope user pwrroute -- pwrroute mcp
//! codex mcp add pwrroute -- pwrroute mcp
//! ```

use std::{io, process::ExitCode, time::Duration};

use tokio::io::{AsyncWriteExt as _, copy};

use crate::link::{self, Endpoint};

/// After the client closes stdin, how long replies still in flight may take
/// to arrive. On Unix the app sees the half-close and hangs up sooner; a
/// Windows named pipe cannot half-close, so there the app only learns the
/// client is gone when the proxy exits.
const DRAIN: Duration = Duration::from_secs(2);

/// Pipes stdio to the app at `endpoint` until either side hangs up.
pub async fn proxy(endpoint: &Endpoint) -> io::Result<()> {
    let stream = link::connect(endpoint).await.map_err(|e| {
        io::Error::new(
            e.kind(),
            format!(
                "PwrRoute is not running, or Local agents is off in its Settings ({endpoint}: {e})"
            ),
        )
    })?;
    let (mut from_app, mut to_app) = tokio::io::split(stream);
    let mut stdin = tokio::io::stdin();
    let mut stdout = tokio::io::stdout();

    let down = async {
        let copied = copy(&mut from_app, &mut stdout).await;
        let _ = stdout.flush().await;
        copied
    };
    let up = async {
        let copied = copy(&mut stdin, &mut to_app).await;
        // The client is done: tell the app, which answers what is in
        // flight and then hangs up, ending `down`. (A no-op on a pipe.)
        let _ = to_app.shutdown().await;
        copied
    };
    tokio::pin!(down);
    tokio::select! {
        // The app hung up (it quit, or Local agents was turned off), or
        // turned this connection away before it said anything.
        result = &mut down => match result {
            Ok(0) => Err(io::Error::new(
                io::ErrorKind::ConnectionRefused,
                format!("PwrRoute at {endpoint} closed the connection before answering (too many local agents connected?)"),
            )),
            result => ended(result),
        },
        result = up => {
            ended(result)?;
            match tokio::time::timeout(DRAIN, &mut down).await {
                Ok(result) => ended(result),
                Err(_) => Ok(()),
            }
        }
    }
}

/// A copy that stopped because the other side went away is a normal end,
/// not an error to report.
fn ended(result: io::Result<u64>) -> io::Result<()> {
    match result {
        Ok(_) => Ok(()),
        Err(e)
            if matches!(
                e.kind(),
                io::ErrorKind::BrokenPipe
                    | io::ErrorKind::ConnectionReset
                    | io::ErrorKind::ConnectionAborted
                    | io::ErrorKind::NotConnected
                    | io::ErrorKind::UnexpectedEof
            ) =>
        {
            Ok(())
        }
        Err(e) => Err(e),
    }
}

/// The whole `pwrroute mcp` command, for the app's `main` to dispatch to.
#[must_use]
pub fn main() -> ExitCode {
    let runtime = match tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
    {
        Ok(runtime) => runtime,
        Err(e) => {
            eprintln!("pwrroute mcp: {e}");
            return ExitCode::FAILURE;
        }
    };
    let result = runtime.block_on(proxy(&Endpoint::for_user()));
    // Stdin is read on a blocking thread that cannot be interrupted; do not
    // wait for it when the app hung up first.
    runtime.shutdown_background();
    match result {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("pwrroute mcp: {e}");
            ExitCode::FAILURE
        }
    }
}
