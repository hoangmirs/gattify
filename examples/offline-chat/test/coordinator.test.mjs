import assert from "node:assert/strict";
import test from "node:test";

import { Coordinator } from "../dist/coordinator.js";

function message(messageId, text = "hello") {
  return {
    version: 1,
    kind: "chat",
    messageId,
    sessionId: "session-123",
    body: { text },
  };
}

test("deduplicates within the admitted connection and preserves sequence", () => {
  const coordinator = new Coordinator("session-123", 5);
  coordinator.admit("connection-a");
  coordinator.admit("connection-b");

  const first = coordinator.acceptChat("connection-a", message("message-001"));
  const duplicate = coordinator.acceptChat("connection-a", message("message-001"));

  assert.equal(first, duplicate);
  assert.equal(first.sequence, 1);
  assert.deepEqual(first.receipts, { "connection-b": "pending" });
});

test("reports a history gap after bounded eviction", () => {
  const coordinator = new Coordinator("session-123", 1);
  coordinator.admit("connection-a");
  coordinator.acceptChat("connection-a", message("message-001", "one"));
  coordinator.acceptChat("connection-a", message("message-002", "two"));

  const result = coordinator.historyAfter(0);
  assert.equal(result.gap, true);
  assert.deepEqual(result.entries.map((entry) => entry.sequence), [2]);
});

