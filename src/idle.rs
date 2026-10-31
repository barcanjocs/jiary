//! How long since anyone touched the keyboard or mouse, system-wide, so a
//! session left running while nobody was there can be caught. Read from
//! `ioreg` on macOS and `xprintidle` (when installed) on Linux; elsewhere,
//! or when the tool fails, it is unknown and only sleep is detected (see
//! `App::observe_idle`).

use std::process::Command;
use std::time::Duration;

/// How often `main` asks; also bounds how late a return is noticed.
pub const POLL: Duration = Duration::from_secs(10);

/// System-wide input idle time, if this platform can tell.
pub fn system_idle() -> Option<Duration> {
    if cfg!(target_os = "macos") {
        let out = Command::new("ioreg")
            .args(["-c", "IOHIDSystem", "-d", "4"])
            .output()
            .ok()?;
        parse_ioreg(&String::from_utf8_lossy(&out.stdout))
    } else if cfg!(target_os = "linux") {
        let out = Command::new("xprintidle").output().ok()?;
        let millis = String::from_utf8_lossy(&out.stdout).trim().parse().ok()?;
        Some(Duration::from_millis(millis))
    } else {
        None
    }
}

/// The `"HIDIdleTime" = <nanoseconds>` line of `ioreg -c IOHIDSystem`.
fn parse_ioreg(output: &str) -> Option<Duration> {
    let line = output.lines().find(|l| l.contains("\"HIDIdleTime\""))?;
    let nanos = line.rsplit('=').next()?.trim().parse().ok()?;
    Some(Duration::from_nanos(nanos))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_the_idle_time_from_ioreg() {
        let output = "    | |   \"HIDIdleTime\" = 136128544291\n    | |   \"Other\" = 1\n";
        assert_eq!(
            parse_ioreg(output),
            Some(Duration::from_nanos(136128544291))
        );
        assert_eq!(parse_ioreg("nothing here"), None);
    }
}
