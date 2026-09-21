import { startEditorMcp } from "./editor-mcp.mjs";
import { createInterface } from "node:readline";
import { fileURLToPath } from "node:url";
import { realpathSync } from "node:fs";

/** Translate SDK events, never execute tools or call Anthropic APIs here. */
export async function runTurn(request, query, emit, ask, signal) {
    // Claude can ask for permissions for parallel tools. Present one question
    // at a time while preserving all callbacks and their cancellation signals.
    let permissionTail = Promise.resolve();
    const options = {
        cwd: request.cwd,
        pathToClaudeCodeExecutable: request.executable,
        systemPrompt: { type: "preset", preset: "claude_code", append:
            "You are assisting the user in Ovim. Editor snapshots are context at the time supplied, and unsaved buffer content can differ from disk. Use mcp__ovim__workspace_context to refresh the current file, cursor, selection and diagnostics when needed. Use mcp__ovim__open_file to show relevant existing project code to the user. Use mcp__ovim__explain_with_codebase for a finished, focused interactive walkthrough; follow its page guidance and wait for completion or dismissal. These editor tools do not grant filesystem writes or access outside the current workspace. Continue to use Claude Code's normal tools and permission rules for implementation."
        },
        ...(request.editorMcp ? { mcpServers: { ovim: request.editorMcp } } : {}),
        settingSources: ["user", "project", "local"],
        permissionMode: "default",
        includePartialMessages: true,
        abortController: request.abortController,
        ...(request.model !== "default" ? { model: request.model } : {}),
        ...(request.effort ? { effort: request.effort } : {}),
        ...(request.resume ? { resume: request.resume } : {}),
        // Query mode is genuinely read-only, including configured MCP tools.
        ...(!request.allowEdits
            ? {
                  tools: ["Read", "Glob", "Grep"],
                  // Only our navigation/context server is available in a query.
                  // User-configured MCP servers may have write tools.
                  strictMcpConfig: true,
              }
            : {}),
        canUseTool: async (name, input, context) => {
            const predecessor = permissionTail;
            let release;
            permissionTail = new Promise((resolve) => {
                release = resolve;
            });
            await predecessor;
            try {
                if (context.signal.aborted)
                    return { behavior: "deny", message: "Cancelled" };
                const answer = await ask(
                    {
                        type: "permission",
                        name,
                        input,
                        reason:
                            context.title ||
                            context.decisionReason ||
                            "Claude Code requests permission",
                    },
                    context.signal,
                );
                if (!answer.allow)
                    return { behavior: "deny", message: "Denied by the user" };
                // Do not turn Ovim's 'allow chat' shortcut into persistent Claude
                // permission changes. Every response applies to this call only.
                return {
                    behavior: "allow",
                    updatedInput: answer.input || input,
                };
            } finally {
                release();
            }
        },
    };
    async function* prompt() {
        yield {
            type: "user",
            session_id: "",
            parent_tool_use_id: null,
            message: { role: "user", content: request.content },
        };
    }
    const session = query({ prompt: prompt(), options });
    let streamedText = false;
    let streamedThinking = false;
    let completed = false;
    try {
        for await (const message of session) {
            if (signal.aborted) throw new Error("Cancelled");
            // Subagents have their own message streams; their parent tool result
            // presents their outcome without mixing their text into the root reply.
            if (message.parent_tool_use_id) continue;
            if (message.type === "stream_event") {
                const event = message.event;
                if (event.type === "content_block_delta") {
                    if (event.delta.type === "text_delta") {
                        streamedText = true;
                        emit({ type: "text", text: event.delta.text });
                    } else if (event.delta.type === "thinking_delta") {
                        streamedThinking = true;
                        emit({ type: "thinking", text: event.delta.thinking });
                    }
                }
            } else if (message.type === "assistant") {
                if (message.error) {
                    const detail = message.message.content
                        .filter((block) => block.type === "text")
                        .map((block) => block.text)
                        .join("\n");
                    throw new Error(
                        `Claude Code: ${message.error}${detail ? ` — ${detail}` : ""}`,
                    );
                }
                for (const block of message.message.content) {
                    if (block.type === "text" && !streamedText)
                        emit({ type: "text", text: block.text });
                    if (block.type === "thinking" && !streamedThinking)
                        emit({ type: "thinking", text: block.thinking });
                    if (block.type === "tool_use") {
                        emit({ type: "message_end" });
                        emit({
                            type: "tool_start",
                            id: block.id,
                            name: block.name,
                            input: block.input,
                        });
                    }
                }
                emit({ type: "message_end" });
                streamedText = streamedThinking = false;
            } else if (
                message.type === "user" &&
                Array.isArray(message.message.content)
            ) {
                for (const block of message.message.content) {
                    if (block.type !== "tool_result") continue;
                    const content =
                        typeof block.content === "string"
                            ? block.content
                            : JSON.stringify(block.content ?? "");
                    emit({
                        type: "tool_result",
                        id: block.tool_use_id,
                        content,
                        error: Boolean(block.is_error),
                    });
                }
            } else if (message.type === "result") {
                if (message.is_error || message.subtype !== "success") {
                    throw new Error(
                        message.errors?.join("\n") ||
                            message.result ||
                            `Claude Code: ${message.subtype}`,
                    );
                }
                if (!message.session_id)
                    throw new Error("Claude Code returned no session ID");
                emit({ type: "session", id: message.session_id });
                completed = true;
                break;
            }
        }
        if (!completed)
            throw new Error("Claude Code exited without completing the turn");
    } finally {
        session.close();
    }
}

