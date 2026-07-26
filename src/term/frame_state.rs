use std::path::{Path, PathBuf};

pub const STATE_VERSION: u32 = 1;

/// Persisted between `rat frame` invocations: how tall the last frame was,
/// at what width, and a hash of its content for change detection.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FrameState {
    pub version: u32,
    pub rows: u16,
    pub cols: u16,
    pub hash: u64,
}

impl FrameState {
    /// None on missing, corrupt, or version-mismatched files — every failure
    /// mode degrades to "repaint from scratch".
    pub fn load(path: &Path) -> Option<Self> {
        let text = std::fs::read_to_string(path).ok()?;
        let mut parts = text.split_whitespace();
        let state = FrameState {
            version: parts.next()?.parse().ok()?,
            rows: parts.next()?.parse().ok()?,
            cols: parts.next()?.parse().ok()?,
            hash: parts.next()?.parse().ok()?,
        };
        (state.version == STATE_VERSION).then_some(state)
    }

    pub fn store(&self, path: &Path) -> std::io::Result<()> {
        std::fs::write(
            path,
            format!(
                "{} {} {} {}\n",
                self.version, self.rows, self.cols, self.hash
            ),
        )
    }
}

/// Keyed per terminal session so two dashboards in two terminals never
/// collide: parent pid + tty inode on unix; terminal-session env vars on
/// Windows (pass --state when running several dashboards in one console).
pub fn default_state_path() -> PathBuf {
    std::env::temp_dir().join(format!("rat-frame-{}", session_key()))
}

#[cfg(unix)]
fn session_key() -> String {
    let parent = std::os::unix::process::parent_id();
    let tty_ino = std::fs::metadata("/dev/tty")
        .map(|m| std::os::unix::fs::MetadataExt::ino(&m))
        .unwrap_or(0);
    format!("{parent}-{tty_ino}")
}

#[cfg(windows)]
fn session_key() -> String {
    use std::hash::{Hash, Hasher};
    let mut hasher = std::hash::DefaultHasher::new();
    std::env::var("WT_SESSION")
        .unwrap_or_default()
        .hash(&mut hasher);
    std::env::var("SESSIONNAME")
        .unwrap_or_default()
        .hash(&mut hasher);
    format!("{:x}", hasher.finish())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trips_through_a_file() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("state");
        let state = FrameState {
            version: STATE_VERSION,
            rows: 12,
            cols: 80,
            hash: 0xDEAD_BEEF,
        };
        state.store(&path).unwrap();
        assert_eq!(FrameState::load(&path), Some(state));
    }

    #[test]
    fn corrupt_and_missing_files_load_as_none() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("state");
        assert_eq!(FrameState::load(&path), None);
        std::fs::write(&path, b"not a state file").unwrap();
        assert_eq!(FrameState::load(&path), None);
    }

    #[test]
    fn version_mismatch_invalidates() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("state");
        let state = FrameState {
            version: STATE_VERSION + 1,
            rows: 1,
            cols: 80,
            hash: 1,
        };
        state.store(&path).unwrap();
        assert_eq!(FrameState::load(&path), None);
    }

    #[test]
    fn default_path_is_stable_within_a_process() {
        assert_eq!(default_state_path(), default_state_path());
    }
}
