//! One jiary at a time: two processes writing the same diary would
//! interleave their changes (say, both ending the same session). The first
//! takes an exclusive lock on a file in the data dir and holds it for its
//! lifetime; a second finds it taken and exits. The OS releases the lock
//! however the first one ends (quit, crash, kill), so a stale lock can't
//! linger. It is a `flock`, separate from SQLite's own locks on the db.

use std::fs::{File, OpenOptions, TryLockError};
use std::io::{self, Read, Seek, Write};
use std::path::Path;

/// Held for as long as this jiary runs; dropping it releases the lock.
pub struct InstanceLock {
    _file: File,
}

#[derive(Debug)]
pub enum LockError {
    /// Another jiary holds the lock; its pid, when it could be read.
    Running(Option<u32>),
    Io(io::Error),
}

impl std::fmt::Display for LockError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            LockError::Running(pid) => {
                write!(f, "jiary is already running")?;
                if let Some(pid) = pid {
                    write!(f, " (pid {pid})")?;
                }
                write!(
                    f,
                    "; only one can run at a time, so the diary stays consistent."
                )
            }
            LockError::Io(e) => write!(f, "jiary: could not take the instance lock: {e}"),
        }
    }
}

/// Takes the lock at `path` (created if missing) and writes this process's
/// pid into it, for a second jiary to report.
pub fn acquire(path: &Path) -> Result<InstanceLock, LockError> {
    let mut file = OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(false)
        .open(path)
        .map_err(LockError::Io)?;
    match file.try_lock() {
        Ok(()) => {
            // Best effort: the lock is what counts, the pid is a courtesy.
            let _ = file
                .set_len(0)
                .and_then(|()| write!(file, "{}", std::process::id()))
                .and_then(|()| file.flush());
            Ok(InstanceLock { _file: file })
        }
        Err(TryLockError::WouldBlock) => {
            let mut contents = String::new();
            let pid = file
                .rewind()
                .and_then(|()| file.read_to_string(&mut contents))
                .ok()
                .and_then(|_| contents.trim().parse().ok());
            Err(LockError::Running(pid))
        }
        Err(TryLockError::Error(e)) => Err(LockError::Io(e)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_second_instance_is_turned_away_until_the_first_ends() {
        let dir = std::env::temp_dir().join(format!("jiary-lock-test-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("jiary.lock");

        let first = acquire(&path).unwrap();
        // A separate open of the file is a separate lock holder, even in
        // the same process: it stands in for a second jiary.
        match acquire(&path) {
            Err(LockError::Running(pid)) => assert_eq!(pid, Some(std::process::id())),
            other => panic!("expected Running, got {:?}", other.map(|_| ())),
        }
        let message = acquire(&path).err().unwrap().to_string();
        assert!(
            message.starts_with("jiary is already running (pid "),
            "{message}"
        );

        // Once the first is gone, the lock is free again.
        drop(first);
        assert!(acquire(&path).is_ok());
        let _ = std::fs::remove_dir_all(&dir);
    }
}
