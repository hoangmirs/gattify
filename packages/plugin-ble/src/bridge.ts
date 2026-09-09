export interface Unlisten {
  (): void;
}

export interface BridgeEvent<T> {
  payload: T;
}

export interface BleBridge {
  invoke<T>(command: string, args?: Record<string, unknown>): Promise<T>;
  listen?<T>(
    event: string,
    handler: (event: BridgeEvent<T>) => void,
  ): Promise<Unlisten>;
}

export async function defaultBridge(): Promise<BleBridge> {
  const core = await import("@tauri-apps/api/core");
  const event = await import("@tauri-apps/api/event");
  return {
    invoke: core.invoke,
    listen: event.listen,
  };
}

