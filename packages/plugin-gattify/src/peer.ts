import type { BleBridge } from "./bridge.js";
import { defaultBridge, subscribe } from "./bridge.js";
import { decodeBytes, encodeBytes } from "./wire.js";
import type { DeliveryOutcome, DeviceId, PeerId } from "./types.js";
import { BleError } from "./types.js";

/** Bytes a peer keeps for the app until its first `onMessage` callback. */
const EARLY_MESSAGE_BYTES = 1024 * 1024;
/** Closed peers an endpoint remembers, so a late dial reply finds them closed. */
const CLOSED_PEER_MEMORY = 64;
/** Events an endpoint keeps while `create_endpoint` has not answered yet. */
const PENDING_EVENT_LIMIT = 256;

export interface PeerSendResult {
  messageId: number;
  delivery: DeliveryOutcome;
}

/** Why a peer closed: this side closed it, the other side sent CLOSE, or the link went away. */
export type PeerCloseReason = "local" | "remote" | "lost";

export interface Peer {
  readonly id: PeerId;
  send(bytes: Uint8Array, options?: { timeoutMs?: number; signal?: AbortSignal }): Promise<PeerSendResult>;
  /**
   * Messages that arrived before the first callback, up to 1 MiB, are
   * delivered to it, even after the peer closed.
   */
  onMessage(callback: (bytes: Uint8Array) => void): () => void;
  onClose(callback: (reason: PeerCloseReason) => void): () => void;
  close(): Promise<void>;
}

export interface Endpoint {
  readonly endpointId: string;
  dial(deviceId: DeviceId): Promise<Peer>;
  /**
   * Peers that dialed this endpoint. Peers that arrived before the first
   * callback are delivered to it.
   */
  onPeer(callback: (peer: Peer) => void): () => void;
  close(): Promise<void>;
}

export interface PeerOptions {
  serviceUuid: string;
  localName?: string;
  maxLogicalPayload?: number;
  /** A host listens: it registers the service and advertises it. False by default. */
  listen?: boolean;
  bridge?: BleBridge;
}

interface PeerReady {
  endpointId: string;
  peerId: PeerId;
  dialed: boolean;
}

interface PeerMessage {
  peerId: PeerId;
  valueBase64: string;
}

interface PeerClosed {
  peerId: PeerId;
  reason: PeerCloseReason;
}

type PeerEventMessage =
  | { kind: "ready"; payload: PeerReady }
  | { kind: "message"; payload: PeerMessage }
  | { kind: "closed"; payload: PeerClosed };

export async function createEndpoint(options: PeerOptions): Promise<Endpoint> {
  const bridge = options.bridge ?? (await defaultBridge());
  // Listen before the endpoint exists: a host advertises before
  // create_endpoint answers, and a fast joiner could otherwise be missed.
  const endpoint = new EndpointHandle(bridge);
  try {
    const result = await bridge.invoke<{ endpointId: string }>("plugin:gattify|create_endpoint", {
      options: {
        serviceUuid: options.serviceUuid,
        localName: options.localName ?? null,
        maxLogicalPayload: options.maxLogicalPayload ?? 16 * 1024,
        listen: options.listen ?? false,
      },
    });
    endpoint.activate(result.endpointId);
    return endpoint;
  } catch (error) {
    endpoint.dispose();
    if (isUnsupported(error)) {
      throw new BleError({
        code: "unsupported",
        message: "The Rust plugin was built without a usable peer-capable native backend",
      });
    }
    throw error;
  }
}

class EndpointHandle implements Endpoint {
  readonly #peers = new Map<PeerId, PeerHandle>();
  readonly #closedOrder: PeerId[] = [];
  readonly #callbacks = new Set<(peer: Peer) => void>();
  readonly #unannounced: PeerHandle[] = [];
  readonly #unlisten: Array<() => void>;
  #pending: PeerEventMessage[] | undefined = [];
  #endpointId: string | undefined;
  #closed = false;

