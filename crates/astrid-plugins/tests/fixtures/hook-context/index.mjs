export default {
    register(api) {
        let storedContext = "default context";

        api.on("session_start", (payload) => {
            storedContext = "Injected system identity context.";
        });

        api.registerTool("get-hook-state", {
            description: "Returns the current plugin environment context",
            inputSchema: { type: "object", properties: {} },
        }, (_name, _args) => {
            return storedContext;
        });
    },
};
