use std::path::PathBuf;
use std::process::{Command, Output, Stdio};
use tracing::info;

pub struct RemoteHost {
    ssh_dest: String,
    target_triple: String,
    control_path: PathBuf,
    // Held for RAII cleanup — the directory is removed on drop, which must
    // happen after the control socket file inside it is closed.
    _control_dir: tempfile::TempDir,
}

impl RemoteHost {
    /// Read configuration from environment variables and auto-detect the remote
    /// target triple if not explicitly set. Returns None if LEITER_E2E_DEST is
    /// not set, allowing the test to skip gracefully.
    pub fn from_env() -> Option<Self> {
        let ssh_dest = match std::env::var("LEITER_E2E_DEST") {
            Ok(v) => v,
            Err(_) => return None,
        };

        let target_triple = match std::env::var("LEITER_E2E_TARGET") {
            Ok(t) => t,
            Err(_) => {
                let uname = ssh_run_ok(&ssh_dest, "uname -sm");
                detect_target(&uname)
            }
        };

        // Multiplex all SSH connections over a single persistent TCP connection.
        // Without this, the many rapid SSH invocations across steps exhaust the
        // server's MaxSessions/MaxStartups limits, causing "Connection closed" drops.
        // Also eliminates per-command TCP+SSH handshake overhead.
        let control_dir =
            tempfile::tempdir().expect("failed to create tempdir for SSH control socket");
        let control_path = control_dir.path().join("ctrl");

        let status = Command::new("ssh")
            .args([
                "-o",
                "ControlMaster=yes",
                "-o",
                &format!("ControlPath={}", control_path.display()),
                "-o",
                "ControlPersist=yes",
                "-o",
                "ConnectTimeout=10",
                "-N",
                "-f",
                &ssh_dest,
            ])
            .status()
            .expect("failed to start SSH control master");
        assert!(status.success(), "SSH control master failed to start");

        info!(
            ssh_dest,
            target_triple, "remote host configured (SSH multiplexing enabled)"
        );
        Some(Self {
            ssh_dest,
            target_triple,
            control_path,
            _control_dir: control_dir,
        })
    }

    fn ssh_control_args(&self) -> [String; 2] {
        [
            "-o".to_string(),
            format!("ControlPath={}", self.control_path.display()),
        ]
    }

    /// Run a command on the remote host via SSH.
    ///
    /// Prepends ~/.local/bin to PATH since `ssh host "cmd"` runs a
    /// non-login non-interactive shell that won't source .profile or
    /// the useful parts of .bashrc (most distros guard on interactive).
    ///
    /// Retries once on SSH transport errors (exit code 255).
    pub fn run(&self, cmd: &str) -> Output {
        let wrapped = format!("export PATH=\"$HOME/.local/bin:$PATH\" && {cmd}");
        for attempt in 0..2 {
            let output = Command::new("ssh")
                .args(self.ssh_control_args())
                .args(["-o", "ConnectTimeout=10", &self.ssh_dest, &wrapped])
                .output()
                .unwrap_or_else(|e| panic!("ssh failed to execute: {e}"));

            if output.status.code() == Some(255) && attempt == 0 {
                let stderr = String::from_utf8_lossy(&output.stderr);
                info!(stderr = %stderr, "SSH transport error, retrying in 2s");
                std::thread::sleep(std::time::Duration::from_secs(2));
                continue;
            }
            return output;
        }
        unreachable!()
    }

    /// Run a command on the remote host, assert success, return stdout.
    pub fn run_ok(&self, cmd: &str) -> String {
        let output = self.run(cmd);
        assert!(
            output.status.success(),
            "Command failed: {cmd}\nstdout: {}\nstderr: {}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr),
        );
        String::from_utf8(output.stdout).expect("non-UTF8 stdout")
    }

    /// Copy a local file to a remote path.
    pub fn scp_to(&self, local: &str, remote: &str) {
        let dest = format!("{}:{remote}", self.ssh_dest);
        let status = Command::new("scp")
            .args(self.ssh_control_args())
            .args(["-o", "ConnectTimeout=10", local, &dest])
            .status()
            .unwrap_or_else(|e| panic!("scp failed to execute: {e}"));
        assert!(status.success(), "scp {local} -> {dest} failed");
    }

