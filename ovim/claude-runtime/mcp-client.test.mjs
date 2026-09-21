// Exercise the private endpoint with an independent, official MCP client.
// This does not start Claude Code or make inference requests.
import test from "node:test";
import assert from "node:assert/strict";
import { Client } from "@modelcontextprotocol/sdk/client/index.js";
import { StreamableHTTPClientTransport } from "@modelcontextprotocol/sdk/client/streamableHttp.js";
import { startEditorMcp } from "../../ovim-core/src/ai/claude/editor-mcp.mjs";

test("official MCP client completes handshake, discovers tools and receives results", async () => {
    const calls = [];
    const server = await startEditorMcp(async (request) => {
        calls.push(request.method);
        let result;
        if (request.method === "initialize") result = {protocolVersion:"2025-03-26", capabilities:{tools:{}}, serverInfo:{name:"ovim-editor", version:"1"}};
        else if (request.method === "tools/list") result = {tools:[{name:"workspace_context", description:"Read the editor state", inputSchema:{type:"object", additionalProperties:false}}]};
        else if (request.method === "tools/call") result = {content:[{type:"text", text:"Editing main.rs · Norsk 🦦"}], isError:false};
        else throw new Error(request.method);
        return {jsonrpc:"2.0", id:request.id, result};
    });
    const client = new Client({name:"ovim-qa", version:"1"});
    try {
        await client.connect(new StreamableHTTPClientTransport(new URL(server.config.url), {requestInit:{headers:server.config.headers}}));
        assert.equal((await client.listTools()).tools[0].name, "workspace_context");
        const result = await client.callTool({name:"workspace_context", arguments:{}});
        assert.equal(result.isError, false);
        assert.match(result.content[0].text, /Norsk 🦦/);
        assert.deepEqual(calls, ["initialize", "tools/list", "tools/call"]);
    } finally { await client.close(); await server.close(); }
});
