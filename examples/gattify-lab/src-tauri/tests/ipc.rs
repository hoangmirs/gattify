use serde_json::json;
use tauri::{
    ipc::{CallbackFn, CapabilityBuilder, InvokeBody},
    test::{get_ipc_response, mock_builder, MockRuntime, INVOKE_KEY},
    webview::InvokeRequest,
    App, Manager, WebviewWindowBuilder,
};

/// The origin Tauri serves the app from, which the ACL treats as local. WebView2
/// cannot load a custom scheme, so Windows uses `http://tauri.localhost`.
const APP_URL: &str = if cfg!(windows) {
    "http://tauri.localhost"
} else {
    "tauri://localhost"
};

fn request(cmd: &str) -> InvokeRequest {
    InvokeRequest {
        cmd: cmd.into(),
        callback: CallbackFn(0),
        error: CallbackFn(1),
        url: APP_URL.parse().unwrap(),
        body: InvokeBody::default(),
        headers: Default::default(),
        invoke_key: INVOKE_KEY.to_string(),
    }
}

fn request_with_body(cmd: &str, body: serde_json::Value) -> InvokeRequest {
    InvokeRequest {
        body: InvokeBody::Json(body),
        ..request(cmd)
    }
}

fn build_app() -> App<MockRuntime> {
    gattify_lab_lib::build(mock_builder())
        .build(tauri::generate_context!())
        .expect("failed to build the lab app")
}

/// macOS and Windows run a real radio backend, so a test must not start radio work there: it
/// would show the Bluetooth prompt or depend on the adapter of the machine. Linux keeps the
/// backend that answers Unsupported.
const RADIO_BACKEND: bool = cfg!(any(target_os = "macos", target_os = "windows"));

#[test]
fn get_state_answers_a_state() {
    let app = build_app();
    let webview = WebviewWindowBuilder::new(&app, "main", Default::default())
        .build()
        .expect("failed to build the main webview");

    let state = get_ipc_response(&webview, request("plugin:gattify|get_state"))
        .expect("get_state should pass the ACL")
        .deserialize::<serde_json::Value>()
        .unwrap();
    assert_eq!(state["kind"], "state");
    if !RADIO_BACKEND {
        assert_eq!(state["payload"], "unknown");
    }
}

