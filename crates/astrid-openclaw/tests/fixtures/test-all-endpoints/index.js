// Test plugin that exercises every Astrid host function endpoint.
//
// Each tool maps to one or more host functions:
//   test-log        -> astrid_log (all 5 levels)
//   test-config     -> astrid_get_config
//   test-kv         -> astrid_kv_set, astrid_kv_get
//   test-file-write -> astrid_write_file
//   test-file-read  -> astrid_read_file
//   test-roundtrip  -> astrid_kv_set + astrid_kv_get (verify data integrity)

module.exports = {
  activate: function(context) {
    context.logger.info("test-all-endpoints activating");

    // ── test-log: exercises all 5 log levels ──────────────────────────
    context.registerTool("test-log", {
      description: "Log at every severity level and return confirmation",
      inputSchema: {
        type: "object",
        properties: {
          message: { type: "string", description: "Message to log" }
        },
        required: ["message"]
      }
    }, function(id, params) {
      var msg = params.message || "test";
      context.logger.debug("debug: " + msg);
      context.logger.info("info: " + msg);
      context.logger.warn("warn: " + msg);
      context.logger.error("error: " + msg);
      return "logged at all levels: " + msg;
    });

    // ── test-config: reads a config key ───────────────────────────────
    context.registerTool("test-config", {
      description: "Read a config key and return its value",
      inputSchema: {
        type: "object",
        properties: {
          key: { type: "string", description: "Config key to read" }
        },
        required: ["key"]
      }
    }, function(id, params) {
      var value = context.config[params.key];
      if (value === undefined) {
        return JSON.stringify({ found: false, key: params.key, value: null });
      }
      return JSON.stringify({ found: true, key: params.key, value: value });
    });

    // ── test-kv: write then read a KV pair ────────────────────────────
    context.registerTool("test-kv", {
      description: "Set a KV pair then read it back to verify round-trip",
      inputSchema: {
        type: "object",
        properties: {
          key: { type: "string", description: "KV key" },
          value: { type: "string", description: "KV value to store" }
        },
        required: ["key", "value"]
      }
    }, function(id, params) {
      // Write via hostKvSet (injected by shim)
      hostKvSet(params.key, params.value);
      // Read back via hostKvGet (injected by shim)
      var readBack = hostKvGet(params.key);
      return JSON.stringify({
        key: params.key,
        written: params.value,
        read_back: readBack,
        match: readBack === params.value
      });
    });

    // ── test-file-write: write content to a file ──────────────────────
    context.registerTool("test-file-write", {
      description: "Write content to a file in the workspace",
      inputSchema: {
        type: "object",
        properties: {
          path: { type: "string", description: "Relative file path" },
          content: { type: "string", description: "Content to write" }
        },
        required: ["path", "content"]
      }
    }, function(id, params) {
      hostWriteFile(params.path, params.content);
      return JSON.stringify({ written: true, path: params.path });
    });

    // ── test-file-read: read content from a file ──────────────────────
    context.registerTool("test-file-read", {
      description: "Read content from a file in the workspace",
      inputSchema: {
        type: "object",
        properties: {
          path: { type: "string", description: "Relative file path" }
        },
        required: ["path"]
      }
    }, function(id, params) {
      var content = hostReadFile(params.path);
      return JSON.stringify({ path: params.path, content: content });
    });

    // ── test-roundtrip: full KV round-trip with data integrity check ──
    context.registerTool("test-roundtrip", {
      description: "Write structured data to KV, read it back, verify integrity",
      inputSchema: {
        type: "object",
        properties: {
          data: { type: "object", description: "Arbitrary JSON to round-trip" }
        },
        required: ["data"]
      }
    }, function(id, params) {
      var serialized = JSON.stringify(params.data);
      hostKvSet("roundtrip-test", serialized);
      var readBack = hostKvGet("roundtrip-test");
      var parsed = JSON.parse(readBack);
      return JSON.stringify({
        original: params.data,
        round_tripped: parsed,
        integrity: serialized === readBack
      });
    });

    context.logger.info("test-all-endpoints activated: 6 tools registered");
  }
};
