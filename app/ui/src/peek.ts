// Live view of the virtual display. Frames are pulled one at a time from the app's `peek://`
// protocol: the next request goes out only after the previous frame is on screen, so the rate
// adapts to the machine and nothing is captured while the window is minimized.
import { convertFileSrc, invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { getCurrentWindow } from "@tauri-apps/api/window";
import { applyI18n, t } from "./i18n";
import type { Status } from "./types";

const FRAME_INTERVAL_MS = 120; // ~8 fps
const OFF_RETRY_MS = 1000;

const frame = document.querySelector<HTMLImageElement>("#frame")!;
const empty = document.querySelector<HTMLElement>("#empty")!;
const title = document.querySelector<HTMLElement>("#title")!;
const appWindow = getCurrentWindow();
let previousUrl: string | undefined;

document.querySelector("#btn-close")!.addEventListener("click", () => appWindow.close());
document.querySelector("#btn-minimize")!.addEventListener("click", () => appWindow.minimize());
document.querySelector("#btn-maximize")!.addEventListener("click", () => appWindow.toggleMaximize());

function showOff(): void {
  frame.hidden = true;
  empty.hidden = false;
}

async function next(): Promise<void> {
  if (document.hidden) {
    window.setTimeout(next, OFF_RETRY_MS);
    return;
  }
  const width = Math.max(320, Math.round(frame.parentElement!.clientWidth * window.devicePixelRatio));
  try {
    const response = await fetch(`${convertFileSrc("frame", "peek")}?w=${width}&t=${Date.now()}`);
    if (response.status !== 200) {
      showOff();
      window.setTimeout(next, OFF_RETRY_MS);
      return;
    }
    const url = URL.createObjectURL(await response.blob());
    frame.src = url;
    await frame.decode().catch(() => undefined);
    if (previousUrl) URL.revokeObjectURL(previousUrl);
    previousUrl = url;
    frame.hidden = false;
    empty.hidden = true;
    window.setTimeout(next, FRAME_INTERVAL_MS);
  } catch {
    showOff();
    window.setTimeout(next, OFF_RETRY_MS);
  }
}

function renderTitle(status: Status): void {
  const power = status.power;
  title.textContent =
    power.kind === "on" ? t("peek.titleOn", { width: power.display.width, height: power.display.height }) : t("peek.title");
}

applyI18n();
await listen<Status>("status", (event) => renderTitle(event.payload));
renderTitle(await invoke<Status>("status"));
next();
