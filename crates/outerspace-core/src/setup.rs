//! Connecting MCP clients to OuterSpace (docs/PLANO.md §7). Each client gets the stdio shim
//! (`outerspace-mcp.exe`, shipped next to OuterSpace.exe) as a local server named "outerspace":
//! - Claude Code through its own CLI (`claude mcp add --scope user`), which owns `~/.claude.json`;
//! - Codex by editing `~/.codex/config.toml` with toml_edit (comments survive), adding a longer
//!   startup timeout because the shim may have to start OuterSpace first;
//! - OpenCode by editing its `opencode.jsonc`/`opencode.json` as a JSONC tree (comments survive) —
//!   its `mcp add` command is interactive for local servers.
//!
//! Every config file is copied to `<name>.outerspace.bak` before it is changed.

use std::os::windows::process::CommandExt;
use std::path::{Path, PathBuf};
use std::process::Command;

use jsonc_parser::ParseOptions;
use jsonc_parser::cst::{CstInputValue, CstRootNode};
use serde::Serialize;

const SERVER: &str = "outerspace";
const CREATE_NO_WINDOW: u32 = 0x0800_0000;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Client {
    Claude,
    Codex,
    OpenCode,
}

impl Client {
    pub const ALL: [Client; 3] = [Client::Claude, Client::Codex, Client::OpenCode];

    pub fn parse(id: &str) -> Option<Client> {
        Client::ALL.into_iter().find(|c| c.id() == id)
    }

