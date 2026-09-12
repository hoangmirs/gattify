import { createBle, type BleSession, type DiscoveredDevice, type ScanHandle } from "tauri-plugin-gattify-api";
import { createEndpoint, type Endpoint, type Peer } from "tauri-plugin-gattify-api/peer";

/** The lab service. The capability file lists it in `gattify:scope`. */
const SERVICE_UUID = "80ff87c3-8e84-4914-aedc-0d6a3ba5534d";
const BULK_BYTES = 4 * 1024;

interface PeerEntry {
  peer: Peer;
  role: "host" | "joiner";
  name: string;
}

const element = <T extends HTMLElement>(id: string): T => {
  const found = document.getElementById(id);
  if (found === null) throw new Error(`missing #${id}`);
  return found as T;
};

const ui = {
  state: element("state"),
  permissions: element("permissions"),
  roles: element("roles"),
  refresh: element<HTMLButtonElement>("refresh"),
  request: element<HTMLButtonElement>("request"),
  localName: element<HTMLInputElement>("local-name"),
  listen: element<HTMLButtonElement>("listen"),
  hostStatus: element("host-status"),
  scan: element<HTMLButtonElement>("scan"),
  devices: element<HTMLUListElement>("devices"),
  peers: element<HTMLUListElement>("peers"),
  messages: element<HTMLOListElement>("messages"),
  composer: element<HTMLFormElement>("composer"),
  text: element<HTMLInputElement>("text"),
  send: element<HTMLButtonElement>("send"),
  sendBulk: element<HTMLButtonElement>("send-bulk"),
  closePeer: element<HTMLButtonElement>("close-peer"),
  log: element<HTMLOListElement>("log"),
};

let ble: BleSession | undefined;
let host: Endpoint | undefined;
let joiner: Endpoint | undefined;
let scan: ScanHandle | undefined;
const devices = new Map<string, DiscoveredDevice>();
const peers = new Map<string, PeerEntry>();
let selected: string | undefined;

function log(message: string, kind: "info" | "ok" | "error" = "info"): void {
  const item = document.createElement("li");
  item.className = kind;
  item.textContent = `${new Date().toLocaleTimeString()} ${message}`;
  ui.log.prepend(item);
  while (ui.log.children.length > 200) ui.log.lastElementChild?.remove();
}

function describe(error: unknown): string {
  if (typeof error === "object" && error !== null && "message" in error) {
    const { code, message, delivery } = error as { code?: string; message: string; delivery?: string };
    return [code, message, delivery && `delivery ${delivery}`].filter(Boolean).join(": ");
  }
  return String(error);
}

/** Runs one user action and logs its failure instead of throwing. */
function action(run: () => Promise<void>): () => void {
  return () => {
    run().catch((error: unknown) => log(describe(error), "error"));
  };
}

async function session(): Promise<BleSession> {
  ble ??= await createBle();
  return ble;
}

async function refresh(): Promise<void> {
  const current = await session();
  ui.state.textContent = await current.getState();
  const permissions = await current.checkPermissions();
  ui.permissions.textContent = `scan ${permissions.scan}, connect ${permissions.connect}, advertise ${permissions.advertise}`;
  const capabilities = await current.getCapabilities();
  ui.roles.textContent = `${capabilities.central.level} / ${capabilities.peripheral.level}`;
}

async function requestPermissions(): Promise<void> {
  const permissions = await (await session()).requestPermissions({
    scan: true,
    connect: true,
    advertise: true,
  });
  log(`permissions: scan ${permissions.scan}, connect ${permissions.connect}, advertise ${permissions.advertise}`);
  await refresh();
}

async function toggleListen(): Promise<void> {
  if (host !== undefined) {
    const closing = host;
    host = undefined;
    await closing.close();
    ui.listen.textContent = "Listen";
    ui.hostStatus.textContent = "Not listening.";
    log("stopped listening");
    return;
  }
  const localName = ui.localName.value.trim() || "gattify lab";
  host = await createEndpoint({ serviceUuid: SERVICE_UUID, localName, listen: true });
  host.onPeer((peer) => addPeer(peer, "host", "central"));
  ui.listen.textContent = "Stop";
  ui.hostStatus.textContent = `Advertising as "${localName}".`;
  log(`listening as ${localName}`, "ok");
}

function deviceName(device: DiscoveredDevice): string {
  return device.name ?? device.id;
}