    /// Read a file on the remote host.
    pub fn read_file(&self, path: &str) -> String {
        self.run_ok(&format!("cat {path}"))
    }

    /// Check if a file exists on the remote host.
    pub fn file_exists(&self, path: &str) -> bool {
        let output = self.run_ok(&format!("test -f {path} && echo yes || echo no"));
        output.trim() == "yes"
    }

    /// Run a `claude -p` prompt on the remote host with a timeout.
    ///
    /// Always logs full stdout and stderr for debuggability.
    pub fn claude_prompt(&self, prompt: &str, max_turns: u32) -> Output {
        let escaped = prompt.replace('\'', "'\\''");
        let cmd = format!(
            "timeout 180 claude -p --max-turns {max_turns} --dangerously-skip-permissions '{escaped}'"
        );
        let output = self.run(&cmd);
        let stdout = String::from_utf8_lossy(&output.stdout);
        let stderr = String::from_utf8_lossy(&output.stderr);
        info!(
            status = %output.status,
            stdout = %stdout,
            stderr = %stderr,
            "claude prompt: {prompt}"
        );
        output
    }

    /// Run a `claude -p` prompt, assert it succeeds, return stdout.
    pub fn claude_prompt_ok(&self, prompt: &str, max_turns: u32) -> String {
        let output = self.claude_prompt(prompt, max_turns);
        assert!(
            output.status.success(),
            "claude prompt failed: {prompt}\nstdout: {}\nstderr: {}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr),
        );
        String::from_utf8(output.stdout).expect("non-UTF8 stdout")
    }

    /// Check if Claude is authenticated on the remote. If not, open an
    /// interactive SSH session so the user can complete the login flow.
    fn ensure_claude_auth(&self) {
        info!("probing claude auth");
        let probe =
            self.run("timeout 30 claude -p --max-turns 1 --dangerously-skip-permissions 'say ok'");
        if probe.status.success() {
            info!("claude is authenticated");
            return;
        }

        info!(
            "basic claude invocation failed on {}; assuming unauthenticated",
            self.ssh_dest
        );
        eprintln!("Press Enter to open Claude Code for interactive login...");
        let mut buf = String::new();
        std::io::stdin()
            .read_line(&mut buf)
            .expect("failed to read stdin");

        let status = Command::new("ssh")
            .args(self.ssh_control_args())
            .args([
                "-t",
                "-o",
                "ConnectTimeout=10",
                &self.ssh_dest,
                "export PATH=\"$HOME/.local/bin:$PATH\" && claude",
            ])
            .stdin(Stdio::inherit())
            .stdout(Stdio::inherit())
            .stderr(Stdio::inherit())
            .status()
            .expect("failed to launch interactive ssh");

        assert!(
            status.success(),
            "Interactive SSH session exited with non-zero status"
        );

        info!("re-probing claude auth");
        let retry =
            self.run("timeout 30 claude -p --max-turns 1 --dangerously-skip-permissions 'say ok'");
        assert!(
            retry.status.success(),
            "Claude is still not authenticated after interactive session.\nstderr: {}",
            String::from_utf8_lossy(&retry.stderr),
        );
    }

    /// Build the leiter binary for the remote target.
    pub fn build_binary(&self) -> String {
        info!(target = self.target_triple, "building leiter");
        let status = Command::new("cargo")
            .args(["build", "--target", &self.target_triple, "--release"])
            .status()
            .expect("cargo build failed to execute");
        assert!(
            status.success(),
            "cargo build --target {} failed",
            self.target_triple
        );

        let binary_path = format!("target/{}/release/leiter", self.target_triple);
        assert!(
            std::path::Path::new(&binary_path).exists(),
            "Expected binary at {binary_path}"
        );
        binary_path
    }

