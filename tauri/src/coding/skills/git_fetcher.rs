use std::path::Path;
use std::process::Command;
use std::process::Stdio;
use std::sync::OnceLock;
use std::sync::RwLock;
use std::time::{Duration, Instant};

use anyhow::{Context, Result};

/// Thread-safe storage for proxy URL
static PROXY_URL: OnceLock<RwLock<Option<String>>> = OnceLock::new();

/// Set the proxy URL to be used for git operations
pub fn set_proxy(proxy_url: Option<String>) {
    let storage = PROXY_URL.get_or_init(|| RwLock::new(None));
    if let Ok(mut guard) = storage.write() {
        *guard = proxy_url.filter(|s| !s.is_empty());
    }
}

/// Get the current proxy URL
fn get_proxy() -> Option<String> {
    PROXY_URL
        .get()
        .and_then(|storage| storage.read().ok())
        .and_then(|guard| guard.clone())
}

fn redact_proxy_for_log(proxy: &str) -> String {
    if let Some(pos) = proxy.find('@') {
        let head = &proxy[..pos];
        if head.contains("://") {
            if let Some(scheme_pos) = head.find("://") {
                return format!("{}://***:***@{}", &head[..scheme_pos], &proxy[pos + 1..]);
            }
        }
    }
    proxy.to_string()
}

/// Clone or pull a git repository
pub fn clone_or_pull(repo_url: &str, dest: &Path, branch: Option<&str>) -> Result<String> {
    // Prefer the system `git` binary if available
    if let Some(git_bin) = resolve_git_bin() {
        let started = Instant::now();
        match clone_or_pull_via_git_cli(repo_url, dest, branch) {
            Ok(head) => {
                log::info!(
                    "[git_fetcher] git-cli ok (bin={}) {}s url={}",
                    git_bin,
                    started.elapsed().as_secs_f32(),
                    repo_url
                );
                return Ok(head);
            }
            Err(err) => {
                log::warn!(
                    "[git_fetcher] git-cli failed (bin={}) {}s url={} err={:#}",
                    git_bin,
                    started.elapsed().as_secs_f32(),
                    repo_url,
                    err
                );
                anyhow::bail!("GIT_COMMAND_FAILED|{:#}", err);
            }
        }
    } else {
        anyhow::bail!("GIT_NOT_FOUND");
    }
}

fn git_timeout() -> Duration {
    let secs = std::env::var("SKILLS_GIT_TIMEOUT_SECS")
        .ok()
        .and_then(|v| v.parse::<u64>().ok())
        .unwrap_or(300);
    Duration::from_secs(secs)
}

fn git_fetch_timeout() -> Duration {
    let secs = std::env::var("SKILLS_GIT_FETCH_TIMEOUT_SECS")
        .ok()
        .and_then(|v| v.parse::<u64>().ok())
        .unwrap_or(180);
    Duration::from_secs(secs)
}

static GIT_BIN: OnceLock<Option<String>> = OnceLock::new();

fn resolve_git_bin() -> Option<String> {
    GIT_BIN
        .get_or_init(|| {
            // Allow overriding from environment
            for key in ["SKILLS_GIT_BIN", "SKILLS_GIT_PATH"] {
                if let Ok(v) = std::env::var(key) {
                    let v = v.trim().to_string();
                    if !v.is_empty() && git_bin_works(&v) {
                        log::info!("[git_fetcher] using git bin from {}: {}", key, v);
                        return Some(v);
                    }
                }
            }

            // Try PATH lookup first
            if git_bin_works("git") {
                log::info!("[git_fetcher] using git bin from PATH: git");
                return Some("git".to_string());
            }

            // Common macOS locations
            for cand in [
                "/usr/bin/git",
                "/opt/homebrew/bin/git",
                "/usr/local/bin/git",
            ] {
                if git_bin_works(cand) {
                    log::info!("[git_fetcher] using git bin: {}", cand);
                    return Some(cand.to_string());
                }
            }

            log::warn!("[git_fetcher] no usable git binary found");
            None
        })
        .clone()
}

fn git_bin_works(bin: &str) -> bool {
    Command::new(bin)
        .arg("--version")
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false)
}

fn git_cmd() -> Command {
    let bin = resolve_git_bin().unwrap_or_else(|| "git".to_string());
    let mut cmd = Command::new(bin);
    // Never block on interactive auth prompts
    cmd.env("GIT_TERMINAL_PROMPT", "0")
        .env("GIT_ASKPASS", "echo");
    // Abort stalled HTTPS transfers
    cmd.env("GIT_HTTP_LOW_SPEED_LIMIT", "1024")
        .env("GIT_HTTP_LOW_SPEED_TIME", "120");

    // Apply proxy settings if configured
    if let Some(proxy_url) = get_proxy() {
        log::info!(
            "[git_fetcher] using proxy: {}",
            redact_proxy_for_log(&proxy_url)
        );
        cmd.env("HTTP_PROXY", &proxy_url)
            .env("HTTPS_PROXY", &proxy_url)
            .env("http_proxy", &proxy_url)
            .env("https_proxy", &proxy_url);
    }

    cmd
}

