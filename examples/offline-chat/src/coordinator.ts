export type ChatEnvelope =
  | { version: 1; kind: "join"; messageId: string; sessionId: string; body: { displayName: string } }
  | { version: 1; kind: "joined"; messageId: string; sessionId: string; body: { sequence: number; resumeToken: string } }
  | { version: 1; kind: "chat"; messageId: string; sessionId: string; body: { text: string } }
  | { version: 1; kind: "chatAck"; messageId: string; sessionId: string; body: { acknowledgedMessageId: string } }
  | { version: 1; kind: "read"; messageId: string; sessionId: string; body: { sequence: number } }
  | { version: 1; kind: "typing"; messageId: string; sessionId: string; body: { active: boolean } }
  | { version: 1; kind: "resume"; messageId: string; sessionId: string; body: { afterSequence: number; token: string } }
  | { version: 1; kind: "leave"; messageId: string; sessionId: string; body: Record<string, never> };

export type Receipt = "pending" | "transportAccepted" | "recipientDelivered" | "read" | "failed";

export interface TimelineEntry {
  sessionId: string;
  sequence: number;
  messageId: string;
  senderConnectionId: string;
  text: string;
  receipts: Record<string, Receipt>;
}

export interface OutboxEntry {
  envelope: ChatEnvelope;
  deadlineMs: number;
  status: "pending" | "coordinatorAccepted" | "failed";
}

export class Outbox {
  readonly #entries = new Map<string, OutboxEntry>();

  constructor(readonly maxMessages = 64) {
    if (!Number.isSafeInteger(maxMessages) || maxMessages <= 0) {
      throw new Error("outbox capacity must be positive");
    }
  }