    /// Clean leiter state and deploy a fresh binary to the remote host.
    pub fn setup(&self) {
        let binary_path = self.build_binary();

        self.run_ok("mkdir -p ~/.local/bin");

        // Ensure ~/.local/bin is on PATH for login shells (e.g. interactive SSH).
        // Non-login SSH commands get PATH from run() directly.
        let profile_check = self.run("grep -q 'local/bin' ~/.profile");
        if !profile_check.status.success() {
            self.run_ok(r#"echo 'export PATH="$HOME/.local/bin:$PATH"' >> ~/.profile"#);
        }

        if !self.run("command -v claude").status.success() {
            info!("installing claude code");
            self.run_ok(
                "npm config set prefix ~/.local && npm install -g @anthropic-ai/claude-code@latest",
            );
        }

        self.ensure_claude_auth();

        info!("deploying binary to remote");
        self.scp_to(&binary_path, "~/.local/bin/leiter");
        self.run_ok("chmod +x ~/.local/bin/leiter");

        info!("cleaning prior leiter state");
        self.run("rm -rf ~/.leiter");
        self.run("rm -rf ~/.claude/skills/leiter-*");

        // Strip leiter hooks from settings.json if present
        self.run(
            r#"if [ -f ~/.claude/settings.json ]; then python3 -c "
import json, sys
try:
    s = json.load(open(sys.argv[1]))
except: sys.exit(0)
hooks = s.get('hooks', {})
for event in list(hooks.keys()):
    groups = hooks[event]
    filtered = []
    for g in groups:
        inner = [h for h in g.get('hooks', []) if 'leiter' not in h.get('command', '')]
        if inner:
            g['hooks'] = inner
            filtered.append(g)
    if filtered:
        hooks[event] = filtered
    else:
        del hooks[event]
if not hooks and 'hooks' in s: del s['hooks']
perms = s.get('permissions', {})
allow = perms.get('allow', [])
allow = [a for a in allow if 'leiter' not in a and '.leiter/' not in a]
if allow: perms['allow'] = allow
elif 'allow' in perms: del perms['allow']
if not perms and 'permissions' in s: del s['permissions']
json.dump(s, open(sys.argv[1], 'w'), indent=2)
print()
" ~/.claude/settings.json; fi"#,
        );

        info!("running leiter claude install");
        let install_output = self.run("~/.local/bin/leiter claude install");
        let install_stdout = String::from_utf8_lossy(&install_output.stdout);
        let install_stderr = String::from_utf8_lossy(&install_output.stderr);
        info!(stdout = %install_stdout, stderr = %install_stderr, "install output");
        assert!(
            install_output.status.success(),
            "leiter claude install failed:\nstdout: {install_stdout}\nstderr: {install_stderr}",
        );

        let skills_listing =
            self.run_ok("ls -la ~/.claude/skills/ 2>&1 || echo 'skills dir missing'");
        info!(skills = %skills_listing, "post-install skills directory");
        info!("setup complete");
    }
}

impl Drop for RemoteHost {
    fn drop(&mut self) {
        let _ = Command::new("ssh")
            .args([
                "-o",
                &format!("ControlPath={}", self.control_path.display()),
                "-O",
                "exit",
                &self.ssh_dest,
            ])
            .output();
    }
}

/// Run a one-off SSH command. Used during `from_env()` to auto-detect the
/// remote target triple before the `RemoteHost` struct exists.
fn ssh_run_ok(dest: &str, cmd: &str) -> String {
    let output = Command::new("ssh")
        .args(["-o", "ConnectTimeout=10", dest, cmd])
        .output()
        .unwrap_or_else(|e| panic!("ssh failed: {e}"));
    assert!(
        output.status.success(),
        "ssh {dest} {cmd} failed:\n{}",
        String::from_utf8_lossy(&output.stderr),
    );
    String::from_utf8(output.stdout)
        .expect("non-UTF8 output")
        .trim()
        .to_string()
}

fn detect_target(uname: &str) -> String {
    match uname.trim() {
        "Linux x86_64" => "x86_64-unknown-linux-musl".to_string(),
        "Linux aarch64" => "aarch64-unknown-linux-musl".to_string(),
        "Darwin x86_64" => "x86_64-apple-darwin".to_string(),
        "Darwin arm64" => "aarch64-apple-darwin".to_string(),
        other => panic!("Unsupported remote platform: {other}"),
    }
}
