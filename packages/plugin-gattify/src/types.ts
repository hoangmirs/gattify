export type Brand<Value, Name extends string> = Value & { readonly __brand: Name };

export type AdapterId = Brand<string, "AdapterId">;
export type DeviceId = Brand<string, "DeviceId">;
export type ConnectionId = Brand<string, "ConnectionId">;
export type ServerId = Brand<string, "ServerId">;
export type PeerId = Brand<string, "PeerId">;
export type ScanId = Brand<string, "ScanId">;
export type SubscriptionId = Brand<string, "SubscriptionId">;
export type OperationId = Brand<string, "OperationId">;
export type ServiceHandle = Brand<string, "ServiceHandle">;
export type CharacteristicHandle = Brand<string, "CharacteristicHandle">;

export type AdapterState =
  | "unknown"
  | "unavailable"
  | "unauthorized"
  | "poweredOff"
  | "resetting"
  | "poweredOn";

export type SupportLevel = "supported" | "unsupported" | "unknown";

export interface Support {
  level: SupportLevel;
  reason: string;
  description?: string | null;
}

export interface Capabilities {
  central: Support;
  peripheral: Support;
  advertising: Support;
  targetedNotify: Support;
  simultaneousRoles: Support;
  background: Support;
  maxConnections: number | null;
  maxAdvertisingDataLength: number | null;
}

export type PermissionOutcome =
  | "granted"
  | "promptable"
  | "deniedPermanently"
  | "restricted"
  | "notRequired"
  | "unknown";

export interface PermissionState {
  scan: PermissionOutcome;
  connect: PermissionOutcome;
  advertise: PermissionOutcome;
}

export interface PermissionRequest {
  scan: boolean;
  connect: boolean;
  advertise: boolean;
}

export interface ScanOptions {
  serviceUuids: string[];
  timeoutMs?: number | null;
  signal?: AbortSignal;
}

export interface ConnectOptions {
  timeoutMs?: number | null;
  signal?: AbortSignal;
}

export interface AdvertisementData {
  localName?: string | null;
  serviceData: Array<{ serviceUuid: string; bytesBase64: string }>;
  manufacturerData: Array<{ companyId: number; bytesBase64: string }>;
  connectable?: boolean | null;
}

export interface DiscoveredDevice {
  id: DeviceId;
  name: string | null;
  rssi: number | null;
  serviceUuids: string[];
  advertisement: AdvertisementData | null;
  observedAtMillis: number;
  scanId: ScanId;
}

export interface LinkLimits {
  writeWithResponse: number | null;
  writeWithoutResponse: number | null;
  notification: number | null;
  attMtu: number | null;
}

export interface CharacteristicProperties {
  read: boolean;
  write: boolean;
  writeWithoutResponse: boolean;
  notify: boolean;
  indicate: boolean;
}

export interface CharacteristicInstance {
  handle: CharacteristicHandle;
  uuid: string;
  properties: CharacteristicProperties;
}

export interface ServiceInstance {
  handle: ServiceHandle;
  uuid: string;
  characteristics: CharacteristicInstance[];
}

export interface LocalCharacteristic {
  instanceKey: string;
  uuid: string;
  properties: CharacteristicProperties;
  initialValueBase64?: string | null;
  maxValueLength: number;
}

export interface LocalService {
  instanceKey: string;
  uuid: string;
  primary: boolean;
  characteristics: LocalCharacteristic[];
}

export interface ServerDefinition {
  services: LocalService[];
}

export interface AdvertisingOptions {
  serviceUuid: string;
  localName?: string | null;
  localNameOptional: boolean;
}

export interface AdvertisingReport {
  localNameIncluded: boolean;
  localNameTruncated: boolean;
}

export type WriteType = "withResponse" | "withoutResponse";

export type ErrorCode =
  | "permissionDenied"
  | "bluetoothOff"
  | "unavailable"
  | "unsupported"
  | "invalidArgument"
  | "invalidHandle"
  | "busy"
  | "timeout"
  | "disconnected"
  | "payloadTooLarge"
  | "queueFull"
  | "protocolMismatch"
  | "cancelled"
  | "internal";

export type DeliveryOutcome = "notSubmitted" | "unknown" | "transportAcknowledged";

export interface BleErrorShape {
  code: ErrorCode;
  message: string;
  operationId?: OperationId;
  resourceId?: string;
  delivery?: DeliveryOutcome;
  nativeCode?: string;
}

export class BleError extends Error implements BleErrorShape {
  readonly code: ErrorCode;
  readonly operationId?: OperationId;
  readonly resourceId?: string;
  readonly delivery?: DeliveryOutcome;
  readonly nativeCode?: string;

  constructor(value: BleErrorShape) {
    super(value.message);
    this.name = "BleError";
    this.code = value.code;
    if (value.operationId !== undefined) this.operationId = value.operationId;
    if (value.resourceId !== undefined) this.resourceId = value.resourceId;
    if (value.delivery !== undefined) this.delivery = value.delivery;
    if (value.nativeCode !== undefined) this.nativeCode = value.nativeCode;
  }
}

