fn main() {
    // `tauri.conf.json` bundles the built browser extension (`bun run ext:build`, which
    // `beforeBuildCommand` runs for a package), and tauri-build copies the resources on every build.
    // An empty folder is skipped, so a build without the extension (tests, `cargo run`) compiles.
    let extension = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../extension/dist");
    if !extension.exists() {
        if let Err(e) = std::fs::create_dir_all(&extension) {
            println!("cargo:warning=could not create {}: {e}", extension.display());
        }
    }
    tauri_build::build()
}
