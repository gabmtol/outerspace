import { invoke as tauriInvoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { getCurrentWindow } from "@tauri-apps/api/window";
import { applyI18n, t } from "./i18n";
import type { ClientSnippet, DisplayWindow, ErrorCode, Status } from "./types";

type Invoke = <T>(cmd: string, args?: Record<string, unknown>) => Promise<T>;

const inTauri = "__TAURI_INTERNALS__" in window;
// Outside Tauri (plain browser, used for design previews) commands are served by a local mock.
const invoke: Invoke = inTauri ? tauriInvoke : (await import("./preview")).invoke;

const $ = <T extends HTMLElement>(selector: string) => document.querySelector<T>(selector)!;

const powerCard = $("#power");
const powerSwitch = $<HTMLButtonElement>("#power-switch");
const statusText = $("#status-text");
const statusDetail = $("#status-detail");
const detailLine = $("#detail-line");
const peekButton = $<HTMLButtonElement>("#btn-peek");
const dock = $("#dock");
const resolutionRow = $("#resolution-row");
const resolution = $<HTMLSelectElement>("#resolution");
const scale = $<HTMLSelectElement>("#scale");
const powerActions = $("#power-actions");
const installButton = $<HTMLButtonElement>("#btn-install");
const restartDriverButton = $<HTMLButtonElement>("#btn-restart-driver");
const retryButton = $<HTMLButtonElement>("#btn-retry");
const autostart = $<HTMLInputElement>("#autostart");
const manual = $("#manual");
const manualButton = $<HTMLButtonElement>("#btn-manual");
const toast = $("#toast");

let current: Status | undefined;
/** Set while a resolution change is on its way, so status refreshes don't snap the menu back. */
let changingResolution = false;
let changingScale = false;
/** What the dock shows, so refreshes only rebuild it when a window came, went or changed. */
let dockKey = "";

const modeKey = (mode: { width: number; height: number }) => `${mode.width}x${mode.height}`;

const ERROR_TEXT: Record<ErrorCode, [title: string, detail?: string]> = {
  driverMissing: ["errors.driverMissing", "errors.driverMissingDetail"],
  driverDisabled: ["errors.driverDisabled", "errors.driverDisabledDetail"],
  restartRequired: ["errors.restartRequired", "errors.restartRequiredDetail"],
  driverFailed: ["errors.driverFailed", "errors.driverFailedDetail"],
  connectFailed: ["errors.connectFailed"],
  installFailed: ["errors.installFailed"],
};

function render(status: Status): void {
  current = status;
  const { power } = status;
  powerCard.dataset.state = power.kind;
  document.body.classList.toggle("solid", !status.transparency);

  const busy = power.kind === "starting" || power.kind === "stopping" || status.installing;
  powerSwitch.disabled = busy;
  powerSwitch.setAttribute("aria-checked", String(power.kind === "on" || power.kind === "starting"));

  let detail = "";
  let actions: HTMLButtonElement[] = [];
  switch (power.kind) {
    case "off":
      statusText.textContent = t("power.off");
      detail = t("power.offDetail");
      break;
    case "starting":
      statusText.textContent = t("power.starting");
      break;
    case "stopping":
      statusText.textContent = t("power.stopping");
      break;
    case "on": {
      statusText.textContent = t("power.on", { ...power.display });
      const count = status.windows.length;
      detail = count === 0 ? t("power.onDetail") : count === 1 ? t("power.oneWindow") : t("power.windows", { count });
      break;
    }
    case "error": {
      const [title, more] = ERROR_TEXT[power.code];
      statusText.textContent = t(title);
      detail = more ? t(more) : (power.message ?? "");
      actions =
        power.code === "driverMissing" || power.code === "installFailed"
          ? [installButton]
          : power.code === "driverFailed"
            ? [restartDriverButton]
            : [retryButton];
      break;
    }
  }
  statusDetail.textContent = detail;
  peekButton.hidden = power.kind !== "on";
  detailLine.hidden = detail === "" && peekButton.hidden;
  renderDock(power.kind === "on" ? status.windows : []);

  resolutionRow.hidden = power.kind === "error";
  if (resolution.options.length !== status.modes.length) {
    resolution.replaceChildren(
      ...status.modes.map((mode) => new Option(t("display.mode", { ...mode }), modeKey(mode))),
    );
  }
  if (!changingResolution) resolution.value = modeKey(status.mode);
  resolution.disabled = busy || changingResolution;

  // Windows only says which scales a display allows while it is on.
  const scales = power.kind === "on" ? status.scales : null;
  scale.hidden = !scales;
  if (scales) {
    const choices = scales.available.join();
    if (scale.dataset.choices !== choices) {
      scale.dataset.choices = choices;
      scale.replaceChildren(
        ...scales.available.map((percent) => new Option(t("display.scaleOption", { percent }), String(percent))),
      );
    }
    if (!changingScale) scale.value = String(scales.current);
    scale.title = t("display.scaleTitle", { recommended: scales.recommended });
    scale.setAttribute("aria-label", scale.title);
  }
  scale.disabled = busy || changingScale;

  installButton.disabled = status.installing;
  installButton.textContent = t(status.installing ? "actions.installing" : "actions.installDriver");
  restartDriverButton.disabled = status.installing;
  restartDriverButton.textContent = t(status.installing ? "actions.restartingDriver" : "actions.restartDriver");
  for (const button of [installButton, restartDriverButton, retryButton]) button.hidden = !actions.includes(button);
  powerActions.hidden = actions.length === 0;

  $("#server-caption").textContent = t("server.caption", { port: status.port });
  $("#version").textContent = `v${status.version}`;
  autostart.checked = status.autostart;
}

/** The windows on the virtual display, as icons: clicking one moves it to the user's screen. */
function renderDock(windows: DisplayWindow[]): void {
  const key = JSON.stringify(windows.map((win) => [win.hwnd, win.title, win.icon !== null]));
  if (key === dockKey) return;
  dockKey = key;
  dock.hidden = windows.length === 0;
  dock.replaceChildren(
    ...windows.map((win) => {
      const name = win.title || win.process;
      const button = document.createElement("button");
      button.className = "dock-item";
      button.title = t("dock.bringBack", { name });
      button.setAttribute("aria-label", button.title);
      if (win.icon) {
        const icon = document.createElement("img");
        icon.src = win.icon;
        icon.alt = "";
        button.append(icon);
      } else {
        button.textContent = (win.process || win.title || "?").charAt(0).toUpperCase();
      }
      button.addEventListener("click", async () => {
        button.disabled = true;
        const status = await run<Status>("bring_back", { hwnd: win.hwnd });
        if (!status) {
          button.disabled = false;
          return;
        }
        const short = name.length > 40 ? `${name.slice(0, 39)}…` : name;
        showToast(t("dock.broughtBack", { name: short }));
        render(status);
      });
      return button;
    }),
  );
}

function showToast(message: string): void {
  toast.textContent = message;
  toast.classList.add("show");
  window.setTimeout(() => toast.classList.remove("show"), 1600);
}

async function copy(text: string, button: HTMLButtonElement): Promise<void> {
  await navigator.clipboard.writeText(text);
  button.textContent = t("clients.copied");
  window.setTimeout(() => (button.textContent = t("clients.copy")), 1400);
}

function renderClients(snippets: ClientSnippet[]): void {
  const template = $<HTMLTemplateElement>("#client-row");
  const list = $("#clients");
  list.replaceChildren();
  for (const snippet of snippets) {
    const row = template.content.firstElementChild!.cloneNode(true) as HTMLElement;
    row.dataset.client = snippet.id;
    row.querySelector(".client-icon")!.textContent = snippet.monogram;
    row.querySelector(".client-name")!.textContent = snippet.name;
    const add = row.querySelector<HTMLButtonElement>(".add")!;
    const vars = { name: snippet.name };
    if (snippet.state === "configured") {
      add.textContent = t("clients.configured");
      add.classList.replace("primary", "done");
      add.title = t("clients.configuredTitle", vars);
    } else {
      add.textContent = t(snippet.state === "outdated" ? "clients.update" : "clients.add");
      add.title = t(snippet.state === "outdated" ? "clients.updateTitle" : "clients.addTitle", vars);
      add.addEventListener("click", async () => {
        add.disabled = true;
        add.textContent = t("clients.adding");
        const result = await run<[string, ClientSnippet[]]>("add_client", { id: snippet.id });
        if (result) {
          showToast(t("clients.added", { message: result[0], name: snippet.name }));
          renderClients(result[1]);
        } else {
          renderClients(snippets);
        }
      });
    }
    const copyButton = row.querySelector<HTMLButtonElement>(".copy")!;
    copyButton.textContent = t("clients.copy");
    copyButton.addEventListener("click", () => copy(snippet.command, copyButton));
    list.append(row);
  }

  manual.replaceChildren(
    ...snippets.map((snippet) => {
      const block = document.createElement("div");
      block.className = "snippet";
      const head = document.createElement("div");
      head.className = "snippet-head";
      const label = document.createElement("span");
      label.textContent = `${snippet.name} · ${snippet.configPath}`;
      const copyButton = document.createElement("button");
      copyButton.className = "link";
      copyButton.textContent = t("clients.copy");
      copyButton.addEventListener("click", () => copy(snippet.config, copyButton));
      head.append(label, copyButton);
      const pre = document.createElement("pre");
      pre.textContent = snippet.config;
      block.append(head, pre);
      return block;
    }),
  );
}

async function run<T>(cmd: string, args?: Record<string, unknown>): Promise<T | undefined> {
  try {
    return await invoke<T>(cmd, args);
  } catch (error) {
    showToast(String(error));
    return undefined;
  }
}

powerSwitch.addEventListener("click", async () => {
  if (!current) return;
  const turnOn = !(current.power.kind === "on");
  const status = await run<Status>("set_power", { on: turnOn });
  if (status) render(status);
});

resolution.addEventListener("change", async () => {
  const [width, height] = resolution.value.split("x").map(Number);
  changingResolution = true;
  resolution.disabled = true;
  const status = await run<Status>("set_resolution", { width, height });
  changingResolution = false;
  if (status?.power.kind === "on") showToast(t("display.changed", { width, height }));
  // On failure the toast already says why; the menu goes back to what is really set.
  render(status ?? (await invoke<Status>("status")));
});

scale.addEventListener("change", async () => {
  const percent = Number(scale.value);
  changingScale = true;
  scale.disabled = true;
  const status = await run<Status>("set_scale", { percent });
  changingScale = false;
  if (status) showToast(t("display.scaleChanged", { percent }));
  render(status ?? (await invoke<Status>("status")));
});

installButton.addEventListener("click", async () => {
  const status = await run<Status>("install_driver");
  if (status) render(status);
});

restartDriverButton.addEventListener("click", async () => {
  const status = await run<Status>("restart_driver");
  if (status) render(status);
});

retryButton.addEventListener("click", async () => {
  const status = await run<Status>("set_power", { on: true });
  if (status) render(status);
});

autostart.addEventListener("change", async () => {
  const status = await run<Status>("set_autostart", { enabled: autostart.checked });
  if (status) render(status);
});

manualButton.addEventListener("click", () => {
  manual.hidden = !manual.hidden;
  manualButton.textContent = t(manual.hidden ? "server.showManual" : "server.hideManual");
});

for (const id of ["#btn-logs", "#btn-log"]) $(id).addEventListener("click", () => run("open_logs"));
peekButton.addEventListener("click", () => run("open_peek"));

if (inTauri) {
  const appWindow = getCurrentWindow();
  $("#btn-minimize").addEventListener("click", () => appWindow.minimize());
  // Closing frees the WebView; the app stays in the tray and reopens the window on demand.
  $("#btn-close").addEventListener("click", () => appWindow.close());
  await listen<Status>("status", (event) => render(event.payload));
}

applyI18n();
const [status, snippets] = await Promise.all([invoke<Status>("status"), invoke<ClientSnippet[]>("client_snippets")]);
render(status);
renderClients(snippets);

// Windows come and go on the virtual display without a power change; refresh while visible.
window.setInterval(async () => {
  if (!document.hidden) render(await invoke<Status>("status"));
}, 3000);
