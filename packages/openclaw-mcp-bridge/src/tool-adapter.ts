/**
 * Adapts OpenClaw tool execute() → MCP CallToolResult.
 *
 * Bridges the gap between the OpenClaw plugin tool handler signature
 * and the MCP SDK's expected tool call result format.
 */

import type { CallToolResult } from "@modelcontextprotocol/sdk/types.js";
import type { RegisteredTool } from "./types.js";

/**
 * Execute a registered OpenClaw tool and convert the result to an
 * MCP CallToolResult.
 *
 * @param tool - The registered tool to execute.
 * @param args - Arguments from the MCP tool call request.
 * @returns MCP-compatible tool call result.
 */
export async function executeToolCall(
  tool: RegisteredTool,
  args: Record<string, unknown>
): Promise<CallToolResult> {
  try {
    const result = await tool.handler(tool.name, args);

    return {
      content: [
        {
          type: "text" as const,
          text: typeof result === "string" ? result : JSON.stringify(result),
        },
      ],
      isError: false,
    };
  } catch (error) {
    const message =
      error instanceof Error ? error.message : String(error);

    return {
      content: [
        {
          type: "text" as const,
          text: `Tool execution failed: ${message}`,
        },
      ],
      isError: true,
    };
  }
}