fn run_cmd_with_timeout(
    mut cmd: Command,
    timeout: Duration,
    context: String,
) -> Result<std::process::Output> {
    cmd.stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());

    let mut child = cmd.spawn().with_context(|| context.clone())?;
    let start = Instant::now();
    loop {
        if start.elapsed() > timeout {
            let _ = child.kill();
            let stderr = child
                .wait_with_output()
                .map(|out| String::from_utf8_lossy(&out.stderr).to_string())
                .unwrap_or_default();
            anyhow::bail!("GIT_TIMEOUT|{}|{}", timeout.as_secs(), stderr.trim());
        }

        match child.try_wait() {
            Ok(Some(_)) => return child.wait_with_output().with_context(|| context.clone()),
            Ok(None) => std::thread::sleep(Duration::from_millis(200)),
            Err(err) => return Err(err).with_context(|| context.clone()),
        }
    }
}

fn clone_or_pull_via_git_cli(repo_url: &str, dest: &Path, branch: Option<&str>) -> Result<String> {
    // Ensure parent exists
    if let Some(parent) = dest.parent() {
        std::fs::create_dir_all(parent)
            .with_context(|| format!("failed to create parent dir {:?}", parent))?;
    }

    if dest.exists() {
        // Fetch updates
        let out = run_cmd_with_timeout(
            {
                let mut cmd = git_cmd();
                cmd.arg("-C").arg(dest).args(["fetch", "--prune", "origin"]);
                cmd
            },
            git_fetch_timeout(),
            format!("git fetch in {:?}", dest),
        )?;
        if !out.status.success() {
            let stderr = String::from_utf8_lossy(&out.stderr);
            anyhow::bail!("GIT_FETCH_FAILED|{}", stderr);
        }

        // Move local HEAD to fetched commit
        if let Some(branch) = branch {
            let out = run_cmd_with_timeout(
                {
                    let mut cmd = git_cmd();
                    cmd.arg("-C").arg(dest).args([
                        "checkout",
                        "-B",
                        branch,
                        &format!("origin/{}", branch),
                    ]);
                    cmd
                },
                git_fetch_timeout(),
                format!("git checkout -B {} in {:?}", branch, dest),
            )?;
            if !out.status.success() {
                let stderr = String::from_utf8_lossy(&out.stderr);
                anyhow::bail!("GIT_CHECKOUT_FAILED|{}|{}", branch, stderr);
            }
        } else {
            let out = run_cmd_with_timeout(
                {
                    let mut cmd = git_cmd();
                    cmd.arg("-C")
                        .arg(dest)
                        .args(["reset", "--hard", "FETCH_HEAD"]);
                    cmd
                },
                git_fetch_timeout(),
                format!("git reset --hard in {:?}", dest),
            )?;
            if !out.status.success() {
                let stderr = String::from_utf8_lossy(&out.stderr);
                anyhow::bail!("GIT_RESET_FAILED|{}", stderr);
            }
        }
    } else {
        // Clone
        let mut cmd = git_cmd();
        cmd.arg("clone")
            .args(["--depth", "1", "--filter=blob:none", "--no-tags"]);
        if let Some(branch) = branch {
            cmd.arg("--branch").arg(branch).arg("--single-branch");
        }
        cmd.arg(repo_url).arg(dest);
        let out = run_cmd_with_timeout(
            cmd,
            git_timeout(),
            format!("git clone {} into {:?}", repo_url, dest),
        )?;
        if !out.status.success() {
            let stderr = String::from_utf8_lossy(&out.stderr);
            anyhow::bail!("GIT_CLONE_FAILED|{}|{}", repo_url, stderr);
        }
    }

    // Checkout desired branch if specified
    if let Some(branch) = branch {
        let out = run_cmd_with_timeout(
            {
                let mut cmd = git_cmd();
                cmd.arg("-C").arg(dest).args(["checkout", branch]);
                cmd
            },
            git_fetch_timeout(),
            format!("git checkout {} in {:?}", branch, dest),
        )?;
        if !out.status.success() {
            let stderr = String::from_utf8_lossy(&out.stderr);
            if !stderr.trim().is_empty() {
                log::warn!("[git_fetcher] checkout warning: {}", stderr);
            }
        }
    }

    // Read HEAD revision
    let out = run_cmd_with_timeout(
        {
            let mut cmd = git_cmd();
            cmd.arg("-C").arg(dest).args(["rev-parse", "HEAD"]);
            cmd
        },
        git_fetch_timeout(),
        format!("git rev-parse HEAD in {:?}", dest),
    )?;
    if !out.status.success() {
        let stderr = String::from_utf8_lossy(&out.stderr);
        anyhow::bail!("GIT_REVPARSE_FAILED|{}", stderr);
    }
    Ok(String::from_utf8_lossy(&out.stdout).trim().to_string())
}
