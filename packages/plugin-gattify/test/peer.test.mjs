import assert from "node:assert/strict";
import test from "node:test";

import { EventHub } from "../dist/bridge.js";
import { createBle } from "../dist/index.js";
import { createEndpoint } from "../dist/peer.js";

const SERVICE = "80ff87c3-8e84-4914-aedc-0d6a3ba5534d";

/** A bridge whose events come from a test instead of a webview channel. */
function eventBridge(replies = {}) {
  const hub = new EventHub();
  const calls = [];
  return {
    calls,
    emit(event, payload) {
      hub.dispatch({ event, payload });
    },
    async invoke(command, args) {
      calls.push([command, args]);
      if (command in replies) return replies[command](args);
      if (command === "plugin:gattify|create_endpoint") return { endpointId: "endpoint-1" };
      if (command === "plugin:gattify|dial_peer") return { peerId: "peer-9" };
      return null;
    },
    listen(event, handler) {
      return Promise.resolve(hub.on(event, handler));
    },
  };
}

const bytes = (text) => new TextEncoder().encode(text);
const text = (value) => new TextDecoder().decode(value);
const base64 = (value) => Buffer.from(value).toString("base64");

test("the event hub hands messages to the listeners of their event", () => {
  const hub = new EventHub();
  const received = [];
  const unlisten = hub.on("gattify://scan-result", (event) => received.push(event.payload));
  hub.dispatch({ event: "gattify://scan-result", payload: 1 });
  hub.dispatch({ event: "gattify://scan-stopped", payload: 2 });
  unlisten();
  hub.dispatch({ event: "gattify://scan-result", payload: 3 });
  assert.deepEqual(received, [1]);
});

test("an endpoint listens only when asked", async () => {
  const bridge = eventBridge();
  await createEndpoint({ serviceUuid: SERVICE, bridge });
  await createEndpoint({ serviceUuid: SERVICE, localName: "Host", listen: true, bridge });
  const options = bridge.calls.map(([, args]) => args.options);
  assert.equal(options[0].listen, false);
  assert.equal(options[1].listen, true);
  assert.equal(options[1].localName, "Host");
});

test("a host meets a peer, reads its messages and sees it close", async () => {
  const bridge = eventBridge();
  const endpoint = await createEndpoint({ serviceUuid: SERVICE, listen: true, bridge });
  const peers = [];
  endpoint.onPeer((peer) => peers.push(peer));

  bridge.emit("gattify://peer-ready", { endpointId: "endpoint-1", peerId: "peer-1", dialed: false });
  bridge.emit("gattify://peer-ready", { endpointId: "endpoint-2", peerId: "peer-2", dialed: false });
  assert.equal(peers.length, 1);
  const [peer] = peers;

  bridge.emit("gattify://peer-message", { peerId: "peer-1", valueBase64: base64(bytes("early")) });
  const messages = [];
  peer.onMessage((value) => messages.push(text(value)));
  bridge.emit("gattify://peer-message", { peerId: "peer-1", valueBase64: base64(bytes("late")) });
  assert.deepEqual(messages, ["early", "late"]);

  const reasons = [];
  peer.onClose((reason) => reasons.push(reason));
  bridge.emit("gattify://peer-closed", { peerId: "peer-1", reason: "remote" });
  bridge.emit("gattify://peer-closed", { peerId: "peer-1", reason: "lost" });
  assert.deepEqual(reasons, ["remote"]);
});

test("a dialed peer is one handle whichever of ready and dial comes first", async () => {
  const bridge = eventBridge();
  const endpoint = await createEndpoint({ serviceUuid: SERVICE, bridge });
  const announced = [];
  endpoint.onPeer((peer) => announced.push(peer));

  bridge.emit("gattify://peer-ready", { endpointId: "endpoint-1", peerId: "peer-9", dialed: true });
  bridge.emit("gattify://peer-message", { peerId: "peer-9", valueBase64: base64(bytes("hi")) });
  const peer = await endpoint.dial("device-1");

  const messages = [];
  peer.onMessage((value) => messages.push(text(value)));
  assert.equal(peer.id, "peer-9");
  assert.deepEqual(messages, ["hi"]);
  assert.equal(announced.length, 0);
  assert.deepEqual(bridge.calls.at(-1), [
    "plugin:gattify|dial_peer",
    { endpointId: "endpoint-1", deviceId: "device-1" },
  ]);
});

test("closing a peer reports local once", async () => {
  const bridge = eventBridge();
  const endpoint = await createEndpoint({ serviceUuid: SERVICE, bridge });
  const peer = await endpoint.dial("device-1");
  const reasons = [];
  peer.onClose((reason) => reasons.push(reason));

  await peer.close();
  await peer.close();
  bridge.emit("gattify://peer-closed", { peerId: "peer-9", reason: "local" });

  assert.deepEqual(reasons, ["local"]);
  assert.equal(
    bridge.calls.filter(([command]) => command === "plugin:gattify|close_peer").length,
    1,
  );
  const late = await new Promise((resolve) => peer.onClose(resolve));
  assert.equal(late, "local");
});

test("closing an endpoint closes its peers", async () => {
  const bridge = eventBridge();
  const endpoint = await createEndpoint({ serviceUuid: SERVICE, listen: true, bridge });
  const peers = [];
  endpoint.onPeer((peer) => peers.push(peer));
  bridge.emit("gattify://peer-ready", { endpointId: "endpoint-1", peerId: "peer-1", dialed: false });
  const reasons = [];
  peers[0].onClose((reason) => reasons.push(reason));

  await endpoint.close();
  bridge.emit("gattify://peer-ready", { endpointId: "endpoint-1", peerId: "peer-3", dialed: false });

  assert.deepEqual(reasons, ["local"]);
  assert.equal(peers.length, 1);
});

test("a send carries the bytes and the timeout", async () => {
  const bridge = eventBridge({
    "plugin:gattify|send_peer": () => ({ messageId: 4, delivery: "transportAcknowledged" }),
  });
  const endpoint = await createEndpoint({ serviceUuid: SERVICE, bridge });
  const peer = await endpoint.dial("device-1");

  const receipt = await peer.send(bytes("ping"), { timeoutMs: 5_000 });

  assert.deepEqual(receipt, { messageId: 4, delivery: "transportAcknowledged" });
  assert.deepEqual(bridge.calls.at(-1), [
    "plugin:gattify|send_peer",
    { peerId: "peer-9", valueBase64: base64(bytes("ping")), timeoutMs: 5_000 },
  ]);
});

test("a scan reads the device out of its event", async () => {
  const bridge = eventBridge({
    "plugin:gattify|execute_scan": (args) =>
      args.request.command.kind === "startScan"
        ? { kind: "scanStarted", payload: { scanId: "scan-1" } }
        : { kind: "empty" },
  });
  const session = await createBle({ bridge });
  const scan = await session.scan({ serviceUuids: [SERVICE] });
  const seen = [];
  scan.onUpdate((device) => seen.push(device.id));

  bridge.emit("gattify://scan-result", { device: { id: "device-1", scanId: "scan-1" } });
  bridge.emit("gattify://scan-result", { device: { id: "device-2", scanId: "scan-other" } });

  assert.deepEqual(seen, ["device-1"]);
  assert.deepEqual(
    scan.snapshot().map((device) => device.id),
    ["device-1"],
  );
});