    pub fn id(self) -> &'static str {
        match self {
            Client::Claude => "claude",
            Client::Codex => "codex",
            Client::OpenCode => "opencode",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum State {
    NotConfigured,
    Configured,
    /// Configured, but pointing at another copy of the shim (e.g. a dev build or an old install).
    Outdated,
}

/// What the UI shows for one client.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ClientInfo {
    id: &'static str,
    name: &'static str,
    monogram: &'static str,
    state: State,
    /// Where its config lives, for the manual-setup panel.
    config_path: String,
    /// What "Copy" puts on the clipboard: a CLI command, or the config itself when there is no CLI.
    command: String,
    config: String,
}

#[derive(Debug, thiserror::Error)]
#[error("{0}")]
pub struct SetupError(String);

/// The shim ships next to OuterSpace.exe (in dev builds: next to it in `target/debug`).
pub fn shim_path() -> PathBuf {
    std::env::current_exe()
        .ok()
        .and_then(|exe| exe.parent().map(|dir| dir.join("outerspace-mcp.exe")))
        .unwrap_or_else(|| PathBuf::from("outerspace-mcp.exe"))
}

pub fn describe(client: Client, shim: &Path) -> ClientInfo {
    let shim_text = shim.display().to_string();
    let state = state(client, shim);
    match client {
        Client::Claude => ClientInfo {
            id: client.id(),
            name: "Claude Code",
            monogram: "CC",
            state,
            config_path: home().join(".claude.json").display().to_string(),
            command: format!(r#"claude mcp add --transport stdio --scope user {SERVER} -- "{shim_text}""#),
            config: pretty(
                &serde_json::json!({ "mcpServers": { SERVER: { "type": "stdio", "command": shim_text, "args": [] } } }),
            ),
        },
        Client::Codex => ClientInfo {
            id: client.id(),
            name: "Codex CLI",
            monogram: "CX",
            state,
            config_path: codex_config().display().to_string(),
            command: format!(r#"codex mcp add {SERVER} -- "{shim_text}""#),
            config: format!("[mcp_servers.{SERVER}]\ncommand = '{shim_text}'\nargs = []\nstartup_timeout_sec = 20\n"),
        },
        Client::OpenCode => {
            let config = pretty(&serde_json::json!({
                "$schema": "https://opencode.ai/config.json",
                "mcp": { SERVER: { "type": "local", "command": [shim_text], "enabled": true, "timeout": 30000 } }
            }));
            ClientInfo {
                id: client.id(),
                name: "OpenCode",
                monogram: "OC",
                state,
                config_path: opencode_config().display().to_string(),
                command: config.clone(),
                config,
            }
        }
    }
}

/// Adds (or repoints) the "outerspace" server in the client's config. Returns a one-line summary.
pub fn add(client: Client, shim: &Path) -> Result<String, SetupError> {
    match client {
        Client::Claude => add_claude(shim),
        Client::Codex => {
            let path = codex_config();
            edit(&path, |text| codex_with_server(text, shim))?;
            Ok(format!("Added to {}", path.display()))
        }
        Client::OpenCode => {
            let path = opencode_config();
            edit(&path, |text| opencode_with_server(text, shim))?;
            Ok(format!("Added to {}", path.display()))
        }
    }
}

fn state(client: Client, shim: &Path) -> State {
    let command = match client {
        Client::Claude => std::fs::read_to_string(home().join(".claude.json")).ok().and_then(|text| {
            let value: serde_json::Value = serde_json::from_str(&text).ok()?;
            value["mcpServers"][SERVER]["command"].as_str().map(str::to_owned)
        }),
        Client::Codex => std::fs::read_to_string(codex_config()).ok().and_then(|text| codex_command(&text)),
        Client::OpenCode => std::fs::read_to_string(opencode_config()).ok().and_then(|text| opencode_command(&text)),
    };
    match command {
        None => State::NotConfigured,
        Some(command) if same_path(&command, shim) => State::Configured,
        Some(_) => State::Outdated,
    }
}

fn add_claude(shim: &Path) -> Result<String, SetupError> {
    let cli = find_on_path(&["claude.exe", "claude.cmd"]).ok_or_else(|| {
        SetupError("Claude Code's `claude` command was not found. Copy the command and run it yourself.".into())
    })?;
    // `add` refuses to overwrite; clear an old entry first (fine if there is none).
    let _ = run(&cli, &["mcp", "remove", "--scope", "user", SERVER]);
    let shim = shim.display().to_string();
    run(&cli, &["mcp", "add", "--transport", "stdio", "--scope", "user", SERVER, "--", &shim])?;
    Ok("Added to Claude Code (user scope)".into())
}

fn run(program: &Path, args: &[&str]) -> Result<(), SetupError> {
    let output = Command::new(program)
        .args(args)
        .creation_flags(CREATE_NO_WINDOW)
        .output()
        .map_err(|e| SetupError(format!("could not run {}: {e}", program.display())))?;
    if output.status.success() {
        Ok(())
    } else {
        let stderr = String::from_utf8_lossy(&output.stderr);
        let stdout = String::from_utf8_lossy(&output.stdout);
        let message = if stderr.trim().is_empty() { stdout } else { stderr };
        Err(SetupError(message.trim().to_owned()))
    }
}

/// `[mcp_servers.outerspace]` set to the shim; everything else in the file left as it was.
fn codex_with_server(text: &str, shim: &Path) -> Result<String, SetupError> {
    let mut doc: toml_edit::DocumentMut =
        text.parse().map_err(|e| SetupError(format!("config.toml is not valid TOML: {e}")))?;
    let servers = doc.entry("mcp_servers").or_insert_with(|| {
        let mut table = toml_edit::Table::new();
        table.set_implicit(true);
        toml_edit::Item::Table(table)
    });
    let servers =
        servers.as_table_mut().ok_or_else(|| SetupError("`mcp_servers` in config.toml is not a table".into()))?;
    let mut server = toml_edit::Table::new();
    server["command"] = toml_edit::value(shim.display().to_string());
    server["args"] = toml_edit::value(toml_edit::Array::new());
    server["startup_timeout_sec"] = toml_edit::value(20);
    servers.insert(SERVER, toml_edit::Item::Table(server));
    Ok(doc.to_string())
}

fn codex_command(text: &str) -> Option<String> {
    let doc: toml_edit::DocumentMut = text.parse().ok()?;
    doc.get("mcp_servers")?.get(SERVER)?.get("command")?.as_str().map(str::to_owned)
}

/// `mcp.outerspace` set to the shim; comments and formatting elsewhere untouched.
fn opencode_with_server(text: &str, shim: &Path) -> Result<String, SetupError> {
    let text = if text.trim().is_empty() { r#"{ "$schema": "https://opencode.ai/config.json" }"# } else { text };
    let root = CstRootNode::parse(text, &ParseOptions::default())
        .map_err(|e| SetupError(format!("the OpenCode config could not be read: {e}")))?;
    let mcp = root.object_value_or_set().object_value_or_set("mcp");
    let shim = shim.display().to_string();
    let server = CstInputValue::Object(vec![
        ("type".into(), "local".into()),
        ("command".into(), CstInputValue::Array(vec![shim.into()])),
        ("enabled".into(), true.into()),
        ("timeout".into(), 30000.into()),
    ]);
    match mcp.get(SERVER) {
        Some(existing) => existing.set_value(server),
        None => {
            mcp.append(SERVER, server);
        }
    }
    Ok(root.to_string())
}

fn opencode_command(text: &str) -> Option<String> {
    let root = CstRootNode::parse(text, &ParseOptions::default()).ok()?;
    let server = root.object_value()?.object_value("mcp")?.object_value(SERVER)?;
    let command = server.get("command")?.value()?.as_array()?.elements().into_iter().next()?;
    command.as_string_lit()?.decoded_value().ok()
}

/// Rewrites `path` through `change`, keeping a backup of what was there.
fn edit(path: &Path, change: impl FnOnce(&str) -> Result<String, SetupError>) -> Result<(), SetupError> {
    let original = std::fs::read_to_string(path).unwrap_or_default();
    let updated = change(&original)?;
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir).map_err(|e| SetupError(format!("could not create {}: {e}", dir.display())))?;
    }
    if !original.is_empty() {
        let backup = path.with_file_name(format!(
            "{}.outerspace.bak",
            path.file_name().map(|n| n.to_string_lossy()).unwrap_or_default()
        ));
        std::fs::write(&backup, &original)
            .map_err(|e| SetupError(format!("could not back up {}: {e}", path.display())))?;
    }
    std::fs::write(path, updated).map_err(|e| SetupError(format!("could not write {}: {e}", path.display())))
}

fn home() -> PathBuf {
    std::env::var_os("USERPROFILE").map(PathBuf::from).unwrap_or_default()
}

fn codex_config() -> PathBuf {
    std::env::var_os("CODEX_HOME").map(PathBuf::from).unwrap_or_else(|| home().join(".codex")).join("config.toml")
}

/// OpenCode reads `opencode.jsonc` or `opencode.json` in `~/.config/opencode` (or `OPENCODE_CONFIG`).
fn opencode_config() -> PathBuf {
    if let Some(path) = std::env::var_os("OPENCODE_CONFIG") {
        return PathBuf::from(path);
    }
    let dir = home().join(".config").join("opencode");
    let jsonc = dir.join("opencode.jsonc");
    if jsonc.exists() { jsonc } else { dir.join("opencode.json") }
}

fn find_on_path(names: &[&str]) -> Option<PathBuf> {
    let path = std::env::var_os("PATH")?;
    std::env::split_paths(&path).find_map(|dir| names.iter().map(|n| dir.join(n)).find(|p| p.is_file()))
}

fn same_path(command: &str, shim: &Path) -> bool {
    command.trim_matches('"').eq_ignore_ascii_case(&shim.display().to_string())
}

fn pretty(value: &serde_json::Value) -> String {
    serde_json::to_string_pretty(value).expect("JSON values always serialize")
}

#[cfg(test)]
mod tests {
    use super::*;

    const SHIM: &str = r"C:\Users\me\AppData\Local\Programs\OuterSpace\outerspace-mcp.exe";

    #[test]
    fn codex_keeps_the_rest_of_the_file() {
        let original = "# my settings\nmodel = \"o3\" # favourite\n\n[mcp_servers.other]\ncommand = \"x\"\n";
        let updated = codex_with_server(original, Path::new(SHIM)).unwrap();
        assert!(updated.starts_with("# my settings\nmodel = \"o3\" # favourite\n"), "{updated}");
        assert!(updated.contains("[mcp_servers.other]\ncommand = \"x\""), "{updated}");
        assert_eq!(codex_command(&updated).as_deref(), Some(SHIM));
        assert!(updated.contains("startup_timeout_sec = 20"));
        // Running it again repoints instead of duplicating.
        let again = codex_with_server(&updated, Path::new(r"D:\other\outerspace-mcp.exe")).unwrap();
        assert_eq!(again.matches("[mcp_servers.outerspace]").count(), 1);
        assert_eq!(codex_command(&again).as_deref(), Some(r"D:\other\outerspace-mcp.exe"));
    }

    #[test]
    fn codex_from_an_empty_file() {
        let updated = codex_with_server("", Path::new(SHIM)).unwrap();
        assert_eq!(codex_command(&updated).as_deref(), Some(SHIM));
    }

    #[test]
    fn opencode_keeps_comments() {
        let original = "{\n  // my theme\n  \"theme\": \"tokyonight\",\n  \"mcp\": {\n    \"other\": { \"type\": \"remote\", \"url\": \"https://x\" } // keep\n  }\n}\n";
        let updated = opencode_with_server(original, Path::new(SHIM)).unwrap();
        assert!(updated.contains("// my theme") && updated.contains("// keep"), "{updated}");
        assert!(updated.contains("\"other\""), "{updated}");
        assert_eq!(opencode_command(&updated).as_deref(), Some(SHIM));
        let again = opencode_with_server(&updated, Path::new(SHIM)).unwrap();
        assert_eq!(again.matches("\"outerspace\"").count(), 1, "{again}");
    }

    #[test]
    fn opencode_from_nothing() {
        let updated = opencode_with_server("", Path::new(SHIM)).unwrap();
        assert!(updated.contains("opencode.ai/config.json"));
        assert_eq!(opencode_command(&updated).as_deref(), Some(SHIM));
    }

    #[test]
    fn paths_compare_case_insensitively() {
        assert!(same_path(&SHIM.to_uppercase(), Path::new(SHIM)));
        assert!(!same_path(r"C:\elsewhere\outerspace-mcp.exe", Path::new(SHIM)));
    }
}
