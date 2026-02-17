// Minimal test fixture: registers a single tool that echoes the plugin config.
// Used by mcp_bridge_e2e.rs to verify config delivery via the
// notifications/astrid.setPluginConfig notification.

export default {
  register(api) {
    api.registerTool("get-config", {
      description: "Returns the current plugin config",
      inputSchema: { type: "object", properties: {} },
    }, (_name, _args) => {
      return JSON.stringify(api.runtime.config.loadConfig());
    });
  },
};
