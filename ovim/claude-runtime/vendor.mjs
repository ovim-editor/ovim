// Preserve the official SDK byte-for-byte. No CLI binary is bundled or modified.
import { readFileSync, writeFileSync } from "node:fs";
import { gzipSync } from "node:zlib";
import { createHash } from "node:crypto";
const sdk = readFileSync(new URL("node_modules/@anthropic-ai/claude-agent-sdk/sdk.mjs", import.meta.url));
writeFileSync(new URL("../../ovim-core/src/ai/claude/sdk.mjs.gz", import.meta.url), gzipSync(sdk, { level: 9 }));
console.log("SDK SHA-256:", createHash("sha256").update(sdk).digest("hex"));
