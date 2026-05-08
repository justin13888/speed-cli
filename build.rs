use std::process::Command;

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

    // Rerun when HEAD or refs change so the captured commit stays
    // current. `.git/refs/heads` covers branch-scoped commits;
    // `.git/HEAD` covers branch switches; `.git/index` covers staging.
    println!("cargo:rerun-if-changed=.git/HEAD");
    println!("cargo:rerun-if-changed=.git/index");
    println!("cargo:rerun-if-changed=.git/refs/heads");
}

fn git_output(args: &[&str]) -> Option<String> {
    git_command(args).map(|s| s.trim().to_string()).filter(|s| !s.is_empty())
}

fn git_command(args: &[&str]) -> Option<String> {
    let out = Command::new("git").args(args).output().ok()?;
    if !out.status.success() {
        return None;
    }
    String::from_utf8(out.stdout).ok()
}
