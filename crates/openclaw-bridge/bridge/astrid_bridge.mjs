// astrid_bridge.mjs — OpenClaw plugin bridge for Astrid (Tier 2).
//
// Loads an OpenClaw plugin, captures tool/channel/service registrations,
// and exposes them over MCP JSON-RPC on stdin/stdout.
//
// Usage: node astrid_bridge.mjs --entry ./src/index.js --plugin-id openclaw-unicity
//
// No npm dependencies — raw JSON-RPC over stdio.

// ── CRITICAL: Redirect console to stderr ────────────────────────────
// stdout is the MCP transport — any non-JSON-RPC output corrupts the
// stream. Plugins and their dependencies may use console.log(), so we
// must intercept it before any imports run.
const _origLog = console.log;
const _origWarn = console.warn;
const _origInfo = console.info;
const _origDebug = console.debug;
console.log = (...args) => process.stderr.write(args.join(" ") + "\n");
console.warn = (...args) => process.stderr.write(args.join(" ") + "\n");
console.info = (...args) => process.stderr.write(args.join(" ") + "\n");
console.debug = (...args) => process.stderr.write(args.join(" ") + "\n");

import { createInterface } from "node:readline";
import { pathToFileURL } from "node:url";
import { resolve, dirname } from "node:path";
import {
  writeFileSync,
  mkdirSync,
} from "node:fs";

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

if (/[/\\]|\.\./.test(pluginId)) {
  process.stderr.write("astrid_bridge: --plugin-id must not contain path separators or '..'\n");
  process.exit(1);
}

// ── Validate HOME early ─────────────────────────────────────────────
// HOME is used to build the plugin config directory. A manipulated HOME
// (e.g. "/tmp/../etc") would cause path traversal via resolve().
// Reject values containing ".." path segments and resolve once.
const rawHome = process.env.HOME || "/tmp";
if (/(?:^|[/\\])\.\.(?:[/\\]|$)/.test(rawHome)) {
  process.stderr.write("astrid_bridge: HOME contains '..' path components — refusing to start\n");
  process.exit(1);
}
const resolvedHome = resolve(rawHome);
const pluginConfigBase = resolve(resolvedHome, ".astrid", "plugins");

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
const registeredHooks = new Map();
const unsupportedRegistrations = [];
const eventHandlers = new Map();
let pluginConfig = {};
let servicesReady = false;
let shuttingDown = false;

