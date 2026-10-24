import { invoke as tauriInvoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { getCurrentWindow } from "@tauri-apps/api/window";
import { applyI18n, t } from "./i18n";
import type { ClientSnippet, ErrorCode, Status } from "./types";

type Invoke = <T>(cmd: string, args?: Record<string, unknown>) => Promise<T>;

const inTauri = "__TAURI_INTERNALS__" in window;
// Outside Tauri (plain browser, used for design previews) commands are served by a local mock.
const invoke: Invoke = inTauri ? tauriInvoke : (await import("./preview")).invoke;

const $ = <T extends HTMLElement>(selector: string) => document.querySelector<T>(selector)!;

const powerCard = $("#power");
const powerSwitch = $<HTMLButtonElement>("#power-switch");
const statusText = $("#status-text");
const statusDetail = $("#status-detail");
const powerActions = $("#power-actions");
const installButton = $<HTMLButtonElement>("#btn-install");
const retryButton = $<HTMLButtonElement>("#btn-retry");
const autostart = $<HTMLInputElement>("#autostart");
const manual = $("#manual");
const manualButton = $<HTMLButtonElement>("#btn-manual");
const toast = $("#toast");

let current: Status | undefined;

const ERROR_TEXT: Record<ErrorCode, [title: string, detail?: string]> = {
  driverMissing: ["errors.driverMissing", "errors.driverMissingDetail"],
  driverDisabled: ["errors.driverDisabled", "errors.driverDisabledDetail"],
  restartRequired: ["errors.restartRequired", "errors.restartRequiredDetail"],
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
    case "on":
      statusText.textContent = t("power.on", { ...power.display });
      detail = t("power.onDetail");
      break;
    case "error": {
      const [title, more] = ERROR_TEXT[power.code];
      statusText.textContent = t(title);
      detail = more ? t(more) : (power.message ?? "");
      actions = power.code === "driverMissing" || power.code === "installFailed" ? [installButton] : [retryButton];
      break;
    }
  }
  statusDetail.textContent = detail;
  statusDetail.hidden = detail === "";

  installButton.disabled = status.installing;
  installButton.textContent = t(status.installing ? "actions.installing" : "actions.installDriver");
  for (const button of [installButton, retryButton]) button.hidden = !actions.includes(button);
  powerActions.hidden = actions.length === 0;

  $("#server-caption").textContent = t("server.caption", { port: status.port });
  $("#version").textContent = `v${status.version}`;
  autostart.checked = status.autostart;
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
    add.textContent = t("clients.add");
    add.disabled = true;
    add.title = t("clients.addSoon");
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

installButton.addEventListener("click", async () => {
  const status = await run<Status>("install_driver");
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

if (inTauri) {
  const appWindow = getCurrentWindow();
  $("#btn-minimize").addEventListener("click", () => appWindow.minimize());
  $("#btn-close").addEventListener("click", () => appWindow.hide());
  await listen<Status>("status", (event) => render(event.payload));
}

applyI18n();
const [status, snippets] = await Promise.all([invoke<Status>("status"), invoke<ClientSnippet[]>("client_snippets")]);
render(status);
renderClients(snippets);
