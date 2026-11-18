//! Local calendar dates and clock labels shared by the app's views.

use std::time::SystemTime;

use jiff::{Timestamp, Zoned, tz::TimeZone};

pub fn at(time: SystemTime) -> Option<Zoned> {
    from_timestamp(Timestamp::try_from(time).ok()?)
}

pub fn from_timestamp(time: Timestamp) -> Option<Zoned> {
    #[cfg(windows)]
    let zone = {
        let seconds = time.as_second() - i64::from(time.subsec_nanosecond() < 0);
        let offset = pwrroute_os::local_utc_offset(seconds)?;
        TimeZone::fixed(jiff::tz::Offset::from_seconds(offset).ok()?)
    };
    #[cfg(not(windows))]
    let zone = TimeZone::system();
    Some(time.to_zoned(zone))
}

pub fn now() -> Zoned {
    let time = Timestamp::now();
    from_timestamp(time).unwrap_or_else(|| time.to_zoned(TimeZone::UTC))
}

/// A local clock label such as `09:41`.
pub fn clock(at: SystemTime) -> String {
    format(at, "%H:%M")
}

/// A local clock label such as `09:41:08`.
pub fn clock_with_seconds(at: SystemTime) -> String {
    format(at, "%H:%M:%S")
}

fn format(time: SystemTime, pattern: &str) -> String {
    at(time)
        .map(|time| time.strftime(pattern).to_string())
        .unwrap_or_default()
}

/// Resolve the fixed, unambiguous wall times used by the app's test fixtures.
#[cfg(test)]
pub fn test_wall_time(wall: jiff::civil::DateTime) -> Zoned {
    #[cfg(windows)]
    {
        let nominal = wall.to_zoned(TimeZone::UTC).unwrap().timestamp();
        let offset = from_timestamp(nominal).unwrap().offset();
        let instant = wall.to_zoned(TimeZone::fixed(offset)).unwrap().timestamp();
        let local = from_timestamp(instant).unwrap();
        assert_eq!(local.datetime(), wall);
        local
    }
    #[cfg(not(windows))]
    wall.to_zoned(TimeZone::system()).unwrap()
}

#[cfg(all(test, windows))]
#[test]
fn windows_local_time_matches_the_os_clock_rules() {
    const TEST: &str = "local_time::windows_local_time_matches_the_os_clock_rules";
    const CHILD: &str = "PWRROUTE_WINDOWS_TZ_CHILD";
    if std::env::var_os(CHILD).is_none() {
        let result = std::process::Command::new(std::env::current_exe().unwrap())
            .args(["--exact", TEST, "--nocapture"])
            // An override that disagrees with the native zone on every supported Windows setup.
            .env("TZ", "UTC-23")
            .env(CHILD, "1")
            .output()
            .unwrap();
        assert!(
            result.status.success(),
            "{}\n{}",
            String::from_utf8_lossy(&result.stdout),
            String::from_utf8_lossy(&result.stderr),
        );
        return;
    }
    let times = [
        "1969-08-01T23:30:00Z".parse::<Timestamp>().unwrap(),
        "2026-07-01T05:30:00Z".parse().unwrap(),
        "2026-11-18T05:30:00Z".parse().unwrap(),
        Timestamp::from_second(Timestamp::now().as_second()).unwrap(),
    ];
    let mut script = String::new();
    for time in times {
        use std::fmt::Write as _;
        writeln!(
            script,
            "[DateTimeOffset]::FromUnixTimeSeconds({}).ToLocalTime().ToString('yyyy-MM-dd HH:mm:ss zzz', [Globalization.CultureInfo]::InvariantCulture)",
            time.as_second()
        )
        .unwrap();
    }
    let result = std::process::Command::new("powershell.exe")
        .args(["-NoProfile", "-NonInteractive", "-Command", &script])
        .output()
        .unwrap();
    assert!(
        result.status.success(),
        "{}",
        String::from_utf8_lossy(&result.stderr)
    );
    let expected = String::from_utf8(result.stdout).unwrap();
    let actual: Vec<_> = times
        .iter()
        .map(|&time| {
            at(time.into())
                .unwrap()
                .strftime("%Y-%m-%d %H:%M:%S %:z")
                .to_string()
        })
        .collect();
    assert_eq!(actual, expected.lines().collect::<Vec<_>>());
}