// ── OpenClaw Plugin API mock (OpenClawPluginApi) ────────────────────
// Matches the real OpenClaw plugin API surface. All 11 registration
// methods are captured; unsupported ones are logged for diagnostics.
const pluginApi = {
  // Plugin identity (populated after manifest is read)
  id: pluginId,
  name: pluginId,
  version: "0.0.0",
  description: "",
  source: "astrid-bridge",

  // Config
  config: {},
  pluginConfig: {},

  // Logger
  logger: log,

  // Path resolution
  resolvePath: (input) => resolve(dirname(resolve(entryPath)), input),

  // Runtime helpers
  runtime: {
    config: {
      loadConfig: () => pluginConfig,
      writeConfigFile: (data) => {
        const configDir = resolve(pluginConfigBase, pluginId);
        // Defense in depth: verify resolved path is under the expected base
        if (!configDir.startsWith(pluginConfigBase + "/")) {
          log.error("writeConfigFile: path traversal detected — refusing to write");
          return;
        }
        try {
          mkdirSync(configDir, { recursive: true });
          const configPath = resolve(configDir, "config.json");
          writeFileSync(configPath, JSON.stringify(data, null, 2));
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

  // ── Registration methods (11 total) ──────────────────────────────
  // Must: registerTool, registerService
  //
  // Supports two calling conventions:
  //   registerTool(nameStr, definitionObj, handlerFn)   — name is a string
  //   registerTool(definitionObj, handlerFn)             — name is on definition.name
  registerTool: (nameOrDef, definitionOrHandler, maybeHandler) => {
    let toolName, definition, handler;
    if (typeof nameOrDef === "string") {
      // registerTool("name", { ... }, handler)
      toolName = nameOrDef;
      definition = definitionOrHandler;
      handler = maybeHandler;
    } else if (nameOrDef && typeof nameOrDef === "object") {
      // registerTool({ name: "name", ... }, handler)
      toolName = nameOrDef.name || "unnamed";
      definition = nameOrDef;
      handler = definitionOrHandler;
    } else {
      log.warn(`registerTool: unexpected first argument type: ${typeof nameOrDef}`);
      return;
    }
    registeredTools.set(toolName, { name: toolName, definition, handler });
    log.debug(`Registered tool: ${toolName}`);
  },
  registerService: (name, service) => {
    registeredServices.set(name, service);
    log.debug(`Registered service: ${name}`);
  },

  // Should: registerChannel, registerHook, on
  //
  // Same dual calling convention as registerTool.
  registerChannel: (nameOrDef, definitionOrHandler, maybeHandler) => {
    let chanName, definition, handler;
    if (typeof nameOrDef === "string") {
      chanName = nameOrDef;
      definition = definitionOrHandler;
      handler = maybeHandler;
    } else if (nameOrDef && typeof nameOrDef === "object") {
      chanName = nameOrDef.name || "unnamed";
      definition = nameOrDef;
      handler = definitionOrHandler;
    } else {
      log.warn(`registerChannel: unexpected first argument type: ${typeof nameOrDef}`);
      return;
    }
    registeredChannels.set(chanName, { name: chanName, definition, handler });
    log.debug(`Registered channel: ${chanName}`);
  },
  registerHook: (name, handler) => {
    registeredHooks.set(name, handler);
    log.debug(`Registered hook: ${name}`);
  },
  on: (event, handler) => {
    if (!eventHandlers.has(event)) eventHandlers.set(event, []);
    eventHandlers.get(event).push(handler);
    log.debug(`Registered event handler: ${event}`);
  },

  // Nice to have: registerCommand, registerGatewayMethod, registerHttpHandler, registerHttpRoute
  registerCommand: (name, definition) => {
    unsupportedRegistrations.push({ type: "command", name });
    log.debug(`Registered command: ${name} (not wired to MCP bridge)`);
  },
  registerGatewayMethod: (name, handler) => {
    unsupportedRegistrations.push({ type: "gatewayMethod", name });
    log.debug(`Registered gateway method: ${name} (not wired to MCP bridge)`);
  },
  registerHttpHandler: (path, handler) => {
    unsupportedRegistrations.push({ type: "httpHandler", name: path });
    log.debug(`Registered HTTP handler: ${path} (not wired to MCP bridge)`);
  },
  registerHttpRoute: (method, path, handler) => {
    unsupportedRegistrations.push({ type: "httpRoute", name: `${method} ${path}` });
    log.debug(`Registered HTTP route: ${method} ${path} (not wired to MCP bridge)`);
  },

  // Out of scope: registerProvider (OAuth flows), registerCli (host-side)
  registerProvider: (name, definition) => {
    unsupportedRegistrations.push({ type: "provider", name });
    log.debug(`Registered provider: ${name} (not wired to MCP bridge)`);
  },
  registerCli: (name, definition) => {
    unsupportedRegistrations.push({ type: "cli", name });
    log.debug(`Registered CLI command: ${name} (not available via MCP bridge)`);
  },
};

// ── JSON-RPC over stdio ─────────────────────────────────────────────

function sendResponse(id, result) {
  const msg = JSON.stringify({ jsonrpc: "2.0", id, result });
  log.debug(`→ ${msg}`);
  process.stdout.write(msg + "\n");
}

function sendError(id, code, message, data) {
  const err = { jsonrpc: "2.0", id, error: { code, message } };
  if (data !== undefined) err.error.data = data;
  const msg = JSON.stringify(err);
  log.debug(`→ ${msg}`);
  process.stdout.write(msg + "\n");
}

function sendNotification(method, params) {
  const msg = JSON.stringify({ jsonrpc: "2.0", method, params });
  log.debug(`→ (notification) ${msg}`);
  process.stdout.write(msg + "\n");
}

// ── MCP method handlers ─────────────────────────────────────────────

function handleInitialize(id, params) {
  sendResponse(id, {
    protocolVersion: "2025-11-25",
    capabilities: {
      tools: { listChanged: false },
    },
    serverInfo: {
      name: `astrid-bridge:${pluginId}`,
      version: "0.1.0",
    },
  });

  // Start services asynchronously after responding
  startServices().catch((e) => {
    log.error(`Service startup failed: ${e?.message ?? e}`);
  });
}

function handleToolsList(id) {
  const tools = [];

  for (const [name, tool] of registeredTools) {
    // Coerce inputSchema to a plain JSON object — rmcp requires a JSON object
    // (serde_json::Map), not an array, null, or primitive.
    let schema = tool.definition?.inputSchema || tool.definition?.input_schema;
    if (!schema || typeof schema !== "object" || Array.isArray(schema)) {
      schema = { type: "object" };
    }
    // Coerce description to string — rmcp expects Option<Cow<str>>
    const desc = typeof tool.definition?.description === "string"
      ? tool.definition.description
      : "";
    // Coerce name to string
    const toolName = typeof name === "string" ? name : String(name);

    tools.push({
      name: toolName,
      description: desc,
      inputSchema: schema,
    });
  }

  // Add special tool for agent context
  tools.push({
    name: "__astrid_get_agent_context",
    description: "Returns plugin context for agent initialization (wallet identity, security rules)",
    inputSchema: { type: "object", properties: {} },
  });

  const response = { tools };
  log.debug(`tools/list response: ${JSON.stringify(response)}`);
  sendResponse(id, response);
}

async function handleToolsCall(id, params) {
  const toolName = params?.name;
  const toolArgs = params?.arguments || {};

  // Special: agent context tool (allowed before services are ready —
  // before_agent_start handlers do not depend on services)
  if (toolName === "__astrid_get_agent_context") {
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
      content: [{ type: "text", text: "Unknown tool" }],
      isError: true,
    });
    return;
  }

  if (!servicesReady) {
    sendResponse(id, {
      content: [{ type: "text", text: "Service not ready yet — plugin is still initializing" }],
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
    log.error(`Tool call failed: ${e.message}\n${e.stack ?? ""}`);
    sendResponse(id, {
      content: [{ type: "text", text: "Tool execution failed" }],
      isError: true,
    });
  }
}

async function handleNotification(method, params) {
  if (method === "notifications/initialized") {
    log.info("MCP session initialized");
    return;
  }
  if (method === "notifications/astrid.setPluginConfig") {
    if (params?.config && typeof params.config === "object" && !Array.isArray(params.config)) {
      pluginConfig = params.config;
      log.info(`Plugin config updated (${Object.keys(pluginConfig).length} keys)`);
    }
    return;
  }
  if (method === "notifications/astrid.hookEvent") {
    const event = params?.event;
    const data = params?.data;
    const handlers = eventHandlers.get(event) || [];
    for (const handler of handlers) {
      try {
        await handler(data);
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
  let failedCount = 0;
  for (const [name, service] of registeredServices) {
    try {
      log.info(`Starting service: ${name}`);
      if (typeof service.start === "function") {
        await service.start();
      }
      log.info(`Service started: ${name}`);
    } catch (e) {
      failedCount++;
      log.error(`Service ${name} failed to start: ${e.message}`);
    }
  }
  servicesReady = true;
  if (failedCount > 0) {
    log.warn(`Services ready (${failedCount} failed to start — tool calls will proceed)`);
  } else {
    log.info("All services started");
  }
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

// ── Shutdown guard ──────────────────────────────────────────────────

async function shutdown(reason) {
  if (shuttingDown) return;
  shuttingDown = true;
  log.info(`${reason} — shutting down`);
  await stopServices();
  process.exit(0);
}

// ── Message dispatch ────────────────────────────────────────────────

async function dispatch(msg) {
  try {
    log.debug(`← ${msg}`);
    const parsed = JSON.parse(msg);

    // Notification (no id)
    if (parsed.id === undefined || parsed.id === null) {
      await handleNotification(parsed.method, parsed.params);
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
        sendError(parsed.id, -32601, "Method not found");
    }
  } catch (e) {
    log.error(`Failed to parse message: ${e.message}`);
    sendError(null, -32700, "Parse error");
  }
}

// ── Plugin loading ──────────────────────────────────────────────────

async function loadPlugin() {
  const resolved = resolve(entryPath);
  const fileUrl = pathToFileURL(resolved).href;

  log.info(`Loading plugin from: ${resolved}`);

  try {
    const mod = await import(fileUrl);
    const defaultExport = mod.default;

    if (defaultExport && typeof defaultExport === "object" && typeof defaultExport.register === "function") {
      // Object form: export default { id, name, configSchema, register(api) {} }
      log.debug("Detected object-form plugin with register(api)");
      if (defaultExport.id) pluginApi.id = defaultExport.id;
      if (defaultExport.name) pluginApi.name = defaultExport.name;
      if (defaultExport.version) pluginApi.version = defaultExport.version;
      if (defaultExport.description) pluginApi.description = defaultExport.description;
      await defaultExport.register(pluginApi);
    } else if (typeof defaultExport === "function") {
      // Function form: export default function(api) {}
      log.debug("Detected function-form plugin");
      await defaultExport(pluginApi);
    } else if (typeof mod.register === "function") {
      // Named export: export function register(api) {}
      log.debug("Detected named register() export");
      await mod.register(pluginApi);
    } else {
      // Fallback: try activate() for backwards compatibility
      // OpenClaw loader uses: def.register ?? def.activate
      const activate = defaultExport?.activate || mod.activate;
      if (typeof activate === "function") {
        log.debug("Detected legacy activate() pattern");
        await activate(pluginApi);
      } else {
        log.warn("No register(), activate(), or callable default export found — plugin may use side-effect registration");
      }
    }

    log.info(
      `Plugin loaded: ${registeredTools.size} tools, ` +
        `${registeredChannels.size} channels, ` +
        `${registeredServices.size} services` +
        (unsupportedRegistrations.length > 0 ? `, ${unsupportedRegistrations.length} unsupported` : "")
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
    if (line.trim()) {
      dispatch(line.trim()).catch((e) => {
        log.error(`Dispatch failed: ${e?.message ?? e}`);
      });
    }
  });

  const onShutdown = (reason) => shutdown(reason).catch((e) => {
    log.error(`Shutdown error: ${e?.message ?? e}`);
    process.exit(1);
  });

  rl.on("close", () => onShutdown("stdin closed"));
  process.on("SIGTERM", () => onShutdown("SIGTERM received"));
  process.on("SIGINT", () => onShutdown("SIGINT received"));
}

main().catch((e) => {
  log.error(`Bridge fatal: ${e.message}\n${e.stack}`);
  process.exit(1);
});
