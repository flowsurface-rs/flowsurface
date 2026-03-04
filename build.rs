use std::process::Command;

fn main() {
    // Capture build timestamp in local timezone (macOS `date` command).
    // Format: "2026-03-03 18:23 PDT"
    let output = Command::new("date")
        .arg("+%Y-%m-%d %H:%M %Z")
        .output()
        .expect("failed to run date command");

    let build_time = String::from_utf8(output.stdout)
        .expect("invalid UTF-8 from date")
        .trim()
        .to_string();

    println!("cargo:rustc-env=FLOWSURFACE_BUILD_TIME={build_time}");

    // Extract resolved opendeviationbar-core version from Cargo.lock.
    // Cargo.lock format:
    //   [[package]]
    //   name = "opendeviationbar-core"
    //   version = "12.43.1"
    let lock = std::fs::read_to_string("Cargo.lock").expect("failed to read Cargo.lock");
    let odb_version = extract_package_version(&lock, "opendeviationbar-core")
        .unwrap_or_else(|| "unknown".to_string());

    println!("cargo:rustc-env=FLOWSURFACE_ODB_VERSION={odb_version}");
    println!("cargo:rerun-if-changed=Cargo.lock");
}

fn extract_package_version(lock_content: &str, package_name: &str) -> Option<String> {
    let target = format!("name = \"{package_name}\"");
    let mut lines = lock_content.lines();
    while let Some(line) = lines.next() {
        if line.trim() == target {
            // Next line should be: version = "x.y.z"
            if let Some(version_line) = lines.next() {
                let version_line = version_line.trim();
                if let Some(version) = version_line
                    .strip_prefix("version = \"")
                    .and_then(|s| s.strip_suffix('"'))
                {
                    return Some(version.to_string());
                }
            }
        }
    }
    None
}
