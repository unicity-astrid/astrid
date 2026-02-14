#!/usr/bin/env node
/**
 * CLI entry point for the OpenClaw MCP Bridge.
 *
 * Usage:
 *   node dist/index.js --plugin-dir <path> [--config <json>]
 *
 * Loads an OpenClaw plugin from the specified directory, activates it,
 * and exposes its tools as an MCP server over stdio.
 */

import * as path from "node:path";
import { loadManifest, loadPluginModule } from "./loader.js";
import { startBridgeServer } from "./bridge-server.js";

function parseArgs(argv: string[]): {
  pluginDir: string;
  config: Record<string, unknown>;
} {
  let pluginDir: string | undefined;
  let configJson: string | undefined;

  for (let i = 2; i < argv.length; i++) {
    const arg = argv[i];
    if (arg === "--plugin-dir" && i + 1 < argv.length) {
      pluginDir = argv[++i];
    } else if (arg === "--config" && i + 1 < argv.length) {
      configJson = argv[++i];
    } else if (arg === "--help" || arg === "-h") {
      console.error(
        "Usage: openclaw-mcp-bridge --plugin-dir <path> [--config <json>]"
      );
      console.error("");
      console.error("Options:");
      console.error(
        "  --plugin-dir <path>  Path to the OpenClaw plugin directory"
      );
      console.error(
        "  --config <json>      Plugin configuration as a JSON string"
      );
      process.exit(0);
    }
  }

  if (!pluginDir) {
    console.error(
      "Error: --plugin-dir is required\n" +
        "Usage: openclaw-mcp-bridge --plugin-dir <path> [--config <json>]"
    );
    process.exit(1);
  }

  let config: Record<string, unknown> = {};
  if (configJson) {
    try {
      config = JSON.parse(configJson);
    } catch (err) {
      console.error(`Error: Invalid JSON in --config: ${err}`);
      process.exit(1);
    }
  }

  return {
    pluginDir: path.resolve(pluginDir),
    config,
  };
}

async function main(): Promise<void> {
  const { pluginDir, config } = parseArgs(process.argv);

  // 1. Load and validate manifest
  console.error(`[bridge] Loading plugin from: ${pluginDir}`);
  const manifest = loadManifest(pluginDir);
  console.error(
    `[bridge] Plugin: ${manifest.name} v${manifest.version} (${manifest.id})`
  );

  // 2. Load the plugin module
  const pluginModule = await loadPluginModule(pluginDir, manifest.main);

  // 3. Start the MCP bridge server (activates plugin + registers tools)
  await startBridgeServer(manifest, pluginModule, config, pluginDir);

  // Server is now running on stdio — the process stays alive until
  // SIGTERM, SIGINT, or stdin closes.
}

main().catch((err) => {
  console.error(`[bridge] Fatal error: ${err}`);
  process.exit(1);
});
