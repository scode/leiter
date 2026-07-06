use std::fs;
use std::path::Path;

use chrono::{TimeZone, Utc};

use crate::commands::agent_setup;
use crate::paths;
use crate::state::LeiterState;
use crate::templates::{SOUL_TEMPLATE, SOUL_TEMPLATE_VERSION};

/// Holds temporary directories for both state and claude home.
/// Exposes `path()` returning the state directory for backward compatibility
/// with tests that only need `tmp.path()`.
pub struct TestDirs {
    pub state: tempfile::TempDir,
    // Held to keep the tempdir alive for the test's duration.
    #[allow(dead_code)]
    pub claude: tempfile::TempDir,
}

impl TestDirs {
    pub fn path(&self) -> &Path {
        self.state.path()
    }
}

pub fn setup_state_dir() -> TestDirs {
    let state = tempfile::tempdir().expect("failed to create temporary state directory");
    let claude = tempfile::tempdir().expect("failed to create temporary claude home");
    agent_setup::run(state.path(), claude.path()).expect("failed to initialize test state");
    TestDirs { state, claude }
}

pub fn bytes_to_string(out: Vec<u8>) -> String {
    String::from_utf8(out).expect("command output must be valid UTF-8")
}

/// Write a minimal initialized state directory with the given epoch values.
pub fn write_state_with_epochs(state_dir: &Path, soft: u32, hard: u32) {
    let state = LeiterState {
        version: crate::state::STATE_VERSION,
        last_distilled: Utc.with_ymd_and_hms(1970, 1, 1, 0, 0, 0).unwrap(),
        soul_version: SOUL_TEMPLATE_VERSION,
        setup_soft_epoch: soft,
        setup_hard_epoch: hard,
        codex: Default::default(),
    };
    fs::create_dir_all(state_dir).unwrap();
    fs::write(paths::soul_path(state_dir), SOUL_TEMPLATE).unwrap();
    state.save(&paths::state_path(state_dir)).unwrap();
}

/// Load, mutate, and atomically rewrite `state.toml` in tests.
pub fn update_state(state_dir: &Path, f: impl FnOnce(&mut LeiterState)) {
    let path = paths::state_path(state_dir);
    let mut state = LeiterState::load(&path).unwrap();
    f(&mut state);
    state.save(&path).unwrap();
}
