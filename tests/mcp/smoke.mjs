// End-to-end check of the running OuterSpace app over MCP, the way agents use it.
// Needs OuterSpace running with the virtual display switched on:
//   cd tests/mcp && npm install && npm run smoke
import { readFileSync, writeFileSync, mkdirSync } from "node:fs";
import { join } from "node:path";
import { Client } from "@modelcontextprotocol/sdk/client/index.js";
import { StreamableHTTPClientTransport } from "@modelcontextprotocol/sdk/client/streamableHttp.js";

const url = "http://127.0.0.1:7342/mcp";
const { token } = JSON.parse(readFileSync(join(process.env.LOCALAPPDATA, "OuterSpace", "config.json"), "utf8"));
const out = join(import.meta.dirname, "..", "..", "target");
mkdirSync(out, { recursive: true });

let failures = 0;
function check(ok, what) {
  console.log(`${ok ? "ok  " : "FAIL"} ${what}`);
  if (!ok) failures++;
}

// Guards: no token, and a browser Origin, must both be refused.
const ping = { jsonrpc: "2.0", id: 1, method: "ping" };
const post = (headers) =>
  fetch(url, {
    method: "POST",
    headers: { "content-type": "application/json", accept: "application/json, text/event-stream", ...headers },
    body: JSON.stringify(ping),
  });
check((await post({})).status === 401, "request without token is refused (401)");
check(
  (await post({ authorization: `Bearer ${token}`, origin: "https://evil.example" })).status === 403,
  "request from a browser origin is refused (403)",
);

const client = new Client({ name: "outerspace-smoke", version: "0.1.0" });
await client.connect(
  new StreamableHTTPClientTransport(new URL(url), { requestInit: { headers: { authorization: `Bearer ${token}` } } }),
);
// Tool errors come back as plain text; everything else is JSON.
const text = (result) => {
  const body = result.content.find((c) => c.type === "text").text;
  return result.isError ? { error: body } : JSON.parse(body);
};
const call = async (name, args = {}) => {
  const started = performance.now();
  const result = await client.callTool({ name, arguments: args });
  return { result, ms: Math.round(performance.now() - started) };
};

const { tools } = await client.listTools();
console.log("tools:", tools.map((t) => t.name).join(", "));
check(
  ["status", "launch", "list_windows", "screenshot", "move_window", "close_window"].every((n) =>
    tools.some((t) => t.name === n),
  ),
  "all phase-2 tools are listed",
);

const status = text((await call("status")).result);
console.log("status:", JSON.stringify(status));
if (!status.enabled) {
  console.log("\nThe virtual display is off: switch it on in OuterSpace and run again.");
  const { result } = await call("list_windows");
  check(result.isError === true, "tools other than status refuse to run while off");
  process.exit(failures ? 1 : 0);
}

const launched = await call("launch", { program: "notepad.exe" });
const launch = text(launched.result);
console.log(`launch (${launched.ms} ms):`, JSON.stringify(launch));
check(!launched.result.isError && launch.window?.on_virtual_display, "notepad opened on the virtual display");
check(launch.window && !launch.window.focused && !launch.focus_repaired, "notepad did not take the focus");
const hwnd = launch.window?.hwnd;

const listed = text((await call("list_windows")).result);
check(listed.some((w) => w.hwnd === hwnd), "list_windows shows it");

const shot = await call("screenshot");
const image = shot.result.content.find((c) => c.type === "image");
const info = text(shot.result);
writeFileSync(join(out, "smoke-screenshot.png"), Buffer.from(image.data, "base64"));
console.log(`screenshot (${shot.ms} ms):`, JSON.stringify(info));
check(image?.mimeType === "image/png" && info.width === 1280 && info.height === 720, "screenshot is a 1280x720 PNG");

const moved = text((await call("move_window", { hwnd, preset: "left" })).result);
check(moved.rect?.x === 0 && moved.rect?.w === 960, `move_window left -> ${JSON.stringify(moved.rect)}`);

const escape = await call("move_window", { hwnd, x: -5000, y: 0 });
check(escape.result.isError === true, "move_window refuses to put a window outside the virtual display");

const closed = text((await call("close_window", { hwnd })).result);
check(closed.closed === true, "close_window closed it");

// `--apps`: GPU-heavy apps that restore their own window positions (plan §10, F2 acceptance).
if (process.argv.includes("--apps")) {
  const apps = [
    { program: "msedge", args: ["--new-window", "about:blank"] },
    { program: "firefox", args: ["-new-window", "about:blank"] },
    { program: "Code.exe", args: ["--new-window"] },
  ];
  const focusedWindow = async () => text((await call("list_windows", { all: true })).result).find((w) => w.focused);
  const focused = async () => {
    const w = await focusedWindow();
    console.log(`     foreground: ${w ? `${w.hwnd} ${w.process} "${w.title}"` : "(none of the listed windows)"}`);
    return w?.hwnd;
  };
  for (const app of apps) {
    const userWindow = await focused();
    const { result, ms } = await call("launch", { ...app, wait_ms: 20000 });
    const launched = result.isError ? null : text(result);
    const window = launched?.window;
    console.log(`launch ${app.program} (${ms} ms):`, window ? `${window.process} "${window.title}" at ${JSON.stringify(window.rect)}` : JSON.stringify(launched ?? result.content));
    check(window?.on_virtual_display === true, `${app.program} opened on the virtual display`);
    if (window) {
      // Give apps that restore their saved position (Firefox) time to try; the keeper must win.
      await new Promise((resolve) => setTimeout(resolve, 3000));
      const listed = text((await call("list_windows")).result);
      check(listed.some((w) => w.hwnd === window.hwnd), `${app.program} is still on the virtual display after 3 s`);
      // The user may legitimately switch windows meanwhile; what must never happen is the agent's
      // window holding the foreground.
      const after = await focused();
      check(after !== window.hwnd, `${app.program} does not hold the foreground`);
      if (userWindow && after !== userWindow) console.log("     (the user's foreground changed — by the user?)");
      const shot = await call("screenshot", { hwnd: window.hwnd, format: "jpeg", scale: 0.5 });
      writeFileSync(join(out, `smoke-${app.program}.jpg`), Buffer.from(shot.result.content[0].data, "base64"));
      check(text((await call("close_window", { hwnd: window.hwnd })).result).closed === true, `${app.program} closed`);
    }
  }
}

await client.close();
console.log(failures ? `\n${failures} check(s) failed` : "\nall checks passed");
process.exit(failures ? 1 : 0);
