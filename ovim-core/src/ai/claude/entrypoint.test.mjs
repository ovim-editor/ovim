import test from "node:test";
import assert from "node:assert/strict";
import { mkdtemp, copyFile, writeFile, symlink, rm } from "node:fs/promises";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { spawn } from "node:child_process";

// Exercise Node's real entrypoint, including the alias used by macOS temp dirs.
// The SDK fixture cannot perform inference or read credentials.
for (const aliased of [false, true]) {
    test(`runtime starts through ${aliased ? "symlink" : "direct"} path`, async () => {
        const root = await mkdtemp(join(tmpdir(), "ovim entry 🦦 "));
        try {
            for (const file of ["runtime.mjs", "editor-mcp.mjs"])
                await copyFile(new URL(file, import.meta.url), join(root, file));
            await writeFile(join(root, "sdk.mjs"), `
                export function query({options}) {
                    return {
                        async *[Symbol.asyncIterator]() {
                            yield { type: "assistant", message: { content: [{type:"text", text:options.model}] } };
                            yield {type:"result", subtype:"success", is_error:false, session_id:"fixture-session"};
                        },
                        close() {}
                    };
                }
            `);
            const alias = join(root, "alias");
            if (aliased) await symlink(root, alias, "dir");
            const child = spawn(process.execPath, [join(aliased ? alias : root, "runtime.mjs")], {stdio: "pipe"});
            let stdout = "", stderr = "";
            child.stdout.setEncoding("utf8").on("data", chunk => stdout += chunk);
            child.stderr.setEncoding("utf8").on("data", chunk => stderr += chunk);
            const timeout = setTimeout(() => child.kill("SIGKILL"), 5000);
            child.stdin.on("error", () => {});
            const exited = new Promise((resolve, reject) => {
                child.on("error", reject);
                child.on("close", code => resolve(code));
            });
            child.stdin.write(JSON.stringify({cwd:root, executable:"unused", model:"opus", allowEdits:true, content:[{type:"text",text:"fixture"}]}) + "\n");
            const code = await exited.finally(() => clearTimeout(timeout));
            assert.equal(code, 0, stderr);
            const events = stdout.trim().split("\n").filter(Boolean).map(line => JSON.parse(line));
            assert.ok(events.some(e => e.type === "text" && e.text === "opus"), stdout);
            assert.equal(events.at(-1)?.type, "done", stdout);
        } finally {
            await rm(root, {recursive:true, force:true});
        }
    });
}
