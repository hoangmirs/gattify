import assert from "node:assert/strict";
import test from "node:test";

import { Coordinator, Outbox, validateEnvelope } from "../dist/coordinator.js";

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

  const admittedAgain = coordinator.acceptChat(
    "connection-a",
    message("message-001", "admitted after eviction"),
  );
  assert.equal(admittedAgain.sequence, 3);
});

test("rejects malformed and unknown envelopes at the runtime boundary", () => {
  assert.throws(
    () =>
      validateEnvelope({
        version: 1,
        kind: "bogus",
        messageId: "message-001",
        sessionId: "session-123",
        body: {},
      }),
    /unknown envelope kind/,
  );
  assert.throws(
    () =>
      validateEnvelope({
        version: 1,
        kind: "chat",
        messageId: "message-001",
        sessionId: "session-123",
        body: { text: 42 },
      }),
    /chat text must be a string/,
  );
});

test("records receipts by coordinator sequence when sender IDs overlap", () => {
  const coordinator = new Coordinator("session-123", 5);
  for (const id of ["connection-a", "connection-b", "connection-c"]) coordinator.admit(id);
  const first = coordinator.acceptChat("connection-a", message("message-001", "one"));
  const second = coordinator.acceptChat("connection-b", message("message-001", "two"));

  coordinator.recordReceipt(second.sequence, "connection-c", "read");
  assert.equal(first.receipts["connection-c"], "pending");
  assert.equal(second.receipts["connection-c"], "read");
});

test("coordinator departure is explicit and the outbox expires pending work", () => {
  const coordinator = new Coordinator("session-123");
  coordinator.admit("connection-a");
  coordinator.depart();
  assert.equal(coordinator.available, false);
  assert.throws(() => coordinator.acceptChat("connection-a", message("message-001")), /unavailable/);

  const outbox = new Outbox(1);
  outbox.enqueue(message("message-002"), 10);
  outbox.expire(10);
  assert.equal(outbox.snapshot()[0].status, "failed");
});
