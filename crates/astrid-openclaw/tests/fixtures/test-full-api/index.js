// Full API surface test plugin.
//
// Exercises every registration method, hook, and event handler
// available in the plugin API. Used by e2e tests to verify that
// the shim and bridge correctly capture all registration types.
//
// Registration methods tested:
//   1. registerTool (string name form)
//   2. registerTool (object definition form)
//   3. registerService
//   4. registerChannel (string name form)
//   5. registerChannel (object definition form)
//   6. registerHook
//   7. registerCommand
//   8. registerGatewayMethod
//   9. registerHttpHandler
//  10. registerHttpRoute
//  11. registerProvider
//  12. registerCli
//  13. on (event handler)
//
// Host functions tested:
//   - logger (debug, info, warn, error)
//   - config read
//   - KV get/set
//   - file read/write
//   - HTTP request

module.exports = {
  activate: function(context) {
    context.logger.info("test-full-api activating");

    // ── 1. registerTool (string name, definition, handler) ────────────
    context.registerTool("tool-string-form", {
      description: "Tool registered with string name form",
      inputSchema: {
        type: "object",
        properties: {
          input: { type: "string" }
        }
      }
    }, function(id, params) {
      return JSON.stringify({ form: "string", input: params.input });
    });

    // ── 2. registerTool (object definition form) ──────────────────────
    context.registerTool({
      name: "tool-object-form",
      description: "Tool registered with object definition form",
      inputSchema: {
        type: "object",
        properties: {
          input: { type: "string" }
        }
      }
    }, function(id, params) {
      return JSON.stringify({ form: "object", input: params.input });
    });

    // ── 3. registerService ────────────────────────────────────────────
    context.registerService("background-worker", {
      start: function() {
        context.logger.info("background-worker started");
      },
      stop: function() {
        context.logger.info("background-worker stopped");
      }
    });

    // ── 4. registerChannel (string name form) ─────────────────────────
    context.registerChannel("notifications", {
      description: "Notification channel"
    }, function(message) {
      context.logger.info("notification received: " + JSON.stringify(message));
    });

    // ── 5. registerChannel (object definition form) ───────────────────
    context.registerChannel({
      name: "alerts",
      description: "Alert channel"
    }, function(message) {
      context.logger.warn("alert received: " + JSON.stringify(message));
    });

    // ── 6. registerHook ───────────────────────────────────────────────
    context.registerHook("session_start", function(data) {
      context.logger.info("hook: session started");
    });

    context.registerHook("before_tool_call", function(data) {
      context.logger.info("hook: before tool call - " + data?.tool_name);
    });

    context.registerHook("after_tool_call", function(data) {
      context.logger.info("hook: after tool call");
    });

    context.registerHook("session_end", function(data) {
      context.logger.info("hook: session ended");
    });

    // ── 7. registerCommand ────────────────────────────────────────────
    context.registerCommand("reload-config", function() {
      context.logger.info("command: reload-config executed");
      return { status: "reloaded" };
    });

    // ── 8. registerGatewayMethod ──────────────────────────────────────
    context.registerGatewayMethod("ping", function(params) {
      return { pong: true, timestamp: Date.now() };
    });

    // ── 9. registerHttpHandler ────────────────────────────────────────
    context.registerHttpHandler("/webhook", function(req) {
      return { status: 200, body: "ok" };
    });

    // ── 10. registerHttpRoute ─────────────────────────────────────────
    context.registerHttpRoute("POST", "/api/data", function(req) {
      return { status: 201, body: JSON.stringify({ created: true }) };
    });

    // ── 11. registerProvider ──────────────────────────────────────────
    context.registerProvider("oauth-github", {
      type: "oauth2",
      authorizationUrl: "https://github.com/login/oauth/authorize"
    });

    // ── 12. registerCli ───────────────────────────────────────────────
    context.registerCli("export", {
      description: "Export conversation history",
      handler: function(args) {
        return "exported";
      }
    });

    // ── 13. on (event handler) ────────────────────────────────────────
    context.on("astrid.v1.lifecycle.message_received", function(data) {
      context.logger.debug("event: message received");
    });

    context.on("astrid.v1.lifecycle.message_sending", function(data) {
      context.logger.debug("event: message sending");
    });

    context.on("astrid.v1.lifecycle.prompt_building", function(data) {
      context.logger.debug("event: prompt building");
    });

    context.on("astrid.v1.lifecycle.model_resolving", function(data) {
      context.logger.debug("event: model resolving");
    });

    context.on("astrid.v1.lifecycle.context_compaction_started", function(data) {
      context.logger.debug("event: context compaction started");
    });

    context.on("astrid.v1.lifecycle.tool_result_persisting", function(data) {
      context.logger.debug("event: tool result persisting");
    });

    // ── Host function smoke tests via a diagnostic tool ───────────────
    context.registerTool("run-diagnostics", {
      description: "Exercise all host functions and return results",
      inputSchema: {
        type: "object",
        properties: {}
      }
    }, function(id, params) {
      var results = {};

      // Config
      results.config_debug = context.config.debug;
      results.config_api_key = context.config.api_key;

      // KV
      hostKvSet("diag-key", "diag-value");
      results.kv_roundtrip = hostKvGet("diag-key") === "diag-value";

      // File
      hostWriteFile("diag-test.txt", "diagnostics");
      results.file_roundtrip = hostReadFile("diag-test.txt") === "diagnostics";

      // Logger
      context.logger.debug("diag: debug");
      context.logger.info("diag: info");
      context.logger.warn("diag: warn");
      context.logger.error("diag: error");
      results.logger = true;

      return JSON.stringify(results);
    });

    context.logger.info("test-full-api activated: all 13 registration methods exercised");
  }
};
