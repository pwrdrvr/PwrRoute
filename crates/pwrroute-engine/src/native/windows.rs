//! Windows: Win32 keyboard state, power status, computer name and a process
//! snapshot.
// Win32 calls need `unsafe`. Each block's safety comment says why the call
// is sound; nothing else in the crate may use `unsafe`.
#![allow(unsafe_code)]

use std::mem::{size_of, zeroed};

use windows_sys::Win32::{
    Foundation::{CloseHandle, INVALID_HANDLE_VALUE},
    System::{
        Diagnostics::ToolHelp::{
            CreateToolhelp32Snapshot, PROCESSENTRY32W, Process32FirstW, Process32NextW,
            TH32CS_SNAPPROCESS,
        },
        Power::{GetSystemPowerStatus, SYSTEM_POWER_STATUS},
        SystemInformation::{ComputerNameDnsHostname, GetComputerNameExW},
    },
    UI::Input::KeyboardAndMouse::{
        GetAsyncKeyState, GetKeyState, VK_CAPITAL, VK_CONTROL, VK_LWIN, VK_MENU, VK_RWIN, VK_SHIFT,
    },
};

use crate::{ModifierKeys, PowerInfo};

fn down(key: u16) -> bool {
    // SAFETY: no pointers; any virtual-key code is accepted.
    (unsafe { GetAsyncKeyState(i32::from(key)) } as u16) & 0x8000 != 0
}

/// Option is Alt and command is the Windows key, where those keys sit on a
/// Mac keyboard. There is no way to read fn.
pub(super) fn modifier_keys() -> ModifierKeys {
    // SAFETY: no pointers. The low bit is the toggle state.
    let caps_lock = unsafe { GetKeyState(i32::from(VK_CAPITAL)) } & 1 != 0;
    ModifierKeys {
        shift: down(VK_SHIFT),
        option: down(VK_MENU),
        command: down(VK_LWIN) || down(VK_RWIN),
        control: down(VK_CONTROL),
        caps_lock,
        fn_key: false,
        function_key: false,
    }
}

/// The DNS host name, for both names: Windows has no separate display name.
pub(super) fn computer_names() -> (String, String) {
    let mut buffer = [0u16; 256];
    let mut len = buffer.len() as u32;
    // SAFETY: `buffer` holds `len` UTF-16 units; on success `len` is the
    // number written, excluding the NUL.
    let ok = unsafe { GetComputerNameExW(ComputerNameDnsHostname, buffer.as_mut_ptr(), &mut len) };
    let name = if ok != 0 {
        String::from_utf16_lossy(&buffer[..len as usize])
    } else {
        std::env::var("COMPUTERNAME").unwrap_or_default()
    };
    (name.clone(), name)
}

pub(super) fn power_info() -> PowerInfo {
    // SAFETY: SYSTEM_POWER_STATUS is plain data, so all-zero is valid, and
    // the call fills it.
    let mut status: SYSTEM_POWER_STATUS = unsafe { zeroed() };
    if unsafe { GetSystemPowerStatus(&mut status) } == 0 {
        return PowerInfo::default();
    }
    // 255 means unknown in each field; battery flag 128 means no battery.
    let has_battery = status.BatteryFlag != 255 && status.BatteryFlag & 128 == 0;
    PowerInfo {
        is_charging: (status.BatteryFlag != 255).then_some(status.BatteryFlag & 8 != 0),
        is_connected: match status.ACLineStatus {
            0 => Some(false),
            1 => Some(true),
            _ => None,
        },
        percentage: (has_battery && status.BatteryLifePercent <= 100)
            .then_some(f64::from(status.BatteryLifePercent)),
    }
}

/// Whether a process's image name is `identifier`, or `identifier.exe`,
/// ignoring case.
pub(super) fn is_app_running(identifier: &str) -> bool {
    let want = identifier.to_lowercase();
    let want_exe = if want.ends_with(".exe") {
        want.clone()
    } else {
        format!("{want}.exe")
    };
    // SAFETY: the snapshot handle is checked and closed below; the entry is
    // plain data with its size set as Process32FirstW requires, and
    // `szExeFile` is NUL-terminated within its array.
    unsafe {
        let snapshot = CreateToolhelp32Snapshot(TH32CS_SNAPPROCESS, 0);
        if snapshot == INVALID_HANDLE_VALUE {
            return false;
        }
        let mut entry: PROCESSENTRY32W = zeroed();
        entry.dwSize = size_of::<PROCESSENTRY32W>() as u32;
        let mut found = false;
        let mut more = Process32FirstW(snapshot, &mut entry) != 0;
        while more {
            let len = entry
                .szExeFile
                .iter()
                .position(|c| *c == 0)
                .unwrap_or(entry.szExeFile.len());
            let exe = String::from_utf16_lossy(&entry.szExeFile[..len]).to_lowercase();
            if exe == want || exe == want_exe {
                found = true;
                break;
            }
            more = Process32NextW(snapshot, &mut entry) != 0;
        }
        CloseHandle(snapshot);
        found
    }
}
