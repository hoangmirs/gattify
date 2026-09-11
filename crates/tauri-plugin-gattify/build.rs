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
    "create_endpoint",
    "dial_peer",
    "send_peer",
    "close_peer",
    "close_endpoint",
];

fn main() {
    #[cfg(feature = "tauri")]
    tauri_plugin::Builder::new(COMMANDS)
        .android_path("android")
        .ios_path("ios")
        .build();
}
