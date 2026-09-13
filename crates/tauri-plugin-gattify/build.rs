// Never list a native command such as execute here: its permission would let a webview reach native code past the Rust owner and role checks.
#[cfg(feature = "tauri")]
const COMMANDS: &[&str] = &[
    "execute_scan",
    "execute_connect",
    "execute_server",
    "execute_advertise",
    "request_scan_permission",
    "request_connect_permission",
    "request_advertise_permission",
    "cancel",
    "get_state",
    "get_capabilities",
    "check_permissions",
    "close",
    "listen_events",
    "create_endpoint",
    "dial_peer",
    "send_peer",
    "close_peer",
    "close_endpoint",
];

fn main() {
    #[cfg(feature = "tauri")]
    {
        tauri_plugin::Builder::new(COMMANDS)
            .android_path("android")
            .ios_path("ios")
            .build();
        if std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("macos") {
            macos::link_engine();
        }
    }
}

/// Builds the Swift engine of the iOS plugin for macOS, without its Tauri iOS glue, and links it
/// with the Swift runtime. macOS has no Tauri Swift plugin API, so `macos/Bridge.swift`
/// exposes the engine through a C ABI instead.
#[cfg(feature = "tauri")]
mod macos {
    use std::{
        env,
        path::{Path, PathBuf},
        process::Command,
    };

    /// The Tauri iOS entry point. Everything else in `ios/Sources` is shared.
    const IOS_ONLY: &str = "GattifyPlugin.swift";
    const LIBRARY: &str = "gattify_apple";

    pub(crate) fn link_engine() {
        let manifest = PathBuf::from(var("CARGO_MANIFEST_DIR"));
        let out = PathBuf::from(var("OUT_DIR"));
        let mut sources = swift_files(&manifest.join("ios").join("Sources"));
        sources.retain(|path| !path.ends_with(IOS_ONLY));
        sources.push(manifest.join("macos").join("Bridge.swift"));
        println!("cargo:rerun-if-changed=ios/Sources");
        println!("cargo:rerun-if-changed=macos");
        println!("cargo:rerun-if-env-changed=MACOSX_DEPLOYMENT_TARGET");

        let arch = match var("CARGO_CFG_TARGET_ARCH").as_str() {
            "aarch64" => "arm64",
            "x86_64" => "x86_64",
            other => panic!("gattify: no Swift target for the macOS architecture {other}"),
        };
        let minimum = env::var("MACOSX_DEPLOYMENT_TARGET").unwrap_or_else(|_| "10.15".into());
        let target = format!("{arch}-apple-macosx{minimum}");
        let optimization = if var("PROFILE") == "release" {
            "-O"
        } else {
            "-Onone"
        };

        let library = out.join(format!("lib{LIBRARY}.a"));
        let mut swiftc = xcrun();
        swiftc
            .args(["swiftc", "-emit-library", "-static", "-parse-as-library"])
            .args(["-swift-version", "5", "-module-name", "GattifyApple"])
            .args(["-target", &target, optimization])
            .arg("-module-cache-path")
            .arg(out.join("swift-module-cache"))
            .arg("-o")
            .arg(&library)
            .args(&sources);
        run(&mut swiftc);

        println!("cargo:rustc-link-search=native={}", out.display());
        println!("cargo:rustc-link-lib=static={LIBRARY}");
        for path in runtime_library_paths(&target) {
            println!("cargo:rustc-link-search=native={path}");
        }
        // `#available` checks call into compiler-rt, which rustc does not link by default.
        let resources = output(xcrun().args(["clang", "--print-resource-dir"]));
        println!(
            "cargo:rustc-link-search=native={}/lib/darwin",
            resources.trim()
        );
        println!("cargo:rustc-link-lib=static=clang_rt.osx");
        println!("cargo:rustc-link-lib=framework=CoreBluetooth");
        println!("cargo:rustc-link-lib=framework=Foundation");
    }

    /// Where the linker finds the Swift runtime and its compatibility libraries.
    fn runtime_library_paths(target: &str) -> Vec<String> {
        let info = output(xcrun().args(["swiftc", "-print-target-info", "-target", target]));
        let info: serde_json::Value =
            serde_json::from_str(&info).expect("gattify: swiftc printed invalid target info");
        info["paths"]["runtimeLibraryPaths"]
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(|path| path.as_str().map(str::to_owned))
            .collect()
    }

    fn swift_files(directory: &Path) -> Vec<PathBuf> {
        let mut files: Vec<PathBuf> = std::fs::read_dir(directory)
            .unwrap_or_else(|error| panic!("gattify: cannot read {}: {error}", directory.display()))
            .filter_map(|entry| entry.ok().map(|entry| entry.path()))
            .filter(|path| {
                path.extension()
                    .is_some_and(|extension| extension == "swift")
            })
            .collect();
        files.sort();
        files
    }

    /// SDKROOT may name the SDK of another platform, so always ask for the macOS one.
    fn xcrun() -> Command {
        let mut command = Command::new("xcrun");
        command.env_remove("SDKROOT").args(["--sdk", "macosx"]);
        command
    }

    fn run(command: &mut Command) {
        let status = command
            .status()
            .unwrap_or_else(|error| panic!("gattify: cannot run {command:?}: {error}"));
        assert!(
            status.success(),
            "gattify: {command:?} failed with {status}"
        );
    }

    fn output(command: &mut Command) -> String {
        let output = command
            .output()
            .unwrap_or_else(|error| panic!("gattify: cannot run {command:?}: {error}"));
        assert!(
            output.status.success(),
            "gattify: {command:?} failed: {}",
            String::from_utf8_lossy(&output.stderr)
        );
        String::from_utf8(output.stdout).expect("gattify: xcrun printed invalid UTF-8")
    }

    fn var(key: &str) -> String {
        env::var(key).unwrap_or_else(|_| panic!("gattify: cargo did not set {key}"))
    }
}
