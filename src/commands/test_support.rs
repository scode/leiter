use std::path::Path;

use crate::commands::agent_setup;

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
