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

/// PwrRoute's diagnostic log, beside the other Pwr apps':
/// `~/Library/Logs/PwrRoute`, `%LOCALAPPDATA%\PwrRoute\logs`, or
/// `$XDG_STATE_HOME/pwrroute` (`~/.local/state/pwrroute`).
pub fn log_dir() -> Option<PathBuf> {
    if cfg!(target_os = "macos") {
        dirs::home_dir().map(|home| home.join("Library/Logs/PwrRoute"))
    } else if cfg!(windows) {
        dirs::data_local_dir().map(|d| d.join("PwrRoute").join("logs"))
    } else {
        dirs::state_dir()
            .or_else(dirs::data_local_dir)
            .map(|d| d.join("pwrroute"))
    }
}
