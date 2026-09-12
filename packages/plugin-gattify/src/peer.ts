import type { BleBridge } from "./bridge.js";
import { defaultBridge, subscribe } from "./bridge.js";
import { decodeBytes, encodeBytes } from "./wire.js";
import type { DeliveryOutcome, DeviceId, PeerId } from "./types.js";
import { BleError } from "./types.js";

/** Messages a peer keeps until the app registers `onMessage`. */
const EARLY_MESSAGE_LIMIT = 64;

export interface PeerSendResult {
  messageId: number;
  delivery: DeliveryOutcome;
}

/** Why a peer closed: this side closed it, the other side sent CLOSE, or the link went away. */
export type PeerCloseReason = "local" | "remote" | "lost";

export interface Peer {
  readonly id: PeerId;
  send(bytes: Uint8Array, options?: { timeoutMs?: number; signal?: AbortSignal }): Promise<PeerSendResult>;
  /** Messages that arrived before the first callback are delivered to it. */
  onMessage(callback: (bytes: Uint8Array) => void): () => void;
  onClose(callback: (reason: PeerCloseReason) => void): () => void;
  close(): Promise<void>;
}

export interface Endpoint {
  readonly endpointId: string;
  dial(deviceId: DeviceId): Promise<Peer>;
  /** Peers that dialed this endpoint. A listening endpoint receives them. */
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

export async function createEndpoint(options: PeerOptions): Promise<Endpoint> {
  const bridge = options.bridge ?? (await defaultBridge());
  try {
    const result = await bridge.invoke<{ endpointId: string }>("plugin:gattify|create_endpoint", {
      options: {
        serviceUuid: options.serviceUuid,
        localName: options.localName ?? null,
        maxLogicalPayload: options.maxLogicalPayload ?? 16 * 1024,
        listen: options.listen ?? false,
      },
    });
    return new EndpointHandle(bridge, result.endpointId);
  } catch (error) {
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
  readonly #callbacks = new Set<(peer: Peer) => void>();
  readonly #unlisten: Array<() => void>;
  #closed = false;

  constructor(
    private readonly bridge: BleBridge,
    readonly endpointId: string,
  ) {
    this.#unlisten = [
      subscribe<PeerReady>(bridge, "gattify://peer-ready", (ready) => {
        if (ready.endpointId !== this.endpointId) return;
        const peer = this.#adopt(ready.peerId);
        if (ready.dialed) return;
        for (const callback of this.#callbacks) callback(peer);
      }),
      subscribe<{ peerId: PeerId; valueBase64: string }>(
        bridge,
        "gattify://peer-message",
        (message) => this.#peers.get(message.peerId)?.deliver(decodeBytes(message.valueBase64)),
      ),
      subscribe<{ peerId: PeerId; reason: PeerCloseReason }>(
        bridge,
        "gattify://peer-closed",
        (closed) => this.#peers.get(closed.peerId)?.finish(closed.reason),
      ),
    ];
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
    return () => this.#callbacks.delete(callback);
  }

  async close(): Promise<void> {
    if (this.#closed) return;
    this.#closed = true;
    try {
      await this.bridge.invoke("plugin:gattify|close_endpoint", { endpointId: this.endpointId });
    } finally {
      for (const unlisten of this.#unlisten) unlisten();
      for (const peer of [...this.#peers.values()]) peer.finish("local");
    }
  }

  /**
   * Returns the handle of a peer. The `peer-ready` event and the reply of
   * `dial` can arrive in either order; both give the same handle.
   */
  #adopt(peerId: PeerId): PeerHandle {
    let peer = this.#peers.get(peerId);
    if (peer === undefined) {
      peer = new PeerHandle(this.bridge, peerId, () => this.#peers.delete(peerId));
      this.#peers.set(peerId, peer);
    }
    return peer;
  }
}

class PeerHandle implements Peer {
  readonly #messages = new Set<(bytes: Uint8Array) => void>();
  readonly #closes = new Set<(reason: PeerCloseReason) => void>();
  readonly #early: Uint8Array[] = [];
  #reason: PeerCloseReason | undefined;
  #closing = false;

  constructor(
    private readonly bridge: BleBridge,
    readonly id: PeerId,
    private readonly forget: () => void,
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
    for (const bytes of this.#early.splice(0)) callback(bytes);
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
    if (this.#messages.size === 0) {
      if (this.#early.length < EARLY_MESSAGE_LIMIT) this.#early.push(bytes);
      return;
    }
    for (const callback of this.#messages) callback(bytes);
  }

  finish(reason: PeerCloseReason): void {
    if (this.#reason !== undefined) return;
    this.#reason = reason;
    this.forget();
    for (const callback of this.#closes) callback(reason);
    this.#closes.clear();
    this.#messages.clear();
    this.#early.length = 0;
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
