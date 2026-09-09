import { BleError, type BleErrorShape } from "./types.js";
import type { BleBridge } from "./bridge.js";

export type Command =
  | { kind: "getState" }
  | { kind: "getCapabilities" }
  | { kind: "checkPermissions" }
  | { kind: "requestPermissions"; payload: { scan: boolean; connect: boolean; advertise: boolean } }
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
    }
  | { kind: "closeOwner" };

export interface Reply<T = unknown> {
  kind: string;
  payload?: T;
}

export async function dispatch<T>(
  bridge: BleBridge,
  command: Command,
  deadlineMillis?: number,
): Promise<Reply<T>> {
  try {
    return await bridge.invoke<Reply<T>>("plugin:ble|execute", {
      request: {
        command,
        deadlineMillis: deadlineMillis ?? null,
      },
    });
  } catch (error) {
    if (isBleError(error)) throw new BleError(error);
    throw error;
  }
}

export async function dispatchStatus<T>(
  bridge: BleBridge,
  command: "get_state" | "get_capabilities" | "check_permissions",
): Promise<Reply<T>> {
  try {
    return await bridge.invoke<Reply<T>>("plugin:ble|" + command);
  } catch (error) {
    if (isBleError(error)) throw new BleError(error);
    throw error;
  }
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
