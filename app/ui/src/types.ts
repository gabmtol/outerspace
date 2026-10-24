// Mirrors the serde types in app/src-tauri/src/commands.rs and crates/outerspace-core.

export interface DisplayInfo {
  width: number;
  height: number;
  hz: number;
  x: number;
  y: number;
}

export interface DisplayMode {
  width: number;
  height: number;
  hz: number;
}

export type ErrorCode =
  | "driverMissing"
  | "driverDisabled"
  | "restartRequired"
  | "driverFailed"
  | "connectFailed"
  | "installFailed";

export type PowerState =
  | { kind: "off" }
  | { kind: "starting" }
  | { kind: "stopping" }
  | { kind: "on"; display: DisplayInfo }
  | { kind: "error"; code: ErrorCode; message?: string };

export interface Status {
  power: PowerState;
  installing: boolean;
  port: number;
  version: string;
  autostart: boolean;
  transparency: boolean;
  windowsOnDisplay: number | null;
  /** The chosen resolution, and the ones to choose from. */
  mode: DisplayMode;
  modes: DisplayMode[];
}

export interface ClientSnippet {
  id: "claude" | "codex" | "opencode";
  name: string;
  monogram: string;
  /// "outdated": configured, but for another copy of outerspace-mcp.exe.
  state: "notConfigured" | "configured" | "outdated";
  command: string;
  configPath: string;
  config: string;
}
