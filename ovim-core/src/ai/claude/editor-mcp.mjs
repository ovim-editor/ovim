import { createServer } from "node:http";
import { randomBytes, timingSafeEqual } from "node:crypto";

const MAX_BODY_BYTES = 1024 * 1024;

/** Stateless Streamable HTTP MCP transport, private to one provider turn.
 * The shared Rust MCP contracts and editor tool registry own the protocol and
 * operation semantics. This layer only authenticates and relays requests.
 */
export async function startEditorMcp(dispatch) {
    const token = randomBytes(32).toString("hex");
    const authorization = Buffer.from(`Bearer ${token}`);
    const active = new Set();
    const requests = new Map();
    const server = createServer(async (req, res) => {
        const supplied = Buffer.from(req.headers.authorization || "");
        if (supplied.length !== authorization.length || !timingSafeEqual(supplied, authorization)) {
            res.writeHead(401).end();
            return;
        }
        // No browser origins are needed by the local SDK client.
        if (req.headers.origin || req.url !== "/mcp") {
            res.writeHead(403).end();
            return;
        }
        if (req.method !== "POST") {
            res.writeHead(405, { Allow: "POST" }).end();
            return;
        }
        if (!req.headers["content-type"]?.startsWith("application/json")) {
            res.writeHead(415).end();
            return;
        }
        const controller = new AbortController();
        active.add(controller);
        res.on("close", () => controller.abort());
        try {
            let length = 0;
            const chunks = [];
            for await (const chunk of req) {
                length += chunk.length;
                if (length > MAX_BODY_BYTES) { res.writeHead(413).end(); return; }
                chunks.push(chunk);
            }
            const message = JSON.parse(Buffer.concat(chunks).toString("utf8"));
            if (!message || message.jsonrpc !== "2.0" || typeof message.method !== "string" || Array.isArray(message)) {
                res.writeHead(400).end();
                return;
            }
            if (message.id === undefined) {
                if (message.method === "notifications/cancelled") {
                    requests.get(JSON.stringify(message.params?.requestId))?.abort();
                }
                res.writeHead(202).end();
                return;
            }
            const key = JSON.stringify(message.id);
            if (requests.has(key)) { res.writeHead(409).end(); return; }
            requests.set(key, controller);
            let result;
            try { result = await dispatch(message, controller.signal); }
            finally { requests.delete(key); }
            if (!res.destroyed) {
                res.writeHead(200, { "Content-Type": "application/json" });
                res.end(JSON.stringify(result));
            }
        } catch {
            if (!res.destroyed) res.writeHead(400).end();
        } finally {
            active.delete(controller);
        }
    });
    server.requestTimeout = 10_000; // Bounds receiving a body, not user walkthrough time.
    server.headersTimeout = 10_000;
    await new Promise((resolve, reject) => {
        server.once("error", reject);
        server.listen(0, "127.0.0.1", resolve);
    });
    return {
        config: { type: "http", timeout: 24 * 60 * 60 * 1000, url: `http://127.0.0.1:${server.address().port}/mcp`, headers: { Authorization: `Bearer ${token}` } },
        async close() {
            for (const controller of active) controller.abort();
            server.closeAllConnections();
            await new Promise((resolve) => server.close(resolve));
        },
    };
}
