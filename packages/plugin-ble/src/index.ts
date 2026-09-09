import { defaultBridge, type BleBridge, type Unlisten } from "./bridge.js";
import { decodeBytes, dispatch, dispatchStatus, encodeBytes, expectReply } from "./wire.js";
import type {
  AdapterState,
  AdvertisingOptions,
  AdvertisingReport,
  Capabilities,
  CharacteristicHandle,
  ConnectOptions,
  ConnectionId,
  DeviceId,
  DiscoveredDevice,
  LinkLimits,
  PeerId,
  PermissionRequest,
  PermissionState,
  ScanId,
  ScanOptions,
  ServerDefinition,
  ServerId,
  ServiceInstance,
  SubscriptionId,
  WriteType,
} from "./types.js";

export * from "./types.js";
export type { BleBridge } from "./bridge.js";

export interface Connected {
  connectionId: ConnectionId;
  limits: LinkLimits;
}

export interface ScanHandle {
  readonly id: ScanId;
  snapshot(): readonly DiscoveredDevice[];
  onUpdate(callback: (device: DiscoveredDevice) => void): () => void;
  stop(): Promise<void>;
}

export interface SubscriptionHandle {
  readonly id: SubscriptionId;
  onValue(callback: (value: Uint8Array) => void): () => void;
  close(): Promise<void>;
}

export interface ConnectionHandle {
  readonly id: ConnectionId;
  readonly limits: LinkLimits;
  discoverServices(): Promise<ServiceInstance[]>;
  read(characteristic: CharacteristicHandle): Promise<Uint8Array>;
  write(
    characteristic: CharacteristicHandle,
    bytes: Uint8Array,
    writeType?: WriteType,
  ): Promise<void>;
  subscribe(characteristic: CharacteristicHandle): Promise<SubscriptionHandle>;
  close(): Promise<void>;
}

export interface ServerHandle {
  readonly id: ServerId;
  startAdvertising(options: AdvertisingOptions): Promise<AdvertisingReport>;
  stopAdvertising(): Promise<void>;
  setValue(characteristicKey: string, bytes: Uint8Array): Promise<void>;
  notify(peerId: PeerId, characteristicKey: string, bytes: Uint8Array): Promise<void>;
  close(): Promise<void>;
}

export interface BleSession {
  getState(): Promise<AdapterState>;
  getCapabilities(): Promise<Capabilities>;
  checkPermissions(): Promise<PermissionState>;
  requestPermissions(request: PermissionRequest): Promise<PermissionState>;
  scan(options?: Partial<ScanOptions>): Promise<ScanHandle>;
  connect(deviceId: DeviceId, options?: ConnectOptions): Promise<ConnectionHandle>;
  createServer(definition: ServerDefinition): Promise<ServerHandle>;
  close(): Promise<void>;
}

class Session implements BleSession {
  readonly #bridge: BleBridge;
  #closed = false;

  constructor(bridge: BleBridge) {
    this.#bridge = bridge;
  }

