use std::env;
use std::fs;
use std::path::Path;
use std::process::Command;
use walkdir::WalkDir;

fn main() {
    // Define source directories
    let tiecc_src = Path::new("src/tiecc");
    let stdlib_src = Path::new("src/stdlib");

    // Define output manifest path in OUT_DIR
    let out_dir = env::var("OUT_DIR").unwrap();
    let manifest_path = Path::new(&out_dir).join("assets_manifest.rs");

    let mut entries: Vec<String> = Vec::new();

    let mut add_entries = |base: &Path, prefix: &str| {
        if base.exists() {
            for entry in WalkDir::new(base) {
                let entry = match entry {
                    Ok(v) => v,
                    Err(_) => continue,
                };
                let path = entry.path();
                if path.is_dir() {
                    continue;
                }

                let rel = match path.strip_prefix(base) {
                    Ok(v) => v,
                    Err(_) => continue,
                };
                let logical_path = Path::new(prefix)
                    .join(rel)
                    .to_string_lossy()
                    .replace('\\', "/");

                let abs_path = match fs::canonicalize(path) {
                    Ok(v) => v,
                    Err(_) => continue,
                };
                let abs_path_str = abs_path.to_string_lossy().replace('\\', "/");

                #[cfg(unix)]
                let unix_mode = fs::metadata(path)
                    .ok()
                    .map(|m| std::os::unix::fs::PermissionsExt::mode(&m.permissions()));
                #[cfg(not(unix))]
                let unix_mode: Option<u32> = None;

                let entry_str = format!(
                    "    EmbeddedAsset {{ path: \"{}\", data: include_bytes!(r#\"{}\"#), unix_mode: {} }},\n",
                    logical_path,
                    abs_path_str,
                    match unix_mode {
                        Some(m) => format!("Some({})", m),
                        None => "None".to_string(),
                    }
                );
                entries.push(entry_str);
            }
        }
    };

    add_entries(tiecc_src, "tiecc");
    add_entries(stdlib_src, "stdlib");

    let mut manifest = String::new();
    manifest.push_str("pub static ASSETS: &[EmbeddedAsset] = &[\n");
    for e in entries {
        manifest.push_str(&e);
    }
    manifest.push_str("];\n");

    fs::write(&manifest_path, manifest).unwrap();

    // Rerun if these directories change
    println!("cargo:rerun-if-changed=src/tiecc");
    println!("cargo:rerun-if-changed=src/stdlib");
    println!("cargo:rerun-if-changed=build.rs");

    // Set build information for CI/CD
    let build_time = chrono::Utc::now().to_rfc3339();
    println!("cargo:rustc-env=BUILD_TIME={}", build_time);

    // Get git commit hash
    let git_commit = Command::new("git")
        .args(&["rev-parse", "HEAD"])
        .output()
        .ok()
        .and_then(|output| String::from_utf8(output.stdout).ok())
        .map(|s| s.trim().to_string())
        .unwrap_or_else(|| "unknown".to_string());
    println!("cargo:rustc-env=GIT_COMMIT={}", git_commit);

    // Get git branch
    let git_branch = Command::new("git")
        .args(&["rev-parse", "--abbrev-ref", "HEAD"])
        .output()
        .ok()
        .and_then(|output| String::from_utf8(output.stdout).ok())
        .map(|s| s.trim().to_string())
        .unwrap_or_else(|| "unknown".to_string());
    println!("cargo:rustc-env=GIT_BRANCH={}", git_branch);
}
