import test from "node:test";
import assert from "node:assert/strict";
import { startEditorMcp } from "./editor-mcp.mjs";

async function post(server, body, headers = {}) {
    return fetch(server.config.url, { method: "POST", headers: { "Content-Type":"application/json", ...server.config.headers, ...headers }, body: JSON.stringify(body) });
}

test("private MCP transport authenticates, isolates turns, and relays structured results", async () => {
    const observed = [];
    const server = await startEditorMcp(async (request) => {
        observed.push(request);
        return { jsonrpc: "2.0", id: request.id, result: { content: [{ type:"text", text:"Norsk 🦦" }] } };
    });
    const other = await startEditorMcp(async () => assert.fail("wrong editor"));
    try {
        const body = {jsonrpc:"2.0", id:1, method:"tools/call", params:{name:"workspace_context", arguments:{}}};
        assert.equal((await post(server, body, {Authorization:"Bearer wrong"})).status, 401);
        assert.equal((await post(server, body, other.config.headers)).status, 401);
        assert.equal((await post(server, body, {Origin:"https://example.com"})).status, 403);
        assert.equal((await post(server, [body])).status, 400);
        assert.equal((await post(server, {jsonrpc:"2.0", method:"notifications/initialized"})).status, 202);
        assert.equal((await post(server, body)).status, 200);
        const result = await (await post(server, body)).json();
        assert.equal(result.id, 1);
        assert.equal(result.result.content[0].text, "Norsk 🦦");
        assert.equal(observed.length, 2);
        assert.equal((await post(server, {...body, params:{text:"x".repeat(1024*1024)}})).status, 413);
    } finally { await server.close(); await other.close(); }
    await assert.rejects(fetch(server.config.url));
});

test("MCP cancellation aborts only the matching request", async () => {
    let started;
    const ready = new Promise(resolve => { started = resolve; });
    let cancelled = false;
    const server = await startEditorMcp((_request, signal) => new Promise((_resolve, reject) => {
        signal.addEventListener("abort", () => { cancelled = true; reject(new Error("cancelled")); }, {once:true});
        started();
    }));
    try {
        const pending = post(server, {jsonrpc:"2.0", id:42, method:"tools/call", params:{}});
        await ready;
        await post(server, {jsonrpc:"2.0", method:"notifications/cancelled", params:{requestId:41}});
        assert.equal(cancelled, false);
        await post(server, {jsonrpc:"2.0", method:"notifications/cancelled", params:{requestId:42}});
        await pending;
        assert.equal(cancelled, true);
    } finally { await server.close(); }
});