  enqueue(envelope: unknown, deadlineMs: number): OutboxEntry {
    validateEnvelope(envelope);
    if (!Number.isSafeInteger(deadlineMs) || deadlineMs < 0) {
      throw new Error("invalid outbox deadline");
    }
    const previous = this.#entries.get(envelope.messageId);
    if (previous) return previous;
    if (this.#entries.size >= this.maxMessages) throw new Error("outbox is full");
    const entry: OutboxEntry = { envelope, deadlineMs, status: "pending" };
    this.#entries.set(envelope.messageId, entry);
    return entry;
  }

  markCoordinatorAccepted(messageId: string): void {
    const entry = this.#entries.get(messageId);
    if (!entry) throw new Error("unknown outbox message");
    entry.status = "coordinatorAccepted";
  }

  expire(nowMs: number): void {
    for (const entry of this.#entries.values()) {
      if (entry.status === "pending" && nowMs >= entry.deadlineMs) entry.status = "failed";
    }
  }

  snapshot(): readonly OutboxEntry[] {
    return [...this.#entries.values()];
  }

  clear(): void {
    this.#entries.clear();
  }
}

export class Coordinator {
  readonly #participants = new Set<string>();
  readonly #history: TimelineEntry[] = [];
  readonly #dedup = new Map<string, TimelineEntry>();
  #sequence = 0;
  #available = true;

  constructor(
    readonly sessionId: string,
    readonly maxHistory: number = 200,
  ) {
    validateBoundedString(sessionId, "session ID");
    if (!Number.isSafeInteger(maxHistory) || maxHistory <= 0) {
      throw new Error("history capacity must be positive");
    }
  }

  get available(): boolean {
    return this.#available;
  }

  admit(connectionId: string): void {
    this.#assertAvailable();
    validateBoundedString(connectionId, "connection ID");
    this.#participants.add(connectionId);
  }

  remove(connectionId: string): void {
    this.#participants.delete(connectionId);
  }

  depart(): void {
    this.#available = false;
    this.#participants.clear();
  }

  acceptChat(connectionId: string, envelope: unknown): TimelineEntry {
    this.#assertAvailable();
    validateEnvelope(envelope);
    if (!this.#participants.has(connectionId)) throw new Error("sender is not admitted");
    if (envelope.sessionId !== this.sessionId) throw new Error("wrong chat session");
    if (envelope.kind !== "chat") throw new Error("expected a chat envelope");

    const dedupKey = keyFor(connectionId, envelope.messageId);
    const previous = this.#dedup.get(dedupKey);
    if (previous) return previous;

    const receipts: Record<string, Receipt> = {};
    for (const participant of this.#participants) {
      if (participant !== connectionId) receipts[participant] = "pending";
    }
    const entry: TimelineEntry = {
      sessionId: this.sessionId,
      sequence: ++this.#sequence,
      messageId: envelope.messageId,
      senderConnectionId: connectionId,
      text: envelope.body.text,
      receipts,
    };
    this.#dedup.set(dedupKey, entry);
    this.#history.push(entry);
    while (this.#history.length > this.maxHistory) {
      const evicted = this.#history.shift();
      if (evicted) this.#dedup.delete(keyFor(evicted.senderConnectionId, evicted.messageId));
    }
    return entry;
  }

  recordReceipt(
    sequence: number,
    recipientConnectionId: string,
    receipt: Exclude<Receipt, "pending">,
  ): void {
    const entry = this.#history.find((candidate) => candidate.sequence === sequence);
    if (!entry || !(recipientConnectionId in entry.receipts)) {
      throw new Error("unknown message sequence or recipient");
    }
    entry.receipts[recipientConnectionId] = receipt;
  }

  historyAfter(sequence: number): { entries: TimelineEntry[]; gap: boolean } {
    if (!Number.isSafeInteger(sequence) || sequence < 0) throw new Error("invalid sequence");
    const oldest = this.#history[0]?.sequence ?? this.#sequence + 1;
    return {
      entries: this.#history.filter((entry) => entry.sequence > sequence),
      gap: sequence + 1 < oldest,
    };
  }

  clearHistory(): void {
    this.#history.length = 0;
    this.#dedup.clear();
  }

  #assertAvailable(): void {
    if (!this.#available) throw new Error("coordinator is unavailable");
  }
}

export function validateEnvelope(value: unknown): asserts value is ChatEnvelope {
  if (!isRecord(value)) throw new Error("envelope must be an object");
  if (value.version !== 1) throw new Error("unsupported envelope version");
  validateBoundedString(value.messageId, "message ID");
  validateBoundedString(value.sessionId, "session ID");
  if (!isRecord(value.body)) throw new Error("envelope body must be an object");

  switch (value.kind) {
    case "join":
      validateDisplayName(value.body.displayName);
      break;
    case "joined":
      validateSequence(value.body.sequence);
      validateBoundedString(value.body.resumeToken, "resume token");
      break;
    case "chat": {
      if (typeof value.body.text !== "string") throw new Error("chat text must be a string");
      const bytes = new TextEncoder().encode(value.body.text);
      if (bytes.length === 0 || bytes.length > 8 * 1024) {
        throw new Error("chat text is empty or too large");
      }
      break;
    }
    case "chatAck":
      validateBoundedString(value.body.acknowledgedMessageId, "acknowledged message ID");
      break;
    case "read":
      validateSequence(value.body.sequence);
      break;
    case "typing":
      if (typeof value.body.active !== "boolean") throw new Error("typing state must be boolean");
      break;
    case "resume":
      validateSequence(value.body.afterSequence);
      validateBoundedString(value.body.token, "resume token");
      break;
    case "leave":
      if (Object.keys(value.body).length !== 0) throw new Error("leave body must be empty");
      break;
    default:
      throw new Error("unknown envelope kind");
  }
}

function validateDisplayName(value: unknown): void {
  if (typeof value !== "string" || value.length === 0 || value.length > 80) {
    throw new Error("invalid display name");
  }
}

function validateBoundedString(value: unknown, label: string): asserts value is string {
  if (typeof value !== "string" || value.length < 8 || value.length > 128) {
    throw new Error(`invalid ${label}`);
  }
}

function validateSequence(value: unknown): asserts value is number {
  if (!Number.isSafeInteger(value) || (value as number) < 0) throw new Error("invalid sequence");
}

function isRecord(value: unknown): value is Record<string, unknown> {
  return typeof value === "object" && value !== null && !Array.isArray(value);
}

function keyFor(connectionId: string, messageId: string): string {
  return `${connectionId.length}:${connectionId}${messageId}`;
}
