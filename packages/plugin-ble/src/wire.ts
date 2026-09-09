import { BleError, type BleErrorShape } from "./types.js";
import type { BleBridge } from "./bridge.js";

export type Command =
  | { kind: "startScan"; payload: { serviceUuids: string[]; timeoutMs: number | null } }
  | { kind: "stopScan"; payload: { scanId: string } }
  | { kind: "connect"; payload: { deviceId: string; options: { timeoutMs: number | null } } }
  | { kind: "disconnect"; payload: { connectionId: string } }
  | { kind: "discoverServices"; payload: { connectionId: string } }
  | { kind: "read"; payload: { connectionId: string; characteristic: string } }
  | {
      kind: "write";
      payload: {
        connectionId: string;
        characteristic: string;
        valueBase64: string;
        writeType: "withResponse" | "withoutResponse";
      };
    }
  | { kind: "subscribe"; payload: { connectionId: string; characteristic: string } }
  | { kind: "unsubscribe"; payload: { subscriptionId: string } }
  | { kind: "createServer"; payload: unknown }
  | { kind: "closeServer"; payload: { serverId: string } }
  | { kind: "startAdvertising"; payload: { serverId: string; options: unknown } }
  | { kind: "stopAdvertising"; payload: { serverId: string } }
  | {
      kind: "setValue";
      payload: { serverId: string; characteristicKey: string; valueBase64: string };
    }
  | {
      kind: "notify";
      payload: {
        serverId: string;
        peerId: string;
        characteristicKey: string;
        valueBase64: string;
      };
    };

export interface Reply<T = unknown> {
  kind: string;
  payload?: T;
}

export async function dispatch<T>(
  bridge: BleBridge,
  command: Command,
  options: { deadlineMillis?: number | undefined; signal?: AbortSignal | undefined } = {},
): Promise<Reply<T>> {
  throwIfAborted(options.signal);
  const operationId = allocateOperationId();
  const invocation = bridge
    .invoke<Reply<T>>("plugin:ble|" + commandEndpoint(command), {
      request: {
        operationId,
        command,
        deadlineMillis: options.deadlineMillis ?? null,
      },
    })
    .catch(throwBridgeError);
  const signal = options.signal;
  if (signal === undefined) return invocation;

  let onAbort: (() => void) | undefined;
  const aborted = new Promise<never>((_resolve, reject) => {
    onAbort = () => {
      void bridge
        .invoke("plugin:ble|cancel", { request: { operationId } })
        .catch(() => undefined);
      reject(new DOMException("The BLE operation was cancelled", "AbortError"));
    };
    signal.addEventListener("abort", onAbort, { once: true });
  });
  try {
    return await Promise.race([invocation, aborted]);
  } finally {
    if (onAbort !== undefined) signal.removeEventListener("abort", onAbort);
  }
}

export async function dispatchStatus<T>(
  bridge: BleBridge,
  command: "get_state" | "get_capabilities" | "check_permissions",
): Promise<Reply<T>> {
  try {
    return await bridge.invoke<Reply<T>>("plugin:ble|" + command);
  } catch (error) {
    throwBridgeError(error);
  }
}

export async function requestPermission<T>(
  bridge: BleBridge,
  role: "scan" | "connect" | "advertise",
): Promise<Reply<T>> {
  try {
    return await bridge.invoke<Reply<T>>(`plugin:ble|request_${role}_permission`);
  } catch (error) {
    throwBridgeError(error);
  }
}

function commandEndpoint(command: Command): string {
  switch (command.kind) {
    case "startScan":
    case "stopScan":
      return "execute_scan";
    case "connect":
    case "disconnect":
    case "discoverServices":
    case "read":
    case "write":
    case "subscribe":
    case "unsubscribe":
      return "execute_connect";
    case "createServer":
    case "closeServer":
    case "setValue":
    case "notify":
      return "execute_server";
    case "startAdvertising":
    case "stopAdvertising":
      return "execute_advertise";
  }
}

let nextOperation = 0;

function allocateOperationId(): string {
  if (typeof globalThis.crypto?.randomUUID === "function") return globalThis.crypto.randomUUID();
  nextOperation += 1;
  return `js-${Date.now()}-${nextOperation}`;
}

function throwIfAborted(signal: AbortSignal | undefined): void {
  if (signal?.aborted) {
    throw new DOMException("The BLE operation was cancelled", "AbortError");
  }
}

function throwBridgeError(error: unknown): never {
  throw isBleError(error) ? new BleError(error) : error;
}

function isBleError(value: unknown): value is BleErrorShape {
  if (typeof value !== "object" || value === null) return false;
  const candidate = value as Partial<BleErrorShape>;
  return typeof candidate.code === "string" && typeof candidate.message === "string";
}

export function expectReply<T>(reply: Reply<T>, kind: string): T {
  if (reply.kind !== kind || reply.payload === undefined) {
    throw new BleError({
      code: "internal",
      message: "native plugin returned " + reply.kind + "; expected " + kind,
    });
  }
  return reply.payload;
}

export function encodeBytes(bytes: Uint8Array): string {
  let binary = "";
  const chunkSize = 0x8000;
  for (let offset = 0; offset < bytes.length; offset += chunkSize) {
    binary += String.fromCharCode(...bytes.subarray(offset, offset + chunkSize));
  }
  return btoa(binary);
}

export function decodeBytes(value: string): Uint8Array {
  let binary: string;
  try {
    binary = atob(value);
  } catch {
    throw new BleError({ code: "internal", message: "native plugin returned invalid base64" });
  }
  return Uint8Array.from(binary, (character) => character.charCodeAt(0));
}
