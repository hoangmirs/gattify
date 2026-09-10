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


/**
 * Subscribes to a plugin event and returns a synchronous unsubscribe function.
 *
 * The returned function works even before `listen` resolves, so a handle that
 * closes immediately after construction still detaches its native listener.
 */
export function subscribe<T>(
  bridge: BleBridge,
  event: string,
  handler: (payload: T) => void,
): () => void {
  let unlisten: Unlisten | undefined;
  let stopped = false;
  void bridge.listen
    ?.<T>(event, (received) => {
      if (!stopped) handler(received.payload);
    })
    .then((received) => {
      if (stopped) received();
      else unlisten = received;
    });
  return () => {
    stopped = true;
    unlisten?.();
  };
}
