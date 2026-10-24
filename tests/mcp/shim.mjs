// End-to-end check of outerspace-mcp.exe, the stdio bridge MCP clients launch:
//   cd tests/mcp && node shim.mjs [path\to\outerspace-mcp.exe]
// Defaults to the dev build in target/debug. Read-only: status, tools, window list, a screenshot.
import { join } from "node:path";
import { Client } from "@modelcontextprotocol/sdk/client/index.js";
import { StdioClientTransport } from "@modelcontextprotocol/sdk/client/stdio.js";

const shim = process.argv[2] ?? join(import.meta.dirname, "..", "..", "target", "debug", "outerspace-mcp.exe");
let failures = 0;
const check = (ok, what) => {
  console.log(`${ok ? "ok  " : "FAIL"} ${what}`);
  if (!ok) failures++;
};

const transport = new StdioClientTransport({ command: shim, args: [], stderr: "inherit" });
const client = new Client({ name: "outerspace-shim-test", version: "0.1.0" });
let started = performance.now();
await client.connect(transport);
console.log(`connected through ${shim} in ${Math.round(performance.now() - started)} ms`);
check(client.getServerVersion()?.name === "outerspace", `server identifies as ${client.getServerVersion()?.name}`);
check(Boolean(client.getInstructions()?.includes("virtual monitor")), "server instructions came through");

const { tools } = await client.listTools();
check(tools.length >= 10, `${tools.length} tools listed`);

const text = (result) => JSON.parse(result.content.find((c) => c.type === "text").text);
const status = text(await client.callTool({ name: "status", arguments: {} }));
console.log("status:", JSON.stringify(status));
check(typeof status.enabled === "boolean", "status answered");

if (status.enabled) {
  const windows = text(await client.callTool({ name: "list_windows", arguments: {} }));
  check(Array.isArray(windows), `list_windows answered (${windows.length} windows)`);
  started = performance.now();
  const shot = await client.callTool({ name: "screenshot", arguments: { scale: 0.25, format: "jpeg" } });
  const image = shot.content.find((c) => c.type === "image");
  check(image?.mimeType === "image/jpeg" && image.data.length > 1000, `screenshot came through the bridge (${Math.round(performance.now() - started)} ms, ${Math.round(image.data.length / 1024)} KB base64)`);
}

// Several requests in flight at once must all get their own answers.
const parallel = await Promise.all([1, 2, 3].map(() => client.callTool({ name: "status", arguments: {} })));
check(parallel.every((r) => text(r).version), "3 concurrent requests all answered");

await client.close();
console.log(failures ? `\n${failures} check(s) failed` : "\nall checks passed");
process.exit(failures ? 1 : 0);
