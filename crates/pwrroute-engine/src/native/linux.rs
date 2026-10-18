//! Linux: `/proc` and `/sys`, no FFI.

use std::{fs, path::Path};

use crate::{ModifierKeys, PowerInfo};

/// Reading the keyboard needs a display connection (X11) or is refused
/// outright (Wayland), so every key reads as up.
pub(super) fn modifier_keys() -> ModifierKeys {
    ModifierKeys::default()
}

/// The kernel host name, and the pretty name from `/etc/machine-info` when
/// one is set.
pub(super) fn computer_names() -> (String, String) {
    let name = fs::read_to_string("/proc/sys/kernel/hostname")
        .or_else(|_| fs::read_to_string("/etc/hostname"))
        .map(|s| s.trim().to_owned())
        .unwrap_or_default();
    let pretty = fs::read_to_string("/etc/machine-info")
        .ok()
        .and_then(|text| pretty_hostname(&text))
        .unwrap_or_else(|| name.clone());
    (name, pretty)
}

fn pretty_hostname(machine_info: &str) -> Option<String> {
    machine_info.lines().find_map(|line| {
        let value = line.trim().strip_prefix("PRETTY_HOSTNAME=")?;
        let value = value.trim_matches('"').trim_matches('\'').trim();
        (!value.is_empty()).then(|| value.to_owned())
    })
}

pub(super) fn power_info() -> PowerInfo {
    power_info_in(Path::new("/sys/class/power_supply"))
}

/// The first system battery (not a mouse's: those have `scope` `Device`) and
/// whether any mains supply is online.
fn power_info_in(root: &Path) -> PowerInfo {
    let mut info = PowerInfo::default();
    let Ok(entries) = fs::read_dir(root) else {
        return info;
    };
    let mut entries: Vec<_> = entries.flatten().map(|e| e.path()).collect();
    entries.sort();
    let read = |dir: &Path, file: &str| {
        fs::read_to_string(dir.join(file))
            .ok()
            .map(|s| s.trim().to_owned())
    };
    for dir in &entries {
        match read(dir, "type").as_deref() {
            Some("Mains") => {
                let online = read(dir, "online").as_deref() == Some("1");
                info.is_connected = Some(info.is_connected.unwrap_or(false) || online);
            }
            Some("Battery")
                if info.percentage.is_none() && read(dir, "scope").as_deref() != Some("Device") =>
            {
                info.percentage = read(dir, "capacity").and_then(|c| c.parse::<f64>().ok());
                if let Some(status) = read(dir, "status") {
                    info.is_charging = Some(status == "Charging");
                    if status == "Charging" || status == "Full" {
                        info.is_connected = Some(true);
                    }
                }
            }
            _ => {}
        }
    }
    info
}

/// Whether a process's name (`/proc/PID/comm`, at most 15 bytes) or its
/// executable's file name is `identifier`, ignoring ASCII case.
pub(super) fn is_app_running(identifier: &str) -> bool {
    let Ok(entries) = fs::read_dir("/proc") else {
        return false;
    };
    // comm is the executable name cut to 15 bytes, possibly mid-character,
    // so it is compared as bytes.
    let wanted = identifier.as_bytes();
    let short = &wanted[..wanted.len().min(15)];
    entries.flatten().any(|entry| {
        let pid = entry.file_name();
        if !pid.to_string_lossy().bytes().all(|b| b.is_ascii_digit()) {
            return false;
        }
        let dir = entry.path();
        let comm_matches = fs::read(dir.join("comm")).is_ok_and(|comm| {
            let comm = comm.strip_suffix(b"\n").unwrap_or(&comm);
            comm.eq_ignore_ascii_case(wanted) || comm.eq_ignore_ascii_case(short)
        });
        comm_matches
            || fs::read_link(dir.join("exe")).is_ok_and(|exe| {
                exe.file_name()
                    .is_some_and(|n| n.to_string_lossy().eq_ignore_ascii_case(identifier))
            })
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_pretty_hostname() {
        assert_eq!(
            pretty_hostname("CHASSIS=laptop\nPRETTY_HOSTNAME=\"Hunt's Laptop\"\n").as_deref(),
            Some("Hunt's Laptop")
        );
        assert_eq!(pretty_hostname("PRETTY_HOSTNAME=\n"), None);
    }

    #[test]
    fn reads_sysfs_power_supplies() {
        let root = std::env::temp_dir().join(format!("pwrroute-power-{}", std::process::id()));
        let _ = fs::remove_dir_all(&root);
        let supply = |name: &str, files: &[(&str, &str)]| {
            let dir = root.join(name);
            fs::create_dir_all(&dir).unwrap();
            for (file, value) in files {
                fs::write(dir.join(file), format!("{value}\n")).unwrap();
            }
        };
        supply("AC", &[("type", "Mains"), ("online", "1")]);
        supply(
            "BAT0",
            &[
                ("type", "Battery"),
                ("capacity", "87"),
                ("status", "Not charging"),
            ],
        );
        supply(
            "hidpp_battery_0",
            &[("type", "Battery"), ("scope", "Device"), ("capacity", "5")],
        );
        let info = power_info_in(&root);
        assert_eq!(info.percentage, Some(87.0));
        assert_eq!(info.is_charging, Some(false));
        assert_eq!(info.is_connected, Some(true));
        fs::remove_dir_all(&root).unwrap();
    }
}
