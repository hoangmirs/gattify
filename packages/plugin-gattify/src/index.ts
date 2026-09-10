import { defaultBridge, subscribe, type BleBridge } from "./bridge.js";
import {
  decodeBytes,
  dispatch,
  dispatchStatus,
  encodeBytes,
  expectReply,
  requestPermission,
} from "./wire.js";
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
    const roles = (["scan", "connect", "advertise"] as const).filter((role) => request[role]);
    if (roles.length === 0) return this.checkPermissions();
    let permissions: PermissionState | undefined;
    for (const role of roles) {
      permissions = expectReply(
        await requestPermission<PermissionState>(this.#bridge, role),
        "permissions",
      );
    }
    return permissions as PermissionState;
  }

  async scan(options: Partial<ScanOptions> = {}): Promise<ScanHandle> {
    this.#assertOpen();
    const payload = expectReply<{ scanId: ScanId }>(
      await dispatch(this.#bridge, {
        kind: "startScan",
        payload: {
          serviceUuids: options.serviceUuids ?? [],
          timeoutMs: options.timeoutMs ?? null,
        },
      }, { deadlineMillis: options.timeoutMs ?? undefined, signal: options.signal }),
      "scanStarted",
    );
    return new Scan(this.#bridge, payload.scanId);
  }

  async connect(deviceId: DeviceId, options: ConnectOptions = {}): Promise<ConnectionHandle> {
    this.#assertOpen();
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
        { deadlineMillis: deadline, signal: options.signal },
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
    await this.#bridge.invoke("plugin:gattify|close");
  }

  #assertOpen(): void {
    if (this.#closed) throw new Error("BLE session is closed");
  }
}

class Scan implements ScanHandle {
  readonly #devices = new Map<DeviceId, DiscoveredDevice>();
  readonly #callbacks = new Set<(device: DiscoveredDevice) => void>();
  readonly #unlisten: () => void;
  #closed = false;

  constructor(
    private readonly bridge: BleBridge,
    readonly id: ScanId,
  ) {
    this.#unlisten = subscribe<DiscoveredDevice>(bridge, "gattify://scan-result", (device) => {
      if (device.scanId !== this.id) return;
      this.#devices.set(device.id, device);
      for (const callback of this.#callbacks) callback(device);
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
    this.#unlisten();
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
  readonly #unlisten: () => void;
  #closed = false;

  constructor(
    private readonly bridge: BleBridge,
    readonly id: SubscriptionId,
  ) {
    this.#unlisten = subscribe<{ subscriptionId: SubscriptionId; valueBase64: string }>(
      bridge,
      "gattify://characteristic-value",
      (received) => {
        if (received.subscriptionId !== this.id) return;
        const value = decodeBytes(received.valueBase64);
        for (const callback of this.#callbacks) callback(value);
      },
    );
  }

  onValue(callback: (value: Uint8Array) => void): () => void {
    this.#callbacks.add(callback);
    return () => this.#callbacks.delete(callback);
  }

  async close(): Promise<void> {
    if (this.#closed) return;
    this.#closed = true;
    this.#unlisten();
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
    this.#assertOpen();
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

export async function createBle(options: { bridge?: BleBridge } = {}): Promise<BleSession> {
  return new Session(options.bridge ?? (await defaultBridge()));
}
