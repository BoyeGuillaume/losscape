//! What the run ran on: host, command line, git state and diff, OS and hardware, GPUs and
//! driver, Slurm job, container, installed packages, selected environment variables.
//! Captured on the background thread so `init()` doesn't wait for it.

use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

use losscape_core::proto::v0::RunEnvironment;
use serde_json::{Map, Value, json};

use crate::redact::redact_str;

const GIT_TIMEOUT: Duration = Duration::from_secs(3);
/// Larger diffs are cut (the server keeps 2 MB).
const MAX_DIFF_BYTES: usize = 2 << 20;
const MAX_UNTRACKED: usize = 200;
const MAX_PACKAGES: usize = 5_000;

/// Environment variables recorded (by prefix, or exact name). Values of secret-looking
/// names are hidden.
const ENV_PREFIXES: &[&str] = &[
    "CUDA_",
    "NCCL_",
    "OMP_",
    "MKL_",
    "PYTORCH_",
    "TORCH_",
    "XLA_",
    "JAX_",
    "TF_",
    "HF_",
    "TRANSFORMERS_",
    "ACCELERATE_",
];
const ENV_NAMES: &[&str] = &[
    "WORLD_SIZE",
    "RANK",
    "LOCAL_RANK",
    "LOCAL_WORLD_SIZE",
    "MASTER_ADDR",
    "MASTER_PORT",
    "CONDA_DEFAULT_ENV",
    "VIRTUAL_ENV",
];

/// What the Python binding adds: where packages are installed, and Python-side details
/// (interpreter, versions of already imported frameworks).
#[derive(Clone, Debug, Default)]
pub struct Extra {
    pub python_path: Vec<PathBuf>,
    pub details: Map<String, Value>,
}

pub fn capture(python_version: &str, sdk_version: &str, extra: &Extra) -> RunEnvironment {
    let cwd = std::env::current_dir().unwrap_or_default();
    let commit = git(&cwd, &["rev-parse", "HEAD"], 1 << 10);
    let in_repo = commit.is_some();

    let mut details = extra.details.clone();
    details.insert("os".into(), os());
    details.insert("cpu".into(), cpu());
    if let Some(gpu) = gpu() {
        details.insert("gpu".into(), gpu);
    }
    let slurm = slurm();
    if !slurm.is_empty() {
        details.insert("slurm".into(), Value::Object(slurm));
    }
    if let Some(container) = container() {
        details.insert("container".into(), container);
    }
    let env = selected_env();
    if !env.is_empty() {
        details.insert("env".into(), Value::Object(env));
    }
    let packages = packages(&extra.python_path);
    if !packages.is_empty() {
        details.insert("packages".into(), json!(packages));
    }
    let mut git_diff = String::new();
    if in_repo {
        if let Some(branch) = git(&cwd, &["rev-parse", "--abbrev-ref", "HEAD"], 1 << 10) {
            details.insert("git_branch".into(), json!(branch));
        }
        if let Some(untracked) = git(
            &cwd,
            &["ls-files", "--others", "--exclude-standard"],
            1 << 20,
        ) {
            let files: Vec<&str> = untracked.lines().take(MAX_UNTRACKED).collect();
            if !files.is_empty() {
                details.insert("git_untracked".into(), json!(files));
            }
        }
        git_diff = git(
            &cwd,
            &["diff", "HEAD", "--no-color", "--no-ext-diff"],
            MAX_DIFF_BYTES,
        )
        .unwrap_or_default();
    }

    RunEnvironment {
        hostname: gethostname::gethostname().to_string_lossy().into_owned(),
        command: redact_args(std::env::args()).join(" "),
        python_version: python_version.to_owned(),
        sdk_version: sdk_version.to_owned(),
        git_remote: git(&cwd, &["remote", "get-url", "origin"], 1 << 12)
            .map(|u| strip_credentials(&u))
            .unwrap_or_default(),
        git_dirty: !git_diff.is_empty(),
        git_commit: commit.unwrap_or_default(),
        details_json: Value::Object(details).to_string(),
        git_diff,
    }
}

fn os() -> Value {
    json!({
        "name": sysinfo::System::name(),
        "version": sysinfo::System::long_os_version(),
        "kernel": sysinfo::System::kernel_version(),
        "arch": sysinfo::System::cpu_arch(),
    })
}

