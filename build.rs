fn main() {
    let version = std::process::Command::new("git")
        .args(["describe", "--exact-match", "--tags", "HEAD"])
        .output()
        .ok()
        .filter(|o| o.status.success())
        .and_then(|o| String::from_utf8(o.stdout).ok())
        .map(|tag| {
            tag.trim()
                .strip_prefix('v')
                .unwrap_or(tag.trim())
                .to_owned()
        })
        .unwrap_or_else(|| "0.0.0-dev".to_owned());

    println!("cargo:rustc-env=LEITER_VERSION={version}");
    println!("cargo:rerun-if-changed=.git/HEAD");
}
