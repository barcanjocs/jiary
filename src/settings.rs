//! Settings persisted between runs, in a `key=value` file beside the
//! layout file: the Pomodoro and whether Jiary asks about sleep and mood.
//!
//! Read and written like `layout.conf` (see layout.rs): unknown keys and
//! bad values are ignored, so an old or hand-edited file never stops the
//! app from starting.

use std::io;
use std::path::Path;

/// How a work session that reaches the Pomodoro target is treated.
///
/// Only `Soft` exists for now. A stricter mode (say, a `Hard` one that
/// rings or ends the session at the target) would be one more variant
/// here, added to `ALL` and `name`, plus what it does in `App::pomodoro`'s
/// callers; the settings file, `/settings` and `p` already go through this
/// type.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PomodoroMode {
    Off,
    /// Shows that the target is reached, without ever interrupting: the
    /// cue is only seen by looking at the main screen.
    Soft,
}

impl PomodoroMode {
    /// Every mode, in the order `/settings` cycles through them.
    pub const ALL: &[PomodoroMode] = &[PomodoroMode::Off, PomodoroMode::Soft];

    /// Its name, as in the settings file and `/settings`.
    pub fn name(self) -> &'static str {
        match self {
            PomodoroMode::Off => "off",
            PomodoroMode::Soft => "soft",
        }
    }

    fn from_name(name: &str) -> Option<Self> {
        Self::ALL.iter().copied().find(|mode| mode.name() == name)
    }

    /// The mode after this one in `ALL`, wrapping around.
    pub fn next(self) -> Self {
        let i = Self::ALL.iter().position(|&m| m == self).unwrap_or(0);
        Self::ALL[(i + 1) % Self::ALL.len()]
    }

    /// The mode before this one in `ALL`, wrapping around.
    pub fn prev(self) -> Self {
        let i = Self::ALL.iter().position(|&m| m == self).unwrap_or(0);
        Self::ALL[(i + Self::ALL.len() - 1) % Self::ALL.len()]
    }
}

/// The Pomodoro target's limits and step, in minutes.
pub const TARGET_MIN: u32 = 5;
pub const TARGET_MAX: u32 = 120;
pub const TARGET_STEP: u32 = 5;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Settings {
    /// The Pomodoro (`p`, `/settings`): its mode and its target session
    /// length in minutes (`TARGET_MIN`–`TARGET_MAX`).
    pub pomodoro: PomodoroMode,
    pub target_minutes: u32,
    /// Whether the day's first `s` asks about sleep and mood, and the
    /// longer views show them (`/mood-on`, `/mood-off`). Turning it off
    /// keeps what was recorded.
    pub sleep_and_mood: bool,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            pomodoro: PomodoroMode::Off,
            target_minutes: 25,
            sleep_and_mood: true,
        }
    }
}

impl Settings {
    pub fn parse(text: &str) -> Self {
        let mut settings = Self::default();
        for line in text.lines() {
            let Some((key, value)) = line.split_once('=') else {
                continue;
            };
            let value = value.trim();
            match key.trim() {
                // An unknown mode (a newer Jiary's) leaves the default.
                "pomodoro" => {
                    if let Some(mode) = PomodoroMode::from_name(value) {
                        settings.pomodoro = mode;
                    }
                }
                "pomodoro_target" => {
                    if let Ok(minutes) = value.parse::<u32>()
                        && (TARGET_MIN..=TARGET_MAX).contains(&minutes)
                    {
                        settings.target_minutes = minutes;
                    }
                }
                "sleep_and_mood" => {
                    if let Ok(on) = value.parse::<bool>() {
                        settings.sleep_and_mood = on;
                    }
                }
                _ => {}
            }
        }
        settings
    }

    pub fn to_file_contents(&self) -> String {
        format!(
            "# jiary settings; written automatically\n\
             pomodoro={}\n\
             pomodoro_target={}\n\
             sleep_and_mood={}\n",
            self.pomodoro.name(),
            self.target_minutes,
            self.sleep_and_mood,
        )
    }

    /// Reads the settings file; a missing file means defaults.
    pub fn load(path: &Path) -> io::Result<Self> {
        match std::fs::read_to_string(path) {
            Ok(text) => Ok(Self::parse(&text)),
            Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(Self::default()),
            Err(e) => Err(e),
        }
    }

    /// Writes the settings file via a temp file and rename, like the
    /// layout file.
    pub fn save(&self, path: &Path) -> io::Result<()> {
        let tmp = path.with_extension("tmp");
        std::fs::write(&tmp, self.to_file_contents())?;
        std::fs::rename(&tmp, path)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn file_round_trips_and_tolerates_junk() {
        let changed = Settings {
            pomodoro: PomodoroMode::Soft,
            target_minutes: 50,
            sleep_and_mood: false,
        };
        assert_eq!(Settings::parse(&changed.to_file_contents()), changed);
        let junk = "sleep_and_mood=sometimes\npomodoro=hard\npomodoro_target=3\n\
                    nonsense\ncolour=blue\n";
        assert_eq!(Settings::parse(junk), Settings::default());
    }

    #[test]
    fn modes_cycle_both_ways() {
        for &mode in PomodoroMode::ALL {
            assert_eq!(mode.next().prev(), mode);
            assert_eq!(PomodoroMode::from_name(mode.name()), Some(mode));
        }
        assert_eq!(PomodoroMode::Off.next(), PomodoroMode::Soft);
    }

    #[test]
    fn missing_file_loads_defaults_and_save_load_round_trips() {
        let dir = std::env::temp_dir().join(format!("jiary-settings-test-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("settings.conf");
        let _ = std::fs::remove_file(&path);
        assert_eq!(Settings::load(&path).unwrap(), Settings::default());

        let off = Settings {
            sleep_and_mood: false,
            ..Default::default()
        };
        off.save(&path).unwrap();
        assert_eq!(Settings::load(&path).unwrap(), off);
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
