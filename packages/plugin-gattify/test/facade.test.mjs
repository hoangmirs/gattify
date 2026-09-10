import assert from "node:assert/strict";
import test from "node:test";

import { createBle } from "../dist/index.js";

class FakeBridge {
  calls = [];

  async invoke(command, args) {
    this.calls.push([command, args]);
    const kind = args?.request?.command?.kind;
    if (command === "plugin:gattify|get_state") return { kind: "state", payload: "poweredOn" };
    if (command === "plugin:gattify|get_capabilities") {
      const supported = { level: "supported", reason: "available", description: null };
      return {
        kind: "capabilities",
        payload: {
          central: supported,
          peripheral: supported,
          advertising: supported,
          targetedNotify: supported,
          simultaneousRoles: supported,
          background: { level: "unsupported", reason: "foregroundOnly", description: null },
          maxConnections: 4,
          maxAdvertisingDataLength: 31,
        },
      };
    }
    if (command.startsWith("plugin:gattify|request_") && command.endsWith("_permission")) {
      return {
        kind: "permissions",
        payload: { scan: "granted", connect: "granted", advertise: "granted" },
      };
    }
    if (kind === "startScan") return { kind: "scanStarted", payload: { scanId: "scan-1" } };
    if (kind === "stopScan" || command === "plugin:gattify|close") return { kind: "empty" };
    throw new Error("unexpected command " + command + " / " + kind);
  }
}

test("session sends typed commands and close is idempotent", async () => {
  const bridge = new FakeBridge();
  const session = await createBle({ bridge });

  assert.equal(await session.getState(), "poweredOn");
  const scan = await session.scan({ serviceUuids: [] });
  assert.equal(scan.id, "scan-1");
  await scan.stop();
  await scan.stop();
  await session.close();
  await session.close();

  assert.equal(
    bridge.calls.filter(([command]) => command === "plugin:gattify|close").length,
    1,
  );
  assert.equal(
    bridge.calls.filter(([, args]) => args?.request?.command?.kind === "stopScan").length,
    1,
  );
  const startCall = bridge.calls.find(
    ([, args]) => args?.request?.command?.kind === "startScan",
  );
  assert.equal(startCall[0], "plugin:gattify|execute_scan");
  assert.equal(typeof startCall[1].request.operationId, "string");
});

test("capabilities preserve unsupported and unknown instead of booleans", async () => {
  const session = await createBle({ bridge: new FakeBridge() });
  const capabilities = await session.getCapabilities();
  assert.equal(capabilities.central.level, "supported");
  assert.equal(capabilities.background.level, "unsupported");
});

test("permission requests use role-specific commands", async () => {
  const bridge = new FakeBridge();
  const session = await createBle({ bridge });
  await session.requestPermissions({ scan: true, connect: false, advertise: true });
  assert.deepEqual(
    bridge.calls.map(([command]) => command),
    ["plugin:gattify|request_scan_permission", "plugin:gattify|request_advertise_permission"],
  );
});

test("aborting a live operation invokes native cancellation", async () => {
  const calls = [];
  const bridge = {
    invoke(command, args) {
      calls.push([command, args]);
      if (command === "plugin:gattify|cancel") return Promise.resolve({ kind: "empty" });
      return new Promise(() => {});
    },
  };
  const session = await createBle({ bridge });
  const controller = new AbortController();
  const pending = session.scan({ signal: controller.signal });
  controller.abort();
  await assert.rejects(pending, { name: "AbortError" });

  const execute = calls.find(([command]) => command === "plugin:gattify|execute_scan");
  const cancel = calls.find(([command]) => command === "plugin:gattify|cancel");
  assert.equal(cancel[1].request.operationId, execute[1].request.operationId);
});

test("closing a scan before listen resolves still detaches the native listener", async () => {
  let unlistened = false;
  let resolveListen;
  const bridge = {
    async invoke(command, args) {
      const kind = args?.request?.command?.kind;
      if (kind === "startScan") return { kind: "scanStarted", payload: { scanId: "scan-1" } };
      if (kind === "stopScan") return { kind: "empty" };
      throw new Error("unexpected command " + command);
    },
    listen() {
      return new Promise((resolve) => {
        resolveListen = resolve;
      });
    },
  };
  const session = await createBle({ bridge });
  const scan = await session.scan({ serviceUuids: [] });
  await scan.stop();

  resolveListen(() => {
    unlistened = true;
  });
  await Promise.resolve();
  assert.equal(unlistened, true);
});

test("scan listens on the gattify event namespace", async () => {
  const bridge = new FakeBridge();
  const events = [];
  bridge.listen = async (event) => {
    events.push(event);
    return () => {};
  };
  const session = await createBle({ bridge });

  const scan = await session.scan({ serviceUuids: [] });
  await scan.stop();

  assert.deepEqual(events, ["gattify://scan-result"]);
});
