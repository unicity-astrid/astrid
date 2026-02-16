// astrid_bridge.mjs — Universal MCP bridge for OpenClaw plugins (Tier 2).
//
// Loads an OpenClaw plugin, captures tool/channel/service registrations,
// and exposes them over MCP JSON-RPC on stdin/stdout.
//
// Usage: node astrid_bridge.mjs --entry ./src/index.js --plugin-id openclaw-unicity
//
// No npm dependencies — raw JSON-RPC over stdio.

import { createInterface } from "node:readline";
import { pathToFileURL } from "node:url";
import { resolve, dirname } from "node:path";

// ── CLI args ────────────────────────────────────────────────────────
const args = process.argv.slice(2);
let entryPath = null;
let pluginId = "unknown";

for (let i = 0; i < args.length; i++) {
  if (args[i] === "--entry" && args[i + 1]) entryPath = args[++i];
  else if (args[i] === "--plugin-id" && args[i + 1]) pluginId = args[++i];
}

if (!entryPath) {
  process.stderr.write("astrid_bridge: --entry <path> is required\n");
  process.exit(1);
}

// ── Logger (stderr only — stdout is MCP transport) ──────────────────
const log = {
  info: (msg) => process.stderr.write(`[${pluginId}] INFO: ${msg}\n`),
  warn: (msg) => process.stderr.write(`[${pluginId}] WARN: ${msg}\n`),
  error: (msg) => process.stderr.write(`[${pluginId}] ERROR: ${msg}\n`),
  debug: (msg) => process.stderr.write(`[${pluginId}] DEBUG: ${msg}\n`),
};

// ── Plugin registrations ────────────────────────────────────────────
const registeredTools = new Map();
const registeredChannels = new Map();
const registeredServices = new Map();
const registeredCli = new Map();
const eventHandlers = new Map();
let pluginConfig = {};
let agentContext = null;
let servicesReady = false;

// ── OpenClaw Plugin API mock ────────────────────────────────────────
const pluginApi = {
  logger: log,
  runtime: {
    config: {
      loadConfig: () => pluginConfig,
      writeConfigFile: (data) => {
        const configPath = resolve(
          process.env.HOME || ".",
          `.astrid-plugin-config.json`
        );
        try {
          const fs = await_import_fs();
          fs.writeFileSync(configPath, JSON.stringify(data, null, 2));
          sendNotification("notifications/astrid.configChanged", {
            pluginId,
            path: configPath,
          });
        } catch (e) {
          log.error(`writeConfigFile failed: ${e.message}`);
        }
      },
    },
    channel: {
      reply: (context, content) => {
        sendNotification("notifications/astrid.inboundMessage", {
          pluginId,
          context,
          content,
        });
      },
    },
  },
  registerTool: (name, definition, handler) => {
    registeredTools.set(name, { name, definition, handler });
    log.debug(`Registered tool: ${name}`);
  },
  registerChannel: (name, definition, handler) => {
    registeredChannels.set(name, { name, definition, handler });
    log.debug(`Registered channel: ${name}`);
  },
  registerService: (name, service) => {
    registeredServices.set(name, service);
    log.debug(`Registered service: ${name}`);
  },
  registerCli: (name, definition) => {
    registeredCli.set(name, definition);
    log.debug(`Registered CLI command: ${name} (not available via MCP bridge)`);
  },
  on: (event, handler) => {
    if (!eventHandlers.has(event)) eventHandlers.set(event, []);
    eventHandlers.get(event).push(handler);
    log.debug(`Registered event handler: ${event}`);
  },
};

// Lazy fs import (only if plugin calls writeConfigFile)
let _fs = null;
function await_import_fs() {
  if (!_fs) {
    // Dynamic import would be async; use createRequire for sync access
    const { createRequire } = await_import_module();
    const require = createRequire(import.meta.url);
    _fs = require("node:fs");
  }
  return _fs;
}
let _module = null;
function await_import_module() {
  if (!_module) _module = { createRequire: (await import("node:module")).createRequire };
  return _module;
}

// ── JSON-RPC over stdio ─────────────────────────────────────────────
let jsonRpcId = 0;

function sendResponse(id, result) {
  const msg = JSON.stringify({ jsonrpc: "2.0", id, result });
  process.stdout.write(msg + "\n");
}

function sendError(id, code, message, data) {
  const err = { jsonrpc: "2.0", id, error: { code, message } };
  if (data !== undefined) err.error.data = data;
  process.stdout.write(JSON.stringify(err) + "\n");
}

function sendNotification(method, params) {
  const msg = JSON.stringify({ jsonrpc: "2.0", method, params });
  process.stdout.write(msg + "\n");
}

// ── MCP method handlers ─────────────────────────────────────────────

function handleInitialize(id, params) {
  // Extract config from initialize params if provided
  if (params?.initializationOptions?.config) {
    pluginConfig = params.initializationOptions.config;
  }

  sendResponse(id, {
    protocolVersion: "2024-11-05",
    capabilities: {
      tools: { listChanged: false },
    },
    serverInfo: {
      name: `astrid-bridge:${pluginId}`,
      version: "0.1.0",
    },
  });

  // Start services asynchronously after responding
  startServices();
}

function handleToolsList(id) {
  const tools = [];

  for (const [name, tool] of registeredTools) {
    const schema = tool.definition?.inputSchema || tool.definition?.input_schema || { type: "object" };
    tools.push({
      name,
      description: tool.definition?.description || "",
      inputSchema: typeof schema === "object" ? schema : { type: "object" },
    });
  }

  // Add special tool for agent context
  tools.push({
    name: "__astrid_get_agent_context",
    description: "Returns plugin context for agent initialization (wallet identity, security rules)",
    inputSchema: { type: "object", properties: {} },
  });

  sendResponse(id, { tools });
}

