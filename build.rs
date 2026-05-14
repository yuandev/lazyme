use std::process::Command;

fn main() {
    // Build frontend if npm is available
    let frontend_dir = std::path::Path::new("frontend");
    if frontend_dir.join("package.json").exists()
        && !frontend_dir.join("dist/index.html").exists()
    {
        println!("cargo:warning=Building frontend...");
        let status = Command::new("npm")
            .args(["run", "build"])
            .current_dir(frontend_dir)
            .status();
        match status {
            Ok(s) if s.success() => println!("cargo:warning=Frontend built successfully"),
            Ok(s) => println!("cargo:warning=Frontend build failed with exit code: {}", s),
            Err(e) => println!("cargo:warning=Frontend build error: {e}. Install Node.js or run 'npm run build' in frontend/"),
        }
    }

    // Embed git commit hash for the /api/version endpoint
    let hash = Command::new("git")
        .args(["rev-parse", "--short", "HEAD"])
        .output()
        .ok()
        .and_then(|o| if o.status.success() { String::from_utf8(o.stdout).ok() } else { None })
        .map(|s| s.trim().to_string())
        .unwrap_or_else(|| "unknown".into());

    println!("cargo:rustc-env=GIT_COMMIT_HASH={hash}");
    println!("cargo:rustc-env=TARGET={}", std::env::var("TARGET").unwrap());
    println!("cargo:rerun-if-changed=frontend/src/");
    println!("cargo:rerun-if-changed=frontend/package.json");
}
