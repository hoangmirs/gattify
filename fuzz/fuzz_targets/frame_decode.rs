#![no_main]

use tauri_plugin_gattify::peer::Frame;
use libfuzzer_sys::fuzz_target;

fuzz_target!(|bytes: &[u8]| {
    let _ = Frame::decode(bytes, 16 * 1024);
});

