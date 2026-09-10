pub fn build<R: tauri::Runtime>(builder: tauri::Builder<R>) -> tauri::Builder<R> {
    builder.plugin(tauri_plugin_gattify::init())
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    build(tauri::Builder::default())
        .run(tauri::generate_context!())
        .expect("error while running the gattify lab");
}