function renderDevices(): void {
  ui.devices.replaceChildren(
    ...[...devices.values()].map((device) => {
      const item = document.createElement("li");
      const name = document.createElement("span");
      name.textContent = deviceName(device);
      const meta = document.createElement("span");
      meta.className = "meta";
      meta.textContent = device.rssi === null ? device.id : `${device.rssi} dBm`;
      item.append(name, meta);
      item.addEventListener("click", action(() => dial(device)));
      return item;
    }),
  );
}

async function toggleScan(): Promise<void> {
  if (scan !== undefined) {
    const stopping = scan;
    scan = undefined;
    ui.scan.textContent = "Scan";
    await stopping.stop();
    log("scan stopped");
    return;
  }
  devices.clear();
  renderDevices();
  scan = await (await session()).scan({ serviceUuids: [SERVICE_UUID] });
  scan.onUpdate((device) => {
    if (!devices.has(device.id)) log(`found ${deviceName(device)} (${device.id})`);
    devices.set(device.id, device);
    renderDevices();
  });
  ui.scan.textContent = "Stop scan";
  log("scanning for the lab service", "ok");
}

async function dial(device: DiscoveredDevice): Promise<void> {
  joiner ??= await createEndpoint({ serviceUuid: SERVICE_UUID });
  log(`dialing ${deviceName(device)}`);
  const started = performance.now();
  const peer = await joiner.dial(device.id);
  log(`joined ${deviceName(device)} in ${Math.round(performance.now() - started)} ms`, "ok");
  addPeer(peer, "joiner", deviceName(device));
}

function addPeer(peer: Peer, role: PeerEntry["role"], name: string): void {
  peers.set(peer.id, { peer, role, name });
  selected = peer.id;
  log(`peer ${peer.id} ready as ${role}`, "ok");
  peer.onMessage((bytes) => {
    const text = bytes.length <= 512 ? new TextDecoder().decode(bytes) : `${bytes.length} bytes`;
    appendMessage("in", text, `from ${name}`);
  });
  peer.onClose((reason) => {
    peers.delete(peer.id);
    if (selected === peer.id) selected = peers.keys().next().value;
    log(`peer ${peer.id} closed: ${reason}`, reason === "local" ? "info" : "error");
    renderPeers();
  });
  renderPeers();
}

function renderPeers(): void {
  ui.peers.replaceChildren(
    ...[...peers.entries()].map(([id, entry]) => {
      const item = document.createElement("li");
      item.setAttribute("aria-selected", String(id === selected));
      const name = document.createElement("span");
      name.textContent = entry.name;
      const meta = document.createElement("span");
      meta.className = "meta";
      meta.textContent = `${entry.role} · ${id}`;
      item.append(name, meta);
      item.addEventListener("click", () => {
        selected = id;
        renderPeers();
      });
      return item;
    }),
  );
  const ready = selected !== undefined;
  ui.send.disabled = !ready;
  ui.sendBulk.disabled = !ready;
  ui.closePeer.disabled = !ready;
}

function appendMessage(direction: "in" | "out", text: string, meta: string): void {
  const item = document.createElement("li");
  item.className = direction;
  const body = document.createElement("div");
  body.textContent = text;
  const detail = document.createElement("div");
  detail.className = "meta";
  detail.textContent = meta;
  item.append(body, detail);
  ui.messages.append(item);
  item.scrollIntoView({ block: "nearest" });
}

async function send(bytes: Uint8Array, shown: string): Promise<void> {
  const entry = selected === undefined ? undefined : peers.get(selected);
  if (entry === undefined) return;
  const started = performance.now();
  const receipt = await entry.peer.send(bytes);
  const elapsed = Math.round(performance.now() - started);
  appendMessage("out", shown, `#${receipt.messageId} ${receipt.delivery} in ${elapsed} ms`);
  log(`message ${receipt.messageId}: ${bytes.length} bytes acknowledged in ${elapsed} ms`, "ok");
}

ui.refresh.addEventListener("click", action(refresh));
ui.request.addEventListener("click", action(requestPermissions));
ui.listen.addEventListener("click", action(toggleListen));
ui.scan.addEventListener("click", action(toggleScan));
ui.composer.addEventListener("submit", (event) => {
  event.preventDefault();
  const text = ui.text.value;
  if (text === "") return;
  ui.text.value = "";
  action(() => send(new TextEncoder().encode(text), text))();
});
ui.sendBulk.addEventListener(
  "click",
  action(() => {
    const bytes = new Uint8Array(BULK_BYTES).map((_, index) => index % 251);
    return send(bytes, `${BULK_BYTES} bytes`);
  }),
);
ui.closePeer.addEventListener(
  "click",
  action(async () => {
    const entry = selected === undefined ? undefined : peers.get(selected);
    await entry?.peer.close();
  }),
);

renderPeers();
action(refresh)();
