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

/** One message on the event channel of a webview. */
export interface EventMessage {
  event: string;
  payload: unknown;
}

/** Hands the messages of one event channel to the listeners of each event name. */
export class EventHub {
  readonly #handlers = new Map<string, Set<(event: BridgeEvent<unknown>) => void>>();

  on<T>(event: string, handler: (event: BridgeEvent<T>) => void): Unlisten {
    let handlers = this.#handlers.get(event);
    if (handlers === undefined) {
      handlers = new Set();
      this.#handlers.set(event, handlers);
    }
    const entry = handler as (event: BridgeEvent<unknown>) => void;
    handlers.add(entry);
    return () => {
      handlers.delete(entry);
    };
  }

  dispatch(message: EventMessage): void {
    const handlers = this.#handlers.get(message.event);
    if (handlers === undefined) return;
    for (const handler of [...handlers]) handler({ payload: message.payload });
  }
}

let shared: Promise<BleBridge> | undefined;

/**
 * Returns the bridge of this page. Its events arrive through one Tauri channel
 * that only this webview receives.
 */
export function defaultBridge(): Promise<BleBridge> {
  shared ??= connect().catch((error: unknown) => {
    shared = undefined;
    throw error;
  });
  return shared;
}

async function connect(): Promise<BleBridge> {
  const core = await import("@tauri-apps/api/core");
  const hub = new EventHub();
  const channel = new core.Channel<EventMessage>();
  channel.onmessage = (message) => hub.dispatch(message);
  await core.invoke("plugin:gattify|listen_events", { channel });
  return {
    invoke: core.invoke,
    listen: <T>(event: string, handler: (event: BridgeEvent<T>) => void) =>
      Promise.resolve(hub.on(event, handler)),
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
