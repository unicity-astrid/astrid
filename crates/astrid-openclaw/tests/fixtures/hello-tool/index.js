module.exports = {
  activate: function(context) {
    context.logger.info("Hello Tool activating");

    context.registerTool("hello", {
      description: "Say hello to someone",
      inputSchema: {
        type: "object",
        properties: {
          name: { type: "string", description: "Name to greet" }
        },
        required: ["name"]
      }
    }, function(id, params) {
      return "Hello, " + params.name + "!";
    });
  }
};