fn cpu() -> Value {
    let system = sysinfo::System::new_with_specifics(
        sysinfo::RefreshKind::nothing()
            .with_cpu(sysinfo::CpuRefreshKind::nothing())
            .with_memory(sysinfo::MemoryRefreshKind::nothing().with_ram()),
    );
    json!({
        "model": system.cpus().first().map(|c| c.brand().trim().to_owned()),
        "cores": sysinfo::System::physical_core_count(),
        "threads": std::thread::available_parallelism().map(|n| n.get()).ok(),
        "memory_gb": (system.total_memory() as f64 / (1u64 << 30) as f64 * 10.0).round() / 10.0,
    })
}

fn gpu() -> Option<Value> {
    let nvml = crate::system::nvml()?;
    let devices: Vec<Value> = (0..nvml.device_count().unwrap_or(0))
        .filter_map(|i| nvml.device_by_index(i).ok())
        .map(|d| {
            json!({
                "name": d.name().ok(),
                "memory_gb": d.memory_info().ok().map(|m| (m.total as f64 / (1u64 << 30) as f64).round()),
            })
        })
        .collect();
    let cuda = nvml.sys_cuda_driver_version().ok().map(|v| {
        format!(
            "{}.{}",
            nvml_wrapper::cuda_driver_version_major(v),
            nvml_wrapper::cuda_driver_version_minor(v)
        )
    });
    Some(json!({
        "driver": nvml.sys_driver_version().ok(),
        "cuda_driver": cuda,
        "devices": devices,
    }))
}

fn slurm() -> Map<String, Value> {
    [
        "SLURM_JOB_ID",
        "SLURM_JOB_NAME",
        "SLURM_ARRAY_JOB_ID",
        "SLURM_ARRAY_TASK_ID",
        "SLURM_PROCID",
        "SLURM_NTASKS",
        "SLURM_JOB_NODELIST",
        "SLURM_JOB_PARTITION",
        "SLURM_JOB_ACCOUNT",
        "SLURM_CLUSTER_NAME",
    ]
    .iter()
    .filter_map(|name| {
        let value = std::env::var(name).ok()?;
        let key = name
            .trim_start_matches("SLURM_")
            .trim_start_matches("JOB_")
            .to_ascii_lowercase();
        Some((key, json!(value)))
    })
    .collect()
}

/// Container runtime and image, when they can be told.
fn container() -> Option<Value> {
    let image = ["LOSSCAPE_CONTAINER_IMAGE", "CONTAINER_IMAGE"]
        .iter()
        .find_map(|name| std::env::var(name).ok());
    let apptainer = ["APPTAINER_CONTAINER", "SINGULARITY_CONTAINER"]
        .iter()
        .find_map(|name| std::env::var(name).ok());
    let (runtime, found_image) = if let Some(path) = apptainer {
        ("apptainer", Some(path))
    } else if let Ok(info) = std::fs::read_to_string("/run/.containerenv") {
        // Podman writes `image="..."` here (for privileged containers).
        let image = info.lines().find_map(|l| {
            l.strip_prefix("image=")
                .map(|v| v.trim_matches('"').to_owned())
        });
        ("podman", image.filter(|i| !i.is_empty()))
    } else if Path::new("/.dockerenv").exists() {
        ("docker", None)
    } else if std::env::var_os("KUBERNETES_SERVICE_HOST").is_some() {
        ("kubernetes", None)
    } else if image.is_some() {
        ("unknown", None)
    } else {
        return None;
    };
    Some(json!({"runtime": runtime, "image": image.or(found_image)}))
}

fn selected_env() -> Map<String, Value> {
    let mut vars: Vec<(String, String)> = std::env::vars()
        .filter(|(name, _)| {
            ENV_NAMES.contains(&name.as_str()) || ENV_PREFIXES.iter().any(|p| name.starts_with(p))
        })
        .collect();
    vars.sort();
    vars.into_iter()
        .map(|(name, value)| {
            let value = redact_str(&name, &value);
            (name, json!(value))
        })
        .collect()
}

