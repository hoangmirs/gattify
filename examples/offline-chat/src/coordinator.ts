export type ChatEnvelope =
  | {
      version: 1;
      kind: "join";
      messageId: string;
      sessionId: string;
      body: { displayName: string };
    }
  | {
      version: 1;
      kind: "chat";
      messageId: string;
      sessionId: string;
      body: { text: string };
    }
  | {
      version: 1;
      kind: "chatAck";
      messageId: string;
      sessionId: string;
      body: { acknowledgedMessageId: string };
    }
  | {
      version: 1;
      kind: "read";
      messageId: string;
      sessionId: string;
      body: { sequence: number };
    }
  | {
      version: 1;
      kind: "typing";
      messageId: string;
      sessionId: string;
      body: { active: boolean };
    }
  | {
      version: 1;
      kind: "resume";
      messageId: string;
      sessionId: string;
      body: { afterSequence: number; token: string };
    }
  | {
      version: 1;
      kind: "leave";
      messageId: string;
      sessionId: string;
      body: Record<string, never>;
    };

export type Receipt = "pending" | "transportAccepted" | "recipientDelivered" | "read" | "failed";

export interface TimelineEntry {
  sessionId: string;
  sequence: number;
  messageId: string;
  senderConnectionId: string;
  text: string;
  receipts: Record<string, Receipt>;
}

export class Coordinator {
  readonly #participants = new Set<string>();
  readonly #history: TimelineEntry[] = [];
  readonly #dedup = new Map<string, TimelineEntry>();
  #sequence = 0;

  constructor(
    readonly sessionId: string,
    readonly maxHistory: number = 200,
  ) {}

  admit(connectionId: string): void {
    this.#participants.add(connectionId);
  }

  remove(connectionId: string): void {
    this.#participants.delete(connectionId);
  }

  acceptChat(connectionId: string, envelope: ChatEnvelope): TimelineEntry {
    validateEnvelope(envelope);
    if (!this.#participants.has(connectionId)) throw new Error("sender is not admitted");
    if (envelope.sessionId !== this.sessionId) throw new Error("wrong chat session");
    if (envelope.kind !== "chat") throw new Error("expected a chat envelope");

    const dedupKey = connectionId + ":" + envelope.messageId;
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
    while (this.#history.length > this.maxHistory) this.#history.shift();
    return entry;
  }

  recordReceipt(
    messageId: string,
    recipientConnectionId: string,
    receipt: Exclude<Receipt, "pending">,
  ): void {
    const entry = this.#history.find((candidate) => candidate.messageId === messageId);
    if (!entry || !(recipientConnectionId in entry.receipts)) {
      throw new Error("unknown message or recipient");
    }
    entry.receipts[recipientConnectionId] = receipt;
  }

  historyAfter(sequence: number): { entries: TimelineEntry[]; gap: boolean } {
    const oldest = this.#history[0]?.sequence ?? this.#sequence + 1;
    return {
      entries: this.#history.filter((entry) => entry.sequence > sequence),
      gap: sequence + 1 < oldest,
    };
  }
}

export function validateEnvelope(value: ChatEnvelope): void {
  if (value.version !== 1) throw new Error("unsupported envelope version");
  if (value.messageId.length < 8 || value.messageId.length > 128) {
    throw new Error("invalid message ID");
  }
  if (value.sessionId.length < 8 || value.sessionId.length > 128) {
    throw new Error("invalid session ID");
  }
  if (value.kind === "chat") {
    const bytes = new TextEncoder().encode(value.body.text);
    if (bytes.length === 0 || bytes.length > 8 * 1024) {
      throw new Error("chat text is empty or too large");
    }
  }
}

