//! What the run ran on: host, command line, git state. Captured on the background thread
//! so `init()` doesn't wait for it.

use std::path::Path;
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

use losscape_core::proto::v0::RunEnvironment;

const GIT_TIMEOUT: Duration = Duration::from_secs(3);

pub fn capture(python_version: &str, sdk_version: &str) -> RunEnvironment {
    let cwd = std::env::current_dir().unwrap_or_default();
    let commit = git(&cwd, &["rev-parse", "HEAD"]);
    RunEnvironment {
        hostname: gethostname::gethostname().to_string_lossy().into_owned(),
        command: redact_args(std::env::args()).join(" "),
        python_version: python_version.to_owned(),
        sdk_version: sdk_version.to_owned(),
        git_remote: git(&cwd, &["remote", "get-url", "origin"])
            .map(|u| strip_credentials(&u))
            .unwrap_or_default(),
        git_dirty: commit.is_some()
            && git(&cwd, &["status", "--porcelain", "--untracked-files=no"])
                .is_some_and(|s| !s.is_empty()),
        git_commit: commit.unwrap_or_default(),
    }
}

/// Output of a git command, or `None` if git is missing, fails, or hangs.
fn git(cwd: &Path, args: &[&str]) -> Option<String> {
    let mut child = Command::new("git")
        .args(args)
        .current_dir(cwd)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .ok()?;
    let started = Instant::now();
    loop {
        match child.try_wait().ok()? {
            Some(status) if status.success() => break,
            Some(_) => return None,
            None if started.elapsed() > GIT_TIMEOUT => {
                child.kill().ok();
                child.wait().ok();
                return None;
            }
            None => std::thread::sleep(Duration::from_millis(5)),
        }
    }
    let output = child.wait_with_output().ok()?;
    Some(String::from_utf8_lossy(&output.stdout).trim().to_owned())
}

/// `https://user:token@host/repo` -> `https://host/repo`.
fn strip_credentials(url: &str) -> String {
    match url.split_once("://") {
        Some((scheme, rest)) => match rest.split_once('@') {
            Some((userinfo, host)) if !userinfo.contains('/') => format!("{scheme}://{host}"),
            _ => url.to_owned(),
        },
        None => url.to_owned(),
    }
}

/// Hides values of arguments that look like secrets: `--api-key X`, `--token=X`, ...
fn redact_args(args: impl Iterator<Item = String>) -> Vec<String> {
    let secret = |name: &str| {
        let name = name.to_ascii_lowercase();
        ["key", "token", "secret", "password", "passwd"]
            .iter()
            .any(|s| name.contains(s))
    };
    let mut out = Vec::new();
    let mut hide_next = false;
    for arg in args {
        if hide_next {
            out.push("***".to_owned());
            hide_next = false;
        } else if let Some((name, _)) = arg
            .split_once('=')
            .filter(|(n, _)| n.starts_with('-') && secret(n))
        {
            out.push(format!("{name}=***"));
        } else {
            hide_next = arg.starts_with('-') && secret(&arg);
            out.push(arg);
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn strips_url_credentials() {
        assert_eq!(
            strip_credentials("https://bob:ghp_x@github.com/a/b.git"),
            "https://github.com/a/b.git"
        );
        assert_eq!(
            strip_credentials("git@github.com:a/b.git"),
            "git@github.com:a/b.git"
        );
        assert_eq!(
            strip_credentials("https://github.com/a/b"),
            "https://github.com/a/b"
        );
    }

    #[test]
    fn redacts_secret_arguments() {
        let args = [
            "train.py",
            "--lr",
            "0.1",
            "--api-key",
            "abc",
            "--hf-token=xyz",
            "--epochs",
            "3",
        ];
        assert_eq!(
            redact_args(args.iter().map(|s| s.to_string())),
            [
                "train.py",
                "--lr",
                "0.1",
                "--api-key",
                "***",
                "--hf-token=***",
                "--epochs",
                "3"
            ]
        );
    }

    #[test]
    fn captures_without_panicking() {
        let env = capture("3.12.0", "0.0.1");
        assert!(!env.hostname.is_empty());
        assert_eq!(env.python_version, "3.12.0");
    }
}
