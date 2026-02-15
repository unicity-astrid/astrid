/**
 * MCP server setup — stdio transport, tool registration.
 *
 * Creates an MCP server that exposes OpenClaw plugin tools over the
 * stdio transport. Also listens for custom `notifications/astrid.hookEvent`
 * notifications from the Astrid client and dispatches them to the
 * plugin's `onEvent()` handler.
 */

import { McpServer } from "@modelcontextprotocol/sdk/server/mcp.js";
import { StdioServerTransport } from "@modelcontextprotocol/sdk/server/stdio.js";
import type { JSONRPCMessage } from "@modelcontextprotocol/sdk/types.js";
import { z } from "zod";

import type {
  PluginManifest,
  PluginModule,
  RegisteredTool,
  PluginContext,
  PluginLogger,
} from "./types.js";
import { executeToolCall } from "./tool-adapter.js";

/**
 * Create and start the MCP bridge server.
 *
 * @param manifest - The plugin manifest.
 * @param pluginModule - The loaded plugin module.
 * @param config - Optional plugin configuration (from CLI `--config`).
 * @param pluginDir - Absolute path to the plugin directory.
 * @returns The MCP server instance (for lifecycle management).
 */
export async function startBridgeServer(
  manifest: PluginManifest,
  pluginModule: PluginModule,
  config: Record<string, unknown>,
  pluginDir: string
): Promise<McpServer> {
  // ── Collect tools registered by the plugin ───────────────────

  const registeredTools: Map<string, RegisteredTool> = new Map();

  const logger: PluginLogger = {
    info: (msg, ...args) =>
      console.error(`[${manifest.id}] INFO: ${msg}`, ...args),
    warn: (msg, ...args) =>
      console.error(`[${manifest.id}] WARN: ${msg}`, ...args),
    error: (msg, ...args) =>
      console.error(`[${manifest.id}] ERROR: ${msg}`, ...args),
    debug: (msg, ...args) =>
      console.error(`[${manifest.id}] DEBUG: ${msg}`, ...args),
  };

  const context: PluginContext = {
    config,
    logger,
    workspace: process.cwd(),
    registerTool(name, definition, handler) {
      if (registeredTools.has(name)) {
        logger.warn(`Tool '${name}' already registered; overwriting`);
      }
      registeredTools.set(name, { name, definition, handler });
      logger.info(`Registered tool: ${name}`);
    },
  };

  // ── Activate the plugin (registers tools) ────────────────────

  await pluginModule.activate(context);

  if (registeredTools.size === 0) {
    logger.warn("Plugin activated but registered no tools");
  }

  // ── Create MCP server ────────────────────────────────────────

  const server = new McpServer({
    name: `openclaw-${manifest.id}`,
    version: manifest.version,
  });

  // ── Register each tool with the MCP server ───────────────────

  for (const [toolName, tool] of registeredTools) {
    // Convert the OpenClaw inputSchema to a Zod schema shape for
    // the MCP SDK. The MCP SDK expects zod schemas, but OpenClaw
    // plugins provide JSON Schema objects. We use z.record() as a
    // permissive schema and pass the raw JSON Schema as description
    // metadata so the LLM sees the actual schema.
    //
    // The actual validation happens in the plugin's handler, not here.
    server.tool(
      toolName,
      tool.definition.description,
      { params: z.record(z.unknown()).optional() },
      async (args) => {
        const result = await executeToolCall(
          tool,
          (args.params ?? {}) as Record<string, unknown>
        );
        return result;
      }
    );
  }

  // ── Start stdio transport ────────────────────────────────────

  const transport = new StdioServerTransport();

  // Hook event forwarding: listen for custom notifications from the
  // Astrid client. The MCP SDK's transport emits 'message' events
  // for incoming JSON-RPC messages. We intercept notifications with
  // method "notifications/astrid.hookEvent" and dispatch to the
  // plugin's onEvent() handler.
  if (pluginModule.onEvent) {
    const onEvent = pluginModule.onEvent.bind(pluginModule);
    transport.onmessage = ((originalHandler: typeof transport.onmessage) => {
      return (message: JSONRPCMessage) => {
        const msg = message as JSONRPCMessage & {
          method?: string;
          params?: { event?: string; data?: unknown };
        };
        if (
          msg.method === "notifications/astrid.hookEvent" &&
          msg.params?.event
        ) {
          // Fire-and-forget dispatch to plugin
          Promise.resolve(onEvent(msg.params.event, msg.params.data)).catch(
            (err) => {
              logger.error(`Hook event handler failed: ${err}`);
            }
          );
        }
        // Always pass through to the original handler
        originalHandler?.call(transport, message);
      };
    })(transport.onmessage);
  }

  await server.connect(transport);

  logger.info(
    `MCP bridge server started for plugin '${manifest.id}' ` +
      `with ${registeredTools.size} tool(s)`
  );

  // ── Graceful shutdown ────────────────────────────────────────

  const shutdown = async () => {
    logger.info("Shutting down...");
    try {
      await pluginModule.deactivate?.();
    } catch (err) {
      logger.error(`Plugin deactivate() failed: ${err}`);
    }
    await server.close();
    process.exit(0);
  };

  process.on("SIGTERM", shutdown);
  process.on("SIGINT", shutdown);

  // Also shut down when stdin closes (parent process died)
  process.stdin.on("end", shutdown);

  return server;
}
