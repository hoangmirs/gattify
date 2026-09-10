import type { BleBridge } from "./bridge.js";
import { defaultBridge, subscribe } from "./bridge.js";
import { decodeBytes, encodeBytes } from "./wire.js";
import type { DeliveryOutcome, DeviceId, PeerId } from "./types.js";
import { BleError } from "./types.js";

export interface PeerSendResult {
  messageId: number;
  delivery: DeliveryOutcome;
}

export interface Peer {
  readonly id: PeerId;
  send(bytes: Uint8Array, options?: { timeoutMs?: number; signal?: AbortSignal }): Promise<PeerSendResult>;
  onMessage(callback: (bytes: Uint8Array) => void): () => void;
  close(): Promise<void>;
}

export interface Endpoint {
  readonly endpointId: string;
  dial(deviceId: DeviceId): Promise<Peer>;
  onPeer(callback: (peer: Peer) => void): () => void;
  close(): Promise<void>;
}

export interface PeerOptions {
  serviceUuid: string;
  localName?: string;
  maxLogicalPayload?: number;
  bridge?: BleBridge;
}

export async function createEndpoint(options: PeerOptions): Promise<Endpoint> {
  const bridge = options.bridge ?? (await defaultBridge());
  try {
    const result = await bridge.invoke<{ endpointId: string }>("plugin:gattify|create_endpoint", {
      options: {
        serviceUuid: options.serviceUuid,
        localName: options.localName ?? null,
        maxLogicalPayload: options.maxLogicalPayload ?? 16 * 1024,
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
  readonly #callbacks = new Set<(peer: Peer) => void>();
  readonly #unlisten: () => void;
  #closed = false;

  constructor(
    private readonly bridge: BleBridge,
    readonly endpointId: string,
  ) {
    this.#unlisten = subscribe<{ endpointId: string; peerId: PeerId }>(
      bridge,
      "gattify://peer-ready",
      (ready) => {
        if (ready.endpointId !== this.endpointId) return;
        const peer = new PeerHandle(this.bridge, ready.peerId);
        for (const callback of this.#callbacks) callback(peer);
      },
    );
  }

  async dial(deviceId: DeviceId): Promise<Peer> {
    const result = await this.bridge.invoke<{ peerId: PeerId }>("plugin:gattify|dial_peer", {
      endpointId: this.endpointId,
      deviceId,
    });
    return new PeerHandle(this.bridge, result.peerId);
  }

  onPeer(callback: (peer: Peer) => void): () => void {
    this.#callbacks.add(callback);
    return () => this.#callbacks.delete(callback);
  }

  async close(): Promise<void> {
    if (this.#closed) return;
    this.#closed = true;
    this.#unlisten();
    await this.bridge.invoke("plugin:gattify|close_endpoint", { endpointId: this.endpointId });
  }
}

class PeerHandle implements Peer {
  readonly #callbacks = new Set<(bytes: Uint8Array) => void>();
  readonly #unlisten: () => void;
  #closed = false;

  constructor(
    private readonly bridge: BleBridge,
    readonly id: PeerId,
  ) {
    this.#unlisten = subscribe<{ peerId: PeerId; valueBase64: string }>(
      bridge,
      "gattify://peer-message",
      (message) => {
        if (message.peerId !== this.id) return;
        const bytes = decodeBytes(message.valueBase64);
        for (const callback of this.#callbacks) callback(bytes);
      },
    );
  }

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
    this.#callbacks.add(callback);
    return () => this.#callbacks.delete(callback);
  }

  async close(): Promise<void> {
    if (this.#closed) return;
    this.#closed = true;
    this.#unlisten();
    await this.bridge.invoke("plugin:gattify|close_peer", { peerId: this.id });
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