async function handleToolsCall(id, params) {
  const toolName = params?.name;
  const toolArgs = params?.arguments || {};

  // Special: agent context tool
  if (toolName === "__astrid_get_agent_context") {
    // Fire before_agent_start handlers
    const handlers = eventHandlers.get("before_agent_start") || [];
    let context = {};
    for (const handler of handlers) {
      try {
        const result = await handler(toolArgs);
        if (result && typeof result === "object") {
          context = { ...context, ...result };
        }
      } catch (e) {
        log.error(`before_agent_start handler failed: ${e.message}`);
      }
    }
    sendResponse(id, {
      content: [{ type: "text", text: JSON.stringify(context) }],
    });
    return;
  }

  const tool = registeredTools.get(toolName);
  if (!tool) {
    sendResponse(id, {
      content: [{ type: "text", text: `Unknown tool: ${toolName}` }],
      isError: true,
    });
    return;
  }

  if (!servicesReady) {
    sendResponse(id, {
      content: [{ type: "text", text: `Service not ready yet — plugin is still initializing` }],
      isError: true,
    });
    return;
  }

  try {
    const result = await tool.handler(toolName, toolArgs);
    const text = typeof result === "string" ? result : JSON.stringify(result);
    sendResponse(id, {
      content: [{ type: "text", text }],
    });
  } catch (e) {
    log.error(`Tool ${toolName} failed: ${e.message}`);
    sendResponse(id, {
      content: [{ type: "text", text: `Tool execution failed: ${e.message}` }],
      isError: true,
    });
  }
}

function handleNotification(method, params) {
  if (method === "notifications/initialized") {
    log.info("MCP session initialized");
    return;
  }
  if (method === "notifications/astrid.hookEvent") {
    const event = params?.event;
    const data = params?.data;
    const handlers = eventHandlers.get(event) || [];
    for (const handler of handlers) {
      try {
        handler(data);
      } catch (e) {
        log.error(`Hook event handler for ${event} failed: ${e.message}`);
      }
    }
    return;
  }
  log.debug(`Unhandled notification: ${method}`);
}

// ── Service lifecycle ───────────────────────────────────────────────

async function startServices() {
  for (const [name, service] of registeredServices) {
    try {
      log.info(`Starting service: ${name}`);
      if (typeof service.start === "function") {
        await service.start();
      }
      log.info(`Service started: ${name}`);
    } catch (e) {
      log.error(`Service ${name} failed to start: ${e.message}`);
    }
  }
  servicesReady = true;
  log.info("All services started");
}

async function stopServices() {
  for (const [name, service] of registeredServices) {
    try {
      if (typeof service.stop === "function") {
        await service.stop();
      }
      log.debug(`Service stopped: ${name}`);
    } catch (e) {
      log.error(`Service ${name} failed to stop: ${e.message}`);
    }
  }
}

// ── Message dispatch ────────────────────────────────────────────────

async function dispatch(msg) {
  try {
    const parsed = JSON.parse(msg);

    // Notification (no id)
    if (parsed.id === undefined || parsed.id === null) {
      handleNotification(parsed.method, parsed.params);
      return;
    }

    // Request
    switch (parsed.method) {
      case "initialize":
        handleInitialize(parsed.id, parsed.params);
        break;
      case "tools/list":
        handleToolsList(parsed.id);
        break;
      case "tools/call":
        await handleToolsCall(parsed.id, parsed.params);
        break;
      case "ping":
        sendResponse(parsed.id, {});
        break;
      default:
        sendError(parsed.id, -32601, `Method not found: ${parsed.method}`);
    }
  } catch (e) {
    log.error(`Failed to parse message: ${e.message}`);
  }
}

// ── Plugin loading ──────────────────────────────────────────────────

async function loadPlugin() {
  const resolved = resolve(entryPath);
  const fileUrl = pathToFileURL(resolved).href;

  log.info(`Loading plugin from: ${resolved}`);

  try {
    const mod = await import(fileUrl);
    const activate = mod.default?.activate || mod.activate;

    if (typeof activate === "function") {
      log.debug("Calling plugin activate()");
      await activate(pluginApi);
    } else {
      // Some plugins export the API object directly and register via side effects
      log.debug("No activate() found — plugin may use side-effect registration");
      // Try calling default export as function
      if (typeof mod.default === "function") {
        await mod.default(pluginApi);
      }
    }

    log.info(
      `Plugin loaded: ${registeredTools.size} tools, ` +
        `${registeredChannels.size} channels, ` +
        `${registeredServices.size} services`
    );
  } catch (e) {
    log.error(`Failed to load plugin: ${e.message}\n${e.stack}`);
    process.exit(1);
  }
}

// ── Main ────────────────────────────────────────────────────────────

async function main() {
  await loadPlugin();

  const rl = createInterface({ input: process.stdin, terminal: false });

  rl.on("line", (line) => {
    if (line.trim()) dispatch(line.trim());
  });

  rl.on("close", async () => {
    log.info("stdin closed — shutting down");
    await stopServices();
    process.exit(0);
  });

  process.on("SIGTERM", async () => {
    log.info("SIGTERM received — shutting down");
    await stopServices();
    process.exit(0);
  });

  process.on("SIGINT", async () => {
    log.info("SIGINT received — shutting down");
    await stopServices();
    process.exit(0);
  });
}

main().catch((e) => {
  log.error(`Bridge fatal: ${e.message}\n${e.stack}`);
  process.exit(1);
});