async function main() {
    const lines = createInterface({
        input: process.stdin,
        crlfDelay: Infinity,
    });
    const controller = new AbortController();
    const pending = new Map();
    let editorMcp;
    let nextId = 0;
    let started = false;
    const emit = (event) => process.stdout.write(JSON.stringify(event) + "\n");
    const ask = (event, signal) =>
        new Promise((resolve) => {
            if (signal.aborted) return resolve({ allow: false });
            const id = String(++nextId);
            const abort = () => {
                emit({ type: "permission_cancelled" });
                resolve({ allow: false });
            };
            signal.addEventListener("abort", abort, { once: true });
            pending.set(id, (answer) => {
                signal.removeEventListener("abort", abort);
                resolve(answer);
            });
            emit({ ...event, id });
        });
    lines.on("close", () => controller.abort());
    process.on("SIGTERM", () => controller.abort());
    lines.on("line", async (line) => {
        try {
            const input = JSON.parse(line);
            if (started) {
                if (
                    typeof input.id !== "string" ||
                    (typeof input.allow !== "boolean" && !Object.hasOwn(input, "result"))
                )
                    throw new Error("Invalid permission response");
                const respond = pending.get(input.id);
                if (!respond) throw new Error("Unknown permission response");
                pending.delete(input.id);
                respond(input);
                return;
            }
            started = true;
            const { query } = await import("./sdk.mjs");
            editorMcp = await startEditorMcp((request, signal) => new Promise((resolve, reject) => {
                const id = String(++nextId);
                const abort = () => {
                    emit({type: "editor_cancelled", id});
                    reject(new Error("Editor request cancelled"));
                };
                signal.addEventListener("abort", abort, { once: true });
                pending.set(id, (answer) => {
                    signal.removeEventListener("abort", abort);
                    resolve(answer.result);
                });
                emit({type: "editor_request", id, request});
            }));
            await runTurn(
                { ...input, abortController: controller, editorMcp: editorMcp.config },
                query,
                emit,
                ask,
                controller.signal,
            );
            await editorMcp.close();
            emit({ type: "done" });
            lines.close();
            process.stdin.destroy();
        } catch (error) {
            controller.abort();
            if (editorMcp) await editorMcp.close();
            emit({
                type: "error",
                message: error instanceof Error ? error.message : String(error),
            });
            lines.close();
            process.stdin.destroy();
            process.exitCode = 1;
        }
    });
}

// Node resolves the module path, but argv retains aliases such as macOS's
// /var → /private/var. Compare filesystem identities before deciding to run.
if (process.argv[1] && realpathSync(fileURLToPath(import.meta.url)) === realpathSync(process.argv[1]))
    await main();
