//! Routes backend events to the peer driver or to the owner webview.
//!
//! A webview receives its events through the Tauri channels it registered with
//! `listen_events`. A channel delivers only to the webview that created it, so
//! no other webview of the app can observe them.

use std::{collections::HashMap, sync::Arc};

use parking_lot::Mutex;
use serde_json::{json, Value};
use tauri::ipc::Channel;
use tokio::sync::mpsc;

use crate::{peer::peer_label, Event, OwnerId, ScopeGuard};

const WEBVIEW_OWNER_PREFIX: &str = "webview:";

/// The owner of the resources a webview creates.
pub(crate) fn webview_owner(label: &str) -> OwnerId {
    OwnerId::new(format!("{WEBVIEW_OWNER_PREFIX}{label}"))
}

#[derive(Debug, Eq, PartialEq)]
pub(crate) enum Route<'a> {
    Peer(&'a str),
    Webview(&'a str),
    Drop,
}

pub(crate) fn route(owner: &OwnerId) -> Route<'_> {
    if let Some(label) = peer_label(owner) {
        Route::Peer(label)
    } else if let Some(label) = owner.as_str().strip_prefix(WEBVIEW_OWNER_PREFIX) {
        Route::Webview(label)
    } else {
        Route::Drop
    }
}

/// `gattify://scan-result` for the event kind `scanResult`.
pub(crate) fn event_name(kind: &str) -> String {
    let mut name = String::from("gattify://");
    for character in kind.chars() {
        if character.is_ascii_uppercase() {
            name.push('-');
            name.push(character.to_ascii_lowercase());
        } else {
            name.push(character);
        }
    }
    name
}

/// The event name and payload that a webview receives for a backend event.
pub(crate) fn event_message(event: &Event) -> (String, Value) {
    let mut value = serde_json::to_value(event).unwrap_or(Value::Null);
    let kind = value
        .get("kind")
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_owned();
    let payload = value.get_mut("payload").map_or(Value::Null, Value::take);
    (event_name(&kind), payload)
}

/// The event channels of each webview.
#[derive(Default)]
pub(crate) struct EventHub {
    channels: Mutex<HashMap<String, Vec<Channel<Value>>>>,
}

impl EventHub {
    pub(crate) fn add(&self, label: &str, channel: Channel<Value>) {
        self.channels
            .lock()
            .entry(label.to_owned())
            .or_default()
            .push(channel);
    }

    pub(crate) fn remove_label(&self, label: &str) {
        self.channels.lock().remove(label);
    }

    pub(crate) fn emit(&self, label: &str, name: &str, payload: &Value) {
        let channels = self.channels.lock().get(label).cloned().unwrap_or_default();
        let message = json!({ "event": name, "payload": payload });
        for channel in channels {
            let _ = channel.send(message.clone());
        }
    }
}

/// Sends each backend event where it belongs.
pub(crate) struct Router {
    pub(crate) hub: Arc<EventHub>,
    pub(crate) guard: Arc<ScopeGuard>,
    pub(crate) driver_inbox: mpsc::UnboundedSender<(OwnerId, Event)>,
}

impl Router {
    pub(crate) fn dispatch(&self, owner: OwnerId, event: Event) {
        match route(&owner) {
            Route::Peer(_) => {
                let _ = self.driver_inbox.send((owner, event));
            }
            Route::Webview(label) => {
                if let Event::ConnectionClosed { connection_id } = &event {
                    self.guard.forget_connection(&owner, connection_id);
                }
                let (name, payload) = event_message(&event);
                self.hub.emit(label, &name, &payload);
            }
            Route::Drop => {}
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{ConnectionId, ScanId};

    #[test]
    fn owners_route_to_the_driver_or_their_webview() {
        assert_eq!(
            route(&OwnerId::new("gattify-peer:main")),
            Route::Peer("main")
        );
        assert_eq!(route(&OwnerId::new("webview:main")), Route::Webview("main"));
        assert_eq!(route(&OwnerId::new("somebody")), Route::Drop);
    }

    #[test]
    fn event_kinds_become_kebab_case_names() {
        assert_eq!(event_name("scanResult"), "gattify://scan-result");
        assert_eq!(
            event_name("characteristicValue"),
            "gattify://characteristic-value"
        );
        assert_eq!(event_name("peer-ready"), "gattify://peer-ready");
    }

    #[test]
    fn a_webview_receives_the_event_payload() {
        let (name, payload) = event_message(&Event::ScanStopped {
            scan_id: ScanId::new("scan-1"),
        });
        assert_eq!(name, "gattify://scan-stopped");
        assert_eq!(payload, json!({ "scanId": "scan-1" }));

        let (name, payload) = event_message(&Event::ConnectionClosed {
            connection_id: ConnectionId::new("connection-1"),
        });
        assert_eq!(name, "gattify://connection-closed");
        assert_eq!(payload, json!({ "connectionId": "connection-1" }));
    }
}