/// `name==version` of the distributions installed in `path` (from `*.dist-info` and
/// `*.egg-info` directory names; nothing is imported or read).
fn packages(path: &[PathBuf]) -> Vec<String> {
    let mut found = std::collections::BTreeMap::new();
    for dir in path {
        let Ok(entries) = std::fs::read_dir(dir) else {
            continue;
        };
        for entry in entries.flatten() {
            let file = entry.file_name();
            let Some(file) = file.to_str() else { continue };
            let Some(stem) = file
                .strip_suffix(".dist-info")
                .or_else(|| file.strip_suffix(".egg-info"))
            else {
                continue;
            };
            // Egg-info names may end with the Python version: `name-1.0-py3.10`.
            let stem = match stem.rsplit_once("-py") {
                Some((rest, py)) if py.chars().all(|c| c.is_ascii_digit() || c == '.') => rest,
                _ => stem,
            };
            if let Some((name, version)) = stem.rsplit_once('-') {
                // Earlier path entries win, as for imports.
                found
                    .entry(name.replace('_', "-").to_ascii_lowercase())
                    .or_insert_with(|| version.to_owned());
            }
            if found.len() >= MAX_PACKAGES {
                break;
            }
        }
    }
    found
        .into_iter()
        .map(|(name, version)| format!("{name}=={version}"))
        .collect()
}

/// Output of a git command (at most `max` bytes), or `None` if git is missing, fails, or
/// hangs. Output is read while git runs, so a large diff can't fill the pipe and stall it.
fn git(cwd: &Path, args: &[&str], max: usize) -> Option<String> {
    let mut child = Command::new("git")
        .args(args)
        .current_dir(cwd)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .ok()?;
    let mut stdout = child.stdout.take()?;
    let reader = std::thread::spawn(move || {
        let mut out = Vec::new();
        (&mut stdout).take(max as u64).read_to_end(&mut out).ok();
        // Drain the rest so git can exit.
        std::io::copy(&mut stdout, &mut std::io::sink()).ok();
        out
    });
    let started = Instant::now();
    let success = loop {
        match child.try_wait().ok()? {
            Some(status) => break status.success(),
            None if started.elapsed() > GIT_TIMEOUT => {
                child.kill().ok();
                child.wait().ok();
                return None;
            }
            None => std::thread::sleep(Duration::from_millis(5)),
        }
    };
    let out = reader.join().ok()?;
    success.then(|| {
        let text = String::from_utf8_lossy(&out);
        let text = if out.len() >= max {
            format!("{text}\n[truncated]\n")
        } else {
            text.into_owned()
        };
        text.trim_end().to_owned()
    })
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
    fn lists_packages_from_dist_info_names() {
        let dir = tempfile::tempdir().unwrap();
        for name in [
            "torch-2.5.1.dist-info",
            "typing_extensions-4.12.2.dist-info",
            "old_pkg-0.1-py3.10.egg-info",
            "torch",
            "README.txt",
        ] {
            std::fs::create_dir(dir.path().join(name)).unwrap();
        }
        assert_eq!(
            packages(&[dir.path().to_owned(), PathBuf::from("/does/not/exist")]),
            ["old-pkg==0.1", "torch==2.5.1", "typing-extensions==4.12.2"]
        );
    }

    #[test]
    fn reads_diffs_larger_than_a_pipe_buffer() {
        let repo = tempfile::tempdir().unwrap();
        let run = |args: &[&str]| {
            Command::new("git")
                .args([
                    "-c",
                    "user.name=t",
                    "-c",
                    "user.email=t@t",
                    "-c",
                    "commit.gpgsign=false",
                ])
                .args(args)
                .current_dir(repo.path())
                .output()
                .is_ok_and(|o| o.status.success())
        };
        if !run(&["init", "-q"]) {
            return; // no git here
        }
        std::fs::write(repo.path().join("f.txt"), "a\n").unwrap();
        assert!(run(&["add", "f.txt"]) && run(&["commit", "-qm", "init"]));
        std::fs::write(repo.path().join("f.txt"), "changed line\n".repeat(20_000)).unwrap();

        let diff = git(repo.path(), &["diff", "HEAD"], MAX_DIFF_BYTES).unwrap();
        assert!(diff.len() > 200_000, "{} bytes", diff.len());
        let cut = git(repo.path(), &["diff", "HEAD"], 1_000).unwrap();
        assert!(cut.ends_with("[truncated]"));
        assert_eq!(git(repo.path(), &["no-such-command"], 1 << 10), None);
    }

    #[test]
    fn captures_without_panicking() {
        let env = capture("3.12.0", "0.0.1", &Extra::default());
        assert!(!env.hostname.is_empty());
        assert_eq!(env.python_version, "3.12.0");
        let details: Value = serde_json::from_str(&env.details_json).unwrap();
        assert!(details["cpu"]["threads"].as_u64().unwrap() >= 1);
        assert!(details["os"]["arch"].is_string());
    }
}
