use std::process::Command;

fn main() {
    // Re-run if git HEAD changes (new commit, checkout, etc.)
    println!("cargo:rerun-if-changed=.git/HEAD");
    println!("cargo:rerun-if-changed=.git/refs/");

    let version = git_version().unwrap_or_else(|| "unknown".to_string());
    let commit = git_commit_short().unwrap_or_else(|| "unknown".to_string());
    let date = git_commit_date().unwrap_or_else(|| "unknown".to_string());

    println!("cargo:rustc-env=VT_VERSION={}", version);
    println!("cargo:rustc-env=VT_COMMIT={}", commit);
    println!("cargo:rustc-env=VT_DATE={}", date);
}

/// If HEAD is tagged with v*, use that tag. Otherwise use the branch name.
fn git_version() -> Option<String> {
    // Try exact tag first (release builds)
    let output = Command::new("git")
        .args(["describe", "--tags", "--exact-match", "HEAD"])
        .output()
        .ok()?;
    if output.status.success() {
        let tag = String::from_utf8_lossy(&output.stdout).trim().to_string();
        if !tag.is_empty() {
            return Some(tag);
        }
    }

    // Fall back to branch name (local dev builds)
    let output = Command::new("git")
        .args(["rev-parse", "--abbrev-ref", "HEAD"])
        .output()
        .ok()?;
    if output.status.success() {
        let branch = String::from_utf8_lossy(&output.stdout).trim().to_string();
        if !branch.is_empty() {
            return Some(branch);
        }
    }

    None
}

fn git_commit_short() -> Option<String> {
    let output = Command::new("git")
        .args(["rev-parse", "--short", "HEAD"])
        .output()
        .ok()?;
    if output.status.success() {
        let hash = String::from_utf8_lossy(&output.stdout).trim().to_string();
        if !hash.is_empty() {
            return Some(hash);
        }
    }
    None
}

fn git_commit_date() -> Option<String> {
    let output = Command::new("git")
        .args(["log", "-1", "--format=%ci"])
        .output()
        .ok()?;
    if output.status.success() {
        let date = String::from_utf8_lossy(&output.stdout).trim().to_string();
        if !date.is_empty() {
            return Some(date);
        }
    }
    None
}
