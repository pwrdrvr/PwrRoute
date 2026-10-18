//! Where PwrRoute keeps its files.

use std::path::PathBuf;

/// PwrRoute's settings and rules: `~/Library/Application Support/PwrRoute`,
/// `%APPDATA%\PwrRoute`, or `$XDG_CONFIG_HOME/pwrroute`.
pub fn data_dir() -> Option<PathBuf> {
    let name = if cfg!(target_os = "linux") {
        "pwrroute"
    } else {
        "PwrRoute"
    };
    dirs::config_dir().map(|d| d.join(name))
}

/// The router's lock and socket. On Linux, `$XDG_RUNTIME_DIR/pwrroute` when
/// the session has one (private to the user, cleared at logout); otherwise
/// [`data_dir`]. Windows keeps only the lock file here: its socket is a
/// named pipe.
pub fn router_dir() -> Option<PathBuf> {
    if cfg!(target_os = "linux")
        && let Some(runtime) = dirs::runtime_dir()
    {
        return Some(runtime.join("pwrroute"));
    }
    data_dir()
}
