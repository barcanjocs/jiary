//! Settings persisted between runs, in a `key=value` file beside the
//! layout file: for now whether Jiary asks about sleep and mood.
//!
//! Read and written like `layout.conf` (see layout.rs): unknown keys and
//! bad values are ignored, so an old or hand-edited file never stops the
//! app from starting.

use std::io;
use std::path::Path;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Settings {
    /// Whether the day's first `s` asks about sleep and mood, and the
    /// longer views show them (`/mood-on`, `/mood-off`). Turning it off
    /// keeps what was recorded.
    pub sleep_and_mood: bool,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
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
            let flag = value.trim().parse::<bool>().ok();
            if let ("sleep_and_mood", Some(on)) = (key.trim(), flag) {
                settings.sleep_and_mood = on;
            }
        }
        settings
    }

    pub fn to_file_contents(&self) -> String {
        format!(
            "# jiary settings; written automatically\n\
             sleep_and_mood={}\n",
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
        let off = Settings {
            sleep_and_mood: false,
        };
        assert_eq!(Settings::parse(&off.to_file_contents()), off);
        let junk = "sleep_and_mood=sometimes\nnonsense\ncolour=blue\n";
        assert_eq!(Settings::parse(junk), Settings::default());
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
        };
        off.save(&path).unwrap();
        assert_eq!(Settings::load(&path).unwrap(), off);
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