#[test]
fn get_capabilities_answers_every_field() {
    let app = build_app();
    let webview = WebviewWindowBuilder::new(&app, "main", Default::default())
        .build()
        .expect("failed to build the main webview");

    let capabilities = get_ipc_response(&webview, request("plugin:gattify|get_capabilities"))
        .expect("get_capabilities should pass the ACL")
        .deserialize::<serde_json::Value>()
        .unwrap();
    assert_eq!(capabilities["kind"], "capabilities");
    assert_eq!(
        capabilities["payload"]["background"]["reason"],
        "foregroundOnlyContract"
    );
    if !RADIO_BACKEND {
        assert_eq!(capabilities["payload"]["central"]["level"], "unknown");
    }
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

const LAB_SERVICE: &str = "80ff87c3-8e84-4914-aedc-0d6a3ba5534d";

fn main_webview(app: &App<MockRuntime>) -> tauri::WebviewWindow<MockRuntime> {
    WebviewWindowBuilder::new(app, "main", Default::default())
        .build()
        .expect("failed to build the main webview")
}

fn scan_body(operation_id: &str, service_uuids: serde_json::Value) -> serde_json::Value {
    json!({
        "request": {
            "operationId": operation_id,
            "command": {
                "kind": "startScan",
                "payload": { "serviceUuids": service_uuids, "timeoutMs": null }
            },
            "deadlineMillis": null
        }
    })
}

#[test]
fn a_role_outside_the_capability_is_rejected_by_the_acl() {
    let app = build_app();
    let webview = main_webview(&app);

    let server_error = get_ipc_response(&webview, request("plugin:gattify|execute_server"))
        .expect_err("the lab does not grant gattify:server, so the ACL must reject it");
    let message = server_error
        .as_str()
        .expect("an ACL rejection is a plain string");
    assert!(
        message.contains("not allowed"),
        "expected an ACL rejection mentioning \"not allowed\", got: {message}"
    );
    assert!(
        message.contains("gattify:allow-execute-server"),
        "expected the ACL rejection to list execute_server's own permission, got: {message}"
    );
}

#[test]
fn execute_scan_checks_its_own_role() {
    let app = build_app();
    let webview = main_webview(&app);

    let connect_body = json!({
        "request": {
            "operationId": "op-1",
            "command": {
                "kind": "connect",
                "payload": { "deviceId": "device-1", "options": { "timeoutMs": null } }
            },
            "deadlineMillis": null
        }
    });
    let role_error = get_ipc_response(
        &webview,
        request_with_body("plugin:gattify|execute_scan", connect_body),
    )
    .expect_err("execute_scan must refuse a connect command");
    assert_eq!(
        role_error["message"],
        "command is not permitted through this role-specific endpoint"
    );
}

#[test]
fn a_scan_must_stay_inside_the_scope() {
    let app = build_app();
    let webview = main_webview(&app);

    let unfiltered = get_ipc_response(
        &webview,
        request_with_body("plugin:gattify|execute_scan", scan_body("op-1", json!([]))),
    )
    .expect_err("a scan without a filter leaves the scope");
    assert_eq!(unfiltered["code"], "permissionDenied");

    let foreign = get_ipc_response(
        &webview,
        request_with_body(
            "plugin:gattify|execute_scan",
            scan_body("op-2", json!(["0000180d-0000-1000-8000-00805f9b34fb"])),
        ),
    )
    .expect_err("a scan for a service outside the scope is refused");
    assert_eq!(foreign["code"], "permissionDenied");

    if RADIO_BACKEND {
        return;
    }
    let scoped = get_ipc_response(
        &webview,
        request_with_body(
            "plugin:gattify|execute_scan",
            scan_body("op-3", json!([LAB_SERVICE])),
        ),
    )
    .expect_err("a scoped scan passes the scope and reaches the Linux backend");
    assert_eq!(scoped["code"], "unsupported");
}

#[test]
fn the_scope_is_app_wide_and_bounds_every_window() {
    let app = build_app();
    app.add_capability(
        CapabilityBuilder::new("second-window")
            .window("second")
            .permission("gattify:scan")
            .permission("gattify:peer"),
    )
    .expect("failed to grant the second window");
    let webview = WebviewWindowBuilder::new(&app, "second", Default::default())
        .build()
        .expect("failed to build the second webview");

    let endpoint = get_ipc_response(
        &webview,
        request_with_body(
            "plugin:gattify|create_endpoint",
            json!({ "options": { "serviceUuid": LAB_SERVICE, "listen": false } }),
        ),
    );
    // The lab capability grants the scope to the whole app, so this window
    // sees it too. Tauri resolves a plugin's global scope per app, not per window.
    let endpoint = endpoint.expect("the app-wide scope admits the lab service");
    assert!(endpoint.deserialize::<serde_json::Value>().unwrap()["endpointId"].is_string());

    let foreign = get_ipc_response(
        &webview,
        request_with_body(
            "plugin:gattify|create_endpoint",
            json!({ "options": { "serviceUuid": "0000180d-0000-1000-8000-00805f9b34fb" } }),
        ),
    )
    .expect_err("an endpoint for a service outside the scope is refused");
    assert_eq!(foreign["code"], "permissionDenied");
}

#[test]
fn a_joiner_endpoint_needs_no_radio_until_it_dials() {
    let app = build_app();
    let webview = main_webview(&app);

    let endpoint = get_ipc_response(
        &webview,
        request_with_body(
            "plugin:gattify|create_endpoint",
            json!({ "options": { "serviceUuid": LAB_SERVICE, "listen": false } }),
        ),
    )
    .expect("a joiner endpoint only records its options")
    .deserialize::<serde_json::Value>()
    .unwrap();
    let endpoint_id = endpoint["endpointId"].as_str().unwrap().to_owned();

    if RADIO_BACKEND {
        return;
    }
    let dial = get_ipc_response(
        &webview,
        request_with_body(
            "plugin:gattify|dial_peer",
            json!({ "endpointId": endpoint_id, "deviceId": "device-1" }),
        ),
    )
    .expect_err("the Linux backend cannot connect");
    assert_eq!(dial["code"], "unsupported");

    let host = get_ipc_response(
        &webview,
        request_with_body(
            "plugin:gattify|create_endpoint",
            json!({ "options": { "serviceUuid": LAB_SERVICE, "listen": true } }),
        ),
    )
    .expect_err("the Linux backend cannot host a server");
    assert_eq!(host["code"], "unsupported");
}

#[test]
fn every_webview_may_listen_for_its_events() {
    let app = build_app();
    let webview = main_webview(&app);

    get_ipc_response(
        &webview,
        request_with_body(
            "plugin:gattify|listen_events",
            json!({ "channel": "__CHANNEL__:7" }),
        ),
    )
    .expect("gattify:default allows listen_events");
}
