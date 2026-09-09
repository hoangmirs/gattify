import assert from "node:assert/strict";
import test from "node:test";

import { createBle } from "../dist/index.js";

class FakeBridge {
  calls = [];

  async invoke(command, args) {
    this.calls.push([command, args]);
    const kind = args?.request?.command?.kind;
    if (command === "plugin:ble|get_state") return { kind: "state", payload: "poweredOn" };
    if (command === "plugin:ble|get_capabilities") {
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
    if (kind === "startScan") return { kind: "scanStarted", payload: { scanId: "scan-1" } };
    if (kind === "stopScan" || command === "plugin:ble|close") return { kind: "empty" };
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
    bridge.calls.filter(([command]) => command === "plugin:ble|close").length,
    1,
  );
  assert.equal(
    bridge.calls.filter(([, args]) => args?.request?.command?.kind === "stopScan").length,
    1,
  );
});

test("capabilities preserve unsupported and unknown instead of booleans", async () => {
  const session = await createBle({ bridge: new FakeBridge() });
  const capabilities = await session.getCapabilities();
  assert.equal(capabilities.central.level, "supported");
  assert.equal(capabilities.background.level, "unsupported");
});
