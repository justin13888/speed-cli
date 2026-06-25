use std::process::Command;
use std::time::{SystemTime, UNIX_EPOCH};

fn main() {
    // Capture the git commit at build time. Falls back silently when
    // there's no `.git` directory (e.g. crates.io publish, tarball
    // build), in which case `option_env!("GIT_COMMIT")` returns `None`.
    if let Some(commit) = git_output(&["rev-parse", "HEAD"]) {
        println!("cargo:rustc-env=GIT_COMMIT={commit}");
    }

    // Dirty-state probe: empty `git status --porcelain` ⇒ clean.
    // We emit one of {true, false, unknown} so the runtime can
    // distinguish "no git" from "clean" from "dirty".
    let dirty_state = match git_command(&["status", "--porcelain"]) {
        Some(stdout) => {
            if stdout.trim().is_empty() {
                "false"
            } else {
                "true"
            }
        }
        None => "unknown",
    };
    println!("cargo:rustc-env=GIT_DIRTY={dirty_state}");

    // Build profile: cargo exports `PROFILE` to build scripts as
    // `debug` or `release`. Anything else (unset) ⇒ `unknown`.
    let profile = std::env::var("PROFILE").unwrap_or_else(|_| "unknown".to_string());
    println!("cargo:rustc-env=BUILD_PROFILE={profile}");

    // rustc toolchain: ask the exact compiler cargo is driving us with
    // (`RUSTC` env var) for its version banner, falling back to whatever
    // `rustc` is on PATH. `unknown` if the probe fails entirely.
    let rustc = std::env::var("RUSTC").unwrap_or_else(|_| "rustc".to_string());
    let rustc_version = Command::new(&rustc)
        .arg("--version")
        .output()
        .ok()
        .filter(|out| out.status.success())
        .and_then(|out| String::from_utf8(out.stdout).ok())
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| "unknown".to_string());
    println!("cargo:rustc-env=BUILD_RUSTC={rustc_version}");

    // Build timestamp, emitted as Unix seconds so build.rs stays
    // dependency-free (the runtime formats it via chrono). Honours
    // `SOURCE_DATE_EPOCH` for reproducible builds, else wall-clock now.
    let build_epoch = std::env::var("SOURCE_DATE_EPOCH")
        .ok()
        .and_then(|s| s.trim().parse::<u64>().ok())
        .or_else(|| {
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .ok()
                .map(|d| d.as_secs())
        })
        .unwrap_or(0);
    println!("cargo:rustc-env=BUILD_UNIX_TIME={build_epoch}");

    // Rerun when HEAD or refs change so the captured commit stays
    // current. `.git/refs/heads` covers branch-scoped commits;
    // `.git/HEAD` covers branch switches; `.git/index` covers staging.
    // `SOURCE_DATE_EPOCH` changes restamp the build time.
    println!("cargo:rerun-if-changed=.git/HEAD");
    println!("cargo:rerun-if-changed=.git/index");
    println!("cargo:rerun-if-changed=.git/refs/heads");
    println!("cargo:rerun-if-env-changed=SOURCE_DATE_EPOCH");
}

fn git_output(args: &[&str]) -> Option<String> {
    git_command(args)
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
}

fn git_command(args: &[&str]) -> Option<String> {
    let out = Command::new("git").args(args).output().ok()?;
    if !out.status.success() {
        return None;
    }
    String::from_utf8(out.stdout).ok()
}
