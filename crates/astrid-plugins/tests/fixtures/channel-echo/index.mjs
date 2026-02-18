// Minimal test fixture: registers a channel and a tool that reports channel count.
// Used by mcp_bridge_e2e.rs to verify connector registration via the
// notifications/astrid.connectorRegistered notification.

export default {
  register(api) {
    api.registerChannel("telegram", {
      description: "Telegram connector",
      capabilities: { canReceive: true, canSend: true },
    }, (_name, _args) => { /* handler */ });

    api.registerTool("get-channels", {
      description: "Returns registered channel count",
      inputSchema: { type: "object", properties: {} },
    }, () => "1");
  },
};
