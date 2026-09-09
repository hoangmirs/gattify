import type { BleBridge, Unlisten } from "./bridge.js";
import { defaultBridge } from "./bridge.js";
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
    const result = await bridge.invoke<{ endpointId: string }>("plugin:ble|create_endpoint", {
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
  #unlisten: Unlisten | undefined;
  #closed = false;

  constructor(
    private readonly bridge: BleBridge,
    readonly endpointId: string,
  ) {
    void bridge
      .listen?.<{ endpointId: string; peerId: PeerId }>("ble://peer-ready", (event) => {
        if (event.payload.endpointId !== this.endpointId || this.#closed) return;
        const peer = new PeerHandle(this.bridge, event.payload.peerId);
        for (const callback of this.#callbacks) callback(peer);
      })
      .then((unlisten) => {
        this.#unlisten = unlisten;
      });
  }

  async dial(deviceId: DeviceId): Promise<Peer> {
    const result = await this.bridge.invoke<{ peerId: PeerId }>("plugin:ble|dial_peer", {
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
    this.#unlisten?.();
    await this.bridge.invoke("plugin:ble|close_endpoint", { endpointId: this.endpointId });
  }
}

class PeerHandle implements Peer {
  readonly #callbacks = new Set<(bytes: Uint8Array) => void>();
  #unlisten: Unlisten | undefined;
  #closed = false;

  constructor(
    private readonly bridge: BleBridge,
    readonly id: PeerId,
  ) {
    void bridge
      .listen?.<{ peerId: PeerId; valueBase64: string }>("ble://peer-message", (event) => {
        if (event.payload.peerId !== this.id || this.#closed) return;
        const bytes = decodeBytes(event.payload.valueBase64);
        for (const callback of this.#callbacks) callback(bytes);
      })
      .then((unlisten) => {
        this.#unlisten = unlisten;
      });
  }

  send(
    bytes: Uint8Array,
    options: { timeoutMs?: number; signal?: AbortSignal } = {},
  ): Promise<PeerSendResult> {
    if (options.signal?.aborted) {
      return Promise.reject(new DOMException("The peer send was cancelled", "AbortError"));
    }
    return this.bridge.invoke("plugin:ble|send_peer", {
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
    this.#unlisten?.();
    await this.bridge.invoke("plugin:ble|close_peer", { peerId: this.id });
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
