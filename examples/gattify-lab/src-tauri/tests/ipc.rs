use serde_json::json;
use tauri::{
    ipc::{CallbackFn, InvokeBody},
    test::{get_ipc_response, mock_builder, MockRuntime, INVOKE_KEY},
    webview::InvokeRequest,
    App, WebviewWindowBuilder,
};

fn request(cmd: &str) -> InvokeRequest {
    InvokeRequest {
        cmd: cmd.into(),
        callback: CallbackFn(0),
        error: CallbackFn(1),
        url: "tauri://localhost".parse().unwrap(),
        body: InvokeBody::default(),
        headers: Default::default(),
        invoke_key: INVOKE_KEY.to_string(),
    }
}

fn build_app() -> App<MockRuntime> {
    gattify_lab_lib::build(mock_builder())
        .build(tauri::generate_context!())
        .expect("failed to build the lab app")
}

#[test]
fn get_state_answers_unknown() {
    let app = build_app();
    let webview = WebviewWindowBuilder::new(&app, "main", Default::default())
        .build()
        .expect("failed to build the main webview");

    let state = get_ipc_response(&webview, request("plugin:gattify|get_state"))
        .expect("get_state should pass the ACL")
        .deserialize::<serde_json::Value>()
        .unwrap();
    assert_eq!(state, json!({"kind": "state", "payload": "unknown"}));
}

#[test]
fn get_capabilities_answers_unknown_central_support() {
    let app = build_app();
    let webview = WebviewWindowBuilder::new(&app, "main", Default::default())
        .build()
        .expect("failed to build the main webview");

    let capabilities = get_ipc_response(&webview, request("plugin:gattify|get_capabilities"))
        .expect("get_capabilities should pass the ACL")
        .deserialize::<serde_json::Value>()
        .unwrap();
    assert_eq!(capabilities["kind"], "capabilities");
    assert_eq!(capabilities["payload"]["central"]["level"], "unknown");
}

#[test]
fn close_succeeds() {
    let app = build_app();
    let webview = WebviewWindowBuilder::new(&app, "main", Default::default())
        .build()
        .expect("failed to build the main webview");

    let close = get_ipc_response(&webview, request("plugin:gattify|close"))
        .expect("close should pass the ACL")
        .deserialize::<serde_json::Value>()
        .unwrap();
    assert_eq!(close, json!({"kind": "empty"}));
}

#[test]
fn execute_scan_is_rejected_by_the_acl() {
    let app = build_app();
    let webview = WebviewWindowBuilder::new(&app, "main", Default::default())
        .build()
        .expect("failed to build the main webview");

    let scan_error = get_ipc_response(&webview, request("plugin:gattify|execute_scan"))
        .expect_err("execute_scan is not in gattify:default and must be rejected by the ACL");
    let message = scan_error
        .as_str()
        .expect("an ACL rejection is a plain string");
    assert!(
        message.contains("not allowed"),
        "expected an ACL rejection mentioning \"not allowed\", got: {message}"
    );
}
