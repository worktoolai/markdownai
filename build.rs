use std::process::Command;

fn main() {
    let version = Command::new("git")
        .args(["describe", "--tags", "--always"])
        .output()
        .ok()
        .filter(|o| o.status.success())
        .map(|o| {
            let s = String::from_utf8_lossy(&o.stdout).trim().to_string();
            s.strip_prefix('v').unwrap_or(&s).to_string()
        })
        .unwrap_or_else(|| env!("CARGO_PKG_VERSION").into());

    println!("cargo:rustc-env=GIT_VERSION={version}");
}
