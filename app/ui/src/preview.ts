// Stand-in for the Tauri backend when the UI is opened in a plain browser, for design previews.
// Pick the state with a query string, e.g. index.html?state=on or ?state=error&code=driverMissing.
import type { ClientSnippet, ErrorCode, PowerState, Status } from "./types";

const params = new URLSearchParams(location.search);

// Stand-in for the blurred desktop that the OS acrylic shows behind the window, at the window's real size.
document.documentElement.style.background = "linear-gradient(160deg, #7a8fb8 0%, #c98a7a 55%, #e7b07c 100%)";
Object.assign(document.body.style, { width: "380px", height: "460px", position: "relative" });

function power(): PowerState {
  switch (params.get("state")) {
    case "on":
      return { kind: "on", display: { width: 1920, height: 1080, hz: 60, x: 2560, y: -1079 } };
    case "starting":
      return { kind: "starting" };
    case "error":
      return { kind: "error", code: (params.get("code") as ErrorCode) ?? "driverMissing" };
    default:
      return { kind: "off" };
  }
}

const shim = "C:\\Users\\you\\AppData\\Local\\OuterSpace\\outerspace-mcp.exe";

const snippets: ClientSnippet[] = [
  { id: "claude", name: "Claude Code", monogram: "CC", state: "configured", command: `claude mcp add --transport stdio --scope user outerspace -- "${shim}"`, configPath: "~/.claude.json", config: "{}" },
  { id: "codex", name: "Codex CLI", monogram: "CX", state: "notConfigured", command: `codex mcp add outerspace -- "${shim}"`, configPath: "~/.codex/config.toml", config: "" },
  { id: "opencode", name: "OpenCode", monogram: "OC", state: "notConfigured", command: "", configPath: "~/.config/opencode/opencode.json", config: "{}" },
];

const status: Status = {
  power: power(),
  installing: false,
  port: 7342,
  version: "0.1.2",
  autostart: false,
  transparency: true,
  windows:
    params.get("state") === "on"
      ? [
          { hwnd: 1, title: "Untitled - Notepad", process: "Notepad.exe", icon: null },
          { hwnd: 2, title: "Example Domain - Microsoft Edge", process: "msedge.exe", icon: null },
        ]
      : [],
  mode: { width: 1920, height: 1080, hz: 60 },
  modes: [
    [1280, 720], [1280, 800], [1366, 768], [1440, 900], [1600, 900], [1680, 1050], [1920, 1080],
    [1920, 1200], [2560, 1080], [2560, 1440], [2560, 1600], [3440, 1440], [3840, 2160],
  ].map(([width, height]) => ({ width, height, hz: 60 })),
};

export async function invoke<T>(cmd: string, args?: Record<string, unknown>): Promise<T> {
  if (cmd === "set_resolution") {
    status.mode = { width: Number(args?.width), height: Number(args?.height), hz: 60 };
    if (status.power.kind === "on") status.power.display = { ...status.power.display, ...status.mode };
  }
  if (cmd === "bring_back") status.windows = status.windows.filter((win) => win.hwnd !== args?.hwnd);
  const replies: Record<string, unknown> = { status, client_snippets: snippets };
  return (replies[cmd] ?? status) as T;
}