  async getState(): Promise<AdapterState> {
    this.#assertOpen();
    return expectReply(await dispatchStatus<AdapterState>(this.#bridge, "get_state"), "state");
  }

  async getCapabilities(): Promise<Capabilities> {
    this.#assertOpen();
    return expectReply(
      await dispatchStatus<Capabilities>(this.#bridge, "get_capabilities"),
      "capabilities",
    );
  }

  async checkPermissions(): Promise<PermissionState> {
    this.#assertOpen();
    return expectReply(
      await dispatchStatus<PermissionState>(this.#bridge, "check_permissions"),
      "permissions",
    );
  }

  async requestPermissions(request: PermissionRequest): Promise<PermissionState> {
    this.#assertOpen();
    return expectReply(
      await dispatch<PermissionState>(this.#bridge, {
        kind: "requestPermissions",
        payload: request,
      }),
      "permissions",
    );
  }

  async scan(options: Partial<ScanOptions> = {}): Promise<ScanHandle> {
    this.#assertOpen();
    throwIfAborted(options.signal);
    const payload = expectReply<{ scanId: ScanId }>(
      await dispatch(this.#bridge, {
        kind: "startScan",
        payload: {
          serviceUuids: options.serviceUuids ?? [],
          timeoutMs: options.timeoutMs ?? null,
        },
      }),
      "scanStarted",
    );
    return new Scan(this.#bridge, payload.scanId);
  }

  async connect(deviceId: DeviceId, options: ConnectOptions = {}): Promise<ConnectionHandle> {
    this.#assertOpen();
    throwIfAborted(options.signal);
    const deadline = options.timeoutMs ?? undefined;
    const payload = expectReply<Connected>(
      await dispatch(
        this.#bridge,
        {
          kind: "connect",
          payload: {
            deviceId,
            options: { timeoutMs: options.timeoutMs ?? null },
          },
        },
        deadline,
      ),
      "connected",
    );
    return new Connection(this.#bridge, payload.connectionId, payload.limits);
  }

  async createServer(definition: ServerDefinition): Promise<ServerHandle> {
    this.#assertOpen();
    const payload = expectReply<{ serverId: ServerId }>(
      await dispatch(this.#bridge, { kind: "createServer", payload: definition }),
      "serverCreated",
    );
    return new Server(this.#bridge, payload.serverId);
  }

  async close(): Promise<void> {
    if (this.#closed) return;
    this.#closed = true;
    await this.#bridge.invoke("plugin:ble|close");
  }

  #assertOpen(): void {
    if (this.#closed) throw new Error("BLE session is closed");
  }
}

class Scan implements ScanHandle {
  readonly #devices = new Map<DeviceId, DiscoveredDevice>();
  readonly #callbacks = new Set<(device: DiscoveredDevice) => void>();
  #unlisten: Unlisten | undefined;
  #closed = false;

  constructor(
    private readonly bridge: BleBridge,
    readonly id: ScanId,
  ) {
    void bridge
      .listen?.<DiscoveredDevice>("ble://scan-result", (event) => {
        if (event.payload.scanId !== this.id || this.#closed) return;
        this.#devices.set(event.payload.id, event.payload);
        for (const callback of this.#callbacks) callback(event.payload);
      })
      .then((unlisten) => {
        this.#unlisten = unlisten;
      });
  }

  snapshot(): readonly DiscoveredDevice[] {
    return [...this.#devices.values()];
  }

  onUpdate(callback: (device: DiscoveredDevice) => void): () => void {
    this.#callbacks.add(callback);
    return () => this.#callbacks.delete(callback);
  }

  async stop(): Promise<void> {
    if (this.#closed) return;
    this.#closed = true;
    this.#unlisten?.();
    await dispatch(this.bridge, { kind: "stopScan", payload: { scanId: this.id } });
  }
}

class Connection implements ConnectionHandle {
  #closed = false;

  constructor(
    private readonly bridge: BleBridge,
    readonly id: ConnectionId,
    readonly limits: LinkLimits,
  ) {}

  async discoverServices(): Promise<ServiceInstance[]> {
    this.#assertOpen();
    return expectReply(
      await dispatch<ServiceInstance[]>(this.bridge, {
        kind: "discoverServices",
        payload: { connectionId: this.id },
      }),
      "services",
    );
  }

  async read(characteristic: CharacteristicHandle): Promise<Uint8Array> {
    this.#assertOpen();
    const payload = expectReply<{ valueBase64: string }>(
      await dispatch(this.bridge, {
        kind: "read",
        payload: { connectionId: this.id, characteristic },
      }),
      "bytes",
    );
    return decodeBytes(payload.valueBase64);
  }

  async write(
    characteristic: CharacteristicHandle,
    bytes: Uint8Array,
    writeType: WriteType = "withResponse",
  ): Promise<void> {
    this.#assertOpen();
    await dispatch(this.bridge, {
      kind: "write",
      payload: {
        connectionId: this.id,
        characteristic,
        valueBase64: encodeBytes(bytes),
        writeType,
      },
    });
  }

  async subscribe(characteristic: CharacteristicHandle): Promise<SubscriptionHandle> {
    this.#assertOpen();
    const payload = expectReply<{ subscriptionId: SubscriptionId }>(
      await dispatch(this.bridge, {
        kind: "subscribe",
        payload: { connectionId: this.id, characteristic },
      }),
      "subscriptionStarted",
    );
    return new Subscription(this.bridge, payload.subscriptionId);
  }

  async close(): Promise<void> {
    if (this.#closed) return;
    this.#closed = true;
    await dispatch(this.bridge, { kind: "disconnect", payload: { connectionId: this.id } });
  }

  #assertOpen(): void {
    if (this.#closed) throw new Error("BLE connection is closed");
  }
}

class Subscription implements SubscriptionHandle {
  readonly #callbacks = new Set<(value: Uint8Array) => void>();
  #unlisten: Unlisten | undefined;
  #closed = false;

  constructor(
    private readonly bridge: BleBridge,
    readonly id: SubscriptionId,
  ) {
    void bridge
      .listen?.<{ subscriptionId: SubscriptionId; valueBase64: string }>(
        "ble://characteristic-value",
        (event) => {
          if (event.payload.subscriptionId !== this.id || this.#closed) return;
          const value = decodeBytes(event.payload.valueBase64);
          for (const callback of this.#callbacks) callback(value);
        },
      )
      .then((unlisten) => {
        this.#unlisten = unlisten;
      });
  }

  onValue(callback: (value: Uint8Array) => void): () => void {
    this.#callbacks.add(callback);
    return () => this.#callbacks.delete(callback);
  }

  async close(): Promise<void> {
    if (this.#closed) return;
    this.#closed = true;
    this.#unlisten?.();
    await dispatch(this.bridge, {
      kind: "unsubscribe",
      payload: { subscriptionId: this.id },
    });
  }
}

class Server implements ServerHandle {
  #closed = false;

  constructor(
    private readonly bridge: BleBridge,
    readonly id: ServerId,
  ) {}

  async startAdvertising(options: AdvertisingOptions): Promise<AdvertisingReport> {
    this.#assertOpen();
    return expectReply(
      await dispatch<AdvertisingReport>(this.bridge, {
        kind: "startAdvertising",
        payload: { serverId: this.id, options },
      }),
      "advertisingStarted",
    );
  }

  async stopAdvertising(): Promise<void> {
    if (this.#closed) return;
    await dispatch(this.bridge, {
      kind: "stopAdvertising",
      payload: { serverId: this.id },
    });
  }

  async setValue(characteristicKey: string, bytes: Uint8Array): Promise<void> {
    this.#assertOpen();
    await dispatch(this.bridge, {
      kind: "setValue",
      payload: {
        serverId: this.id,
        characteristicKey,
        valueBase64: encodeBytes(bytes),
      },
    });
  }

  async notify(peerId: PeerId, characteristicKey: string, bytes: Uint8Array): Promise<void> {
    this.#assertOpen();
    await dispatch(this.bridge, {
      kind: "notify",
      payload: {
        serverId: this.id,
        peerId,
        characteristicKey,
        valueBase64: encodeBytes(bytes),
      },
    });
  }

  async close(): Promise<void> {
    if (this.#closed) return;
    this.#closed = true;
    await dispatch(this.bridge, { kind: "closeServer", payload: { serverId: this.id } });
  }

  #assertOpen(): void {
    if (this.#closed) throw new Error("BLE server is closed");
  }
}

function throwIfAborted(signal: AbortSignal | undefined): void {
  if (signal?.aborted) {
    throw new DOMException("The BLE operation was cancelled", "AbortError");
  }
}

export async function createBle(options: { bridge?: BleBridge } = {}): Promise<BleSession> {
  return new Session(options.bridge ?? (await defaultBridge()));
}
