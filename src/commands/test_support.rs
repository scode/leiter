use crate::commands::agent_setup;

pub fn setup_state_dir() -> tempfile::TempDir {
    let tmp = tempfile::tempdir().expect("failed to create temporary state directory");
    agent_setup::run(tmp.path(), &mut Vec::new()).expect("failed to initialize test state");
    tmp
}

pub fn bytes_to_string(out: Vec<u8>) -> String {
    String::from_utf8(out).expect("command output must be valid UTF-8")
}
