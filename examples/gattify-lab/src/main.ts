import { createBle } from "tauri-plugin-gattify-api";

const status = document.querySelector<HTMLPreElement>("#status")!;

async function show(): Promise<void> {
  const ble = await createBle();
  const report = {
    state: await ble.getState(),
    capabilities: await ble.getCapabilities(),
  };
  status.textContent = JSON.stringify(report, null, 2);
}

show().catch((error: unknown) => {
  status.textContent = error instanceof Error ? error.message : JSON.stringify(error);
});
