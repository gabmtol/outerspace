// Stand-in for the Tauri backend when the UI is opened in a plain browser, for design previews.
// Pick the state with a query string, e.g. index.html?state=on or ?state=error&code=driverMissing.
import type { ClientSnippet, ErrorCode, PowerState, Status } from "./types";

const params = new URLSearchParams(location.search);

// Stand-in for the blurred desktop that the OS acrylic shows behind the window, at the window's real size.
document.documentElement.style.background = "linear-gradient(160deg, #7a8fb8 0%, #c98a7a 55%, #e7b07c 100%)";
Object.assign(document.body.style, { width: "380px", height: "540px", position: "relative" });

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

const shim = "C:\\Users\\you\\AppData\\Local\\Programs\\OuterSpace\\outerspace-mcp.exe";

const snippets: ClientSnippet[] = [
  { id: "claude", name: "Claude Code", monogram: "CC", command: `claude mcp add --transport stdio --scope user outerspace -- "${shim}"`, configPath: "~/.claude.json", config: "{}" },
  { id: "codex", name: "Codex CLI", monogram: "CX", command: `codex mcp add outerspace -- "${shim}"`, configPath: "~/.codex/config.toml", config: "" },
  { id: "opencode", name: "OpenCode", monogram: "OC", command: "", configPath: "~/.config/opencode/opencode.json", config: "{}" },
];

const status: Status = { power: power(), installing: false, port: 7342, version: "0.1.0", autostart: false, transparency: true };

export async function invoke<T>(cmd: string): Promise<T> {
  const replies: Record<string, unknown> = { status, client_snippets: snippets };
  return (replies[cmd] ?? status) as T;
}
