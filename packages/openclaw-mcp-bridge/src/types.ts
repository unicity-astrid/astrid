/**
 * OpenClaw plugin types — pinned API surface.
 *
 * We support only the tool-plugin subset (~20% of `PluginRuntime`).
 *
 * Supported:
 * - activate(context) / deactivate()
 * - registerTool(name, definition, handler)
 * - tool.execute(id, params) (via handler callback)
 * - context.config, context.logger, context.workspace
 *
 * Unsupported (documented):
 * - registerProvider, registerCompletion, slash commands, inline
 *   completions, multi-file edits — these are editor-specific features
 *   that don't map to MCP tools.
 */

import { z } from "zod";

// ── Plugin Manifest (openclaw.plugin.json) ─────────────────────────

export const PluginManifestSchema = z.object({
  id: z.string().min(1),
  name: z.string().min(1),
  version: z.string().min(1),
  description: z.string().optional(),
  main: z.string().min(1),
  engines: z
    .object({
      openclaw: z.string().optional(),
    })
    .optional(),
});

export type PluginManifest = z.infer<typeof PluginManifestSchema>;

// ── Tool Definition ────────────────────────────────────────────────

export interface ToolDefinition {
  /** Human-readable description for the LLM. */
  description: string;
  /** JSON Schema for tool input parameters. */
  inputSchema: Record<string, unknown>;
}

/**
 * Handler function invoked when the LLM calls a tool.
 * Returns the tool's output as a string.
 */
export type ToolHandler = (
  id: string,
  params: Record<string, unknown>
) => Promise<string>;

/** A registered tool with its definition and handler. */
export interface RegisteredTool {
  name: string;
  definition: ToolDefinition;
  handler: ToolHandler;
}

// ── Plugin Context (passed to activate()) ──────────────────────────

export interface PluginLogger {
  info(message: string, ...args: unknown[]): void;
  warn(message: string, ...args: unknown[]): void;
  error(message: string, ...args: unknown[]): void;
  debug(message: string, ...args: unknown[]): void;
}

export interface PluginContext {
  /** Plugin configuration from the manifest or runtime. */
  config: Record<string, unknown>;

  /** Logger scoped to this plugin. */
  logger: PluginLogger;

  /** Workspace root directory. */
  workspace: string;

  /**
   * Register a tool that the LLM can invoke.
   *
   * @param name - Tool name (unique within the plugin).
   * @param definition - Tool description and input schema.
   * @param handler - Async function called when the tool is invoked.
   */
  registerTool(
    name: string,
    definition: ToolDefinition,
    handler: ToolHandler
  ): void;
}

// ── Plugin Module (the default export from the plugin entry point) ─

/**
 * The OpenClaw plugin module interface.
 *
 * Plugins export an object with `activate` and optionally `deactivate`
 * lifecycle hooks, plus an optional `onEvent` handler for hook events
 * from Astrid.
 */
export interface PluginModule {
  /** Called when the plugin is loaded. Register tools here. */
  activate(context: PluginContext): void | Promise<void>;

  /** Called when the plugin is being unloaded. Clean up resources. */
  deactivate?(): void | Promise<void>;

  /**
   * Called when Astrid sends a hook event notification.
   * This is a fire-and-forget callback; return value is ignored.
   */
  onEvent?(event: string, data: unknown): void | Promise<void>;
}