  constructor(private readonly bridge: BleBridge) {
    this.#unlisten = [
      subscribe<PeerReady>(bridge, "gattify://peer-ready", (payload) =>
        this.#receive({ kind: "ready", payload }),
      ),
      subscribe<PeerMessage>(bridge, "gattify://peer-message", (payload) =>
        this.#receive({ kind: "message", payload }),
      ),
      subscribe<PeerClosed>(bridge, "gattify://peer-closed", (payload) =>
        this.#receive({ kind: "closed", payload }),
      ),
    ];
  }

  get endpointId(): string {
    if (this.#endpointId === undefined) throw new Error("the endpoint is not created yet");
    return this.#endpointId;
  }

  activate(endpointId: string): void {
    this.#endpointId = endpointId;
    const pending = this.#pending ?? [];
    this.#pending = undefined;
    for (const event of pending) this.#apply(event);
  }

  dispose(): void {
    this.#pending = undefined;
    for (const unlisten of this.#unlisten) unlisten();
  }

  async dial(deviceId: DeviceId): Promise<Peer> {
    const result = await this.bridge.invoke<{ peerId: PeerId }>("plugin:gattify|dial_peer", {
      endpointId: this.endpointId,
      deviceId,
    });
    return this.#adopt(result.peerId);
  }

  onPeer(callback: (peer: Peer) => void): () => void {
    this.#callbacks.add(callback);
    for (const peer of this.#unannounced.splice(0)) callback(peer);
    return () => this.#callbacks.delete(callback);
  }

  async close(): Promise<void> {
    if (this.#closed) return;
    this.#closed = true;
    try {
      await this.bridge.invoke("plugin:gattify|close_endpoint", { endpointId: this.endpointId });
    } finally {
      this.dispose();
      for (const peer of this.#peers.values()) peer.finish("local");
    }
  }

  #receive(event: PeerEventMessage): void {
    if (this.#pending !== undefined) {
      if (this.#pending.length < PENDING_EVENT_LIMIT) this.#pending.push(event);
      return;
    }
    this.#apply(event);
  }

  #apply(event: PeerEventMessage): void {
    switch (event.kind) {
      case "ready": {
        if (event.payload.endpointId !== this.#endpointId) return;
        const known = this.#peers.has(event.payload.peerId);
        const peer = this.#adopt(event.payload.peerId);
        if (event.payload.dialed || known) return;
        if (this.#callbacks.size === 0) this.#unannounced.push(peer);
        for (const callback of this.#callbacks) callback(peer);
        return;
      }
      case "message":
        this.#peers.get(event.payload.peerId)?.deliver(decodeBytes(event.payload.valueBase64));
        return;
      case "closed":
        this.#peers.get(event.payload.peerId)?.finish(event.payload.reason);
    }
  }

  /**
   * Returns the one handle of a peer. The `peer-ready` event and the reply of
   * `dial` can arrive in either order, and the peer may have closed already.
   */
  #adopt(peerId: PeerId): PeerHandle {
    let peer = this.#peers.get(peerId);
    if (peer === undefined) {
      peer = new PeerHandle(this.bridge, peerId, () => this.#remember(peerId));
      this.#peers.set(peerId, peer);
      if (this.#closed) peer.finish("local");
    }
    return peer;
  }

  #remember(peerId: PeerId): void {
    this.#closedOrder.push(peerId);
    while (this.#closedOrder.length > CLOSED_PEER_MEMORY) {
      const forgotten = this.#closedOrder.shift();
      if (forgotten !== undefined) this.#peers.delete(forgotten);
    }
  }
}

class PeerHandle implements Peer {
  readonly #messages = new Set<(bytes: Uint8Array) => void>();
  readonly #closes = new Set<(reason: PeerCloseReason) => void>();
  readonly #early: Uint8Array[] = [];
  #earlyBytes = 0;
  #reason: PeerCloseReason | undefined;
  #closing = false;

  constructor(
    private readonly bridge: BleBridge,
    readonly id: PeerId,
    private readonly closed: () => void,
  ) {}

  send(
    bytes: Uint8Array,
    options: { timeoutMs?: number; signal?: AbortSignal } = {},
  ): Promise<PeerSendResult> {
    if (options.signal?.aborted) {
      return Promise.reject(new DOMException("The peer send was cancelled", "AbortError"));
    }
    return this.bridge.invoke("plugin:gattify|send_peer", {
      peerId: this.id,
      valueBase64: encodeBytes(bytes),
      timeoutMs: options.timeoutMs ?? 30_000,
    });
  }

  onMessage(callback: (bytes: Uint8Array) => void): () => void {
    this.#messages.add(callback);
    const early = this.#early.splice(0);
    this.#earlyBytes = 0;
    for (const bytes of early) callback(bytes);
    return () => this.#messages.delete(callback);
  }

  onClose(callback: (reason: PeerCloseReason) => void): () => void {
    const reason = this.#reason;
    if (reason !== undefined) {
      queueMicrotask(() => callback(reason));
      return () => {};
    }
    this.#closes.add(callback);
    return () => this.#closes.delete(callback);
  }

  async close(): Promise<void> {
    if (this.#closing || this.#reason !== undefined) return;
    this.#closing = true;
    try {
      await this.bridge.invoke("plugin:gattify|close_peer", { peerId: this.id });
    } finally {
      this.finish("local");
    }
  }

  deliver(bytes: Uint8Array): void {
    if (this.#reason !== undefined) return;
    if (this.#messages.size > 0) {
      for (const callback of this.#messages) callback(bytes);
      return;
    }
    if (this.#earlyBytes + bytes.length > EARLY_MESSAGE_BYTES) {
      console.warn(`gattify: dropped a ${bytes.length}-byte message of ${this.id}: no onMessage callback`);
      return;
    }
    this.#early.push(bytes);
    this.#earlyBytes += bytes.length;
  }

  /** Ends the peer. Messages kept for a first `onMessage` stay available. */
  finish(reason: PeerCloseReason): void {
    if (this.#reason !== undefined) return;
    this.#reason = reason;
    this.closed();
    for (const callback of this.#closes) callback(reason);
    this.#closes.clear();
  }
}

function isUnsupported(value: unknown): boolean {
  return (
    typeof value === "object" &&
    value !== null &&
    "code" in value &&
    (value as { code: unknown }).code === "unsupported"
  );
}
