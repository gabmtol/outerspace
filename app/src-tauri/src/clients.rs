//! Setup snippets for the MCP clients (docs/PLANO.md §7). The one-click "Add" lands in phase F4;
//! for now the UI copies these.

use serde::Serialize;
use serde_json::json;

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ClientSnippet {
    id: &'static str,
    name: &'static str,
    monogram: &'static str,
    /// What "Copy" puts on the clipboard: a CLI command, or the config itself when there is no CLI.
    command: String,
    config_path: &'static str,
    config: String,
}

pub fn snippets() -> Vec<ClientSnippet> {
    let shim = shim_path();
    let opencode = pretty(&json!({
        "$schema": "https://opencode.ai/config.json",
        "mcp": { "outerspace": { "type": "local", "command": [shim], "enabled": true, "timeout": 30000 } }
    }));
    vec![
        ClientSnippet {
            id: "claude",
            name: "Claude Code",
            monogram: "CC",
            command: format!(r#"claude mcp add --transport stdio --scope user outerspace -- "{shim}""#),
            config_path: "~/.claude.json",
            config: pretty(
                &json!({ "mcpServers": { "outerspace": { "type": "stdio", "command": shim, "args": [] } } }),
            ),
        },
        ClientSnippet {
            id: "codex",
            name: "Codex CLI",
            monogram: "CX",
            command: format!(r#"codex mcp add outerspace -- "{shim}""#),
            config_path: "~/.codex/config.toml",
            // Literal TOML string: backslashes in the Windows path need no escaping. The longer
            // timeout covers the shim starting the tray app first.
            config: format!("[mcp_servers.outerspace]\ncommand = '{shim}'\nargs = []\nstartup_timeout_sec = 20\n"),
        },
        ClientSnippet {
            id: "opencode",
            name: "OpenCode",
            monogram: "OC",
            command: opencode.clone(),
            config_path: "~/.config/opencode/opencode.json",
            config: opencode,
        },
    ]
}

/// The stdio shim ships next to OuterSpace.exe.
fn shim_path() -> String {
    std::env::current_exe()
        .ok()
        .and_then(|exe| exe.parent().map(|dir| dir.join("outerspace-mcp.exe").display().to_string()))
        .unwrap_or_else(|| "outerspace-mcp.exe".into())
}

fn pretty(value: &serde_json::Value) -> String {
    serde_json::to_string_pretty(value).expect("JSON values always serialize")
}
