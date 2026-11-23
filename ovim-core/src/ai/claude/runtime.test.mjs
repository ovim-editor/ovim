import test from "node:test";
import assert from "node:assert/strict";
import { runTurn } from "./runtime.mjs";

const assistant = (content, extra = {}) => ({
    type: "assistant",
    message: { content },
    parent_tool_use_id: null,
    ...extra,
});
const text = (text) => ({ type: "text", text });
const result = {
    type: "result",
    subtype: "success",
    is_error: false,
    session_id: "session-1",
};
const delta = (text) => ({
    type: "stream_event",
    event: { type: "content_block_delta", delta: { type: "text_delta", text } },
});

async function run(
    messages,
    { request = {}, permission, answer = { allow: false } } = {},
) {
    const events = [];
    let options;
    let closed = false;
    let decision;
    const query = (input) => {
        options = input.options;
        return {
            async *[Symbol.asyncIterator]() {
                const prompts = [];
                for await (const prompt of input.prompt) prompts.push(prompt);
                assert.equal(prompts.length, 1);
                if (permission)
                    decision = await options.canUseTool(
                        permission.name,
                        permission.input,
                        {
                            signal: new AbortController().signal,
                            decisionReason: "Needs approval",
                        },
                    );
                yield* messages;
            },
            close() {
                closed = true;
            },
        };
    };
    try {
        await runTurn(
            {
                cwd: "/project",
                executable: "/bin/claude",
                model: "default",
                permissionMode: "auto",
                allowEdits: true,
                content: [text("hello")],
                ...request,
            },
            query,
            (event) => events.push(event),
            async (event) => {
                events.push(event);
                return answer;
            },
            new AbortController().signal,
        );
    } finally {
        assert.equal(closed, true, "SDK session closes on success and failure");
    }
    return { events, options, decision };
}

test("streams Unicode text once and keeps tool results in order", async () => {
    const { events } = await run([
        delta("Hei 🦦"),
        assistant([text("Hei 🦦")]),
        assistant([
            {
                type: "tool_use",
                id: "read-1",
                name: "Read",
                input: { file_path: "/project/file" },
            },
        ]),
        {
            type: "user",
            message: {
                content: [
                    {
                        type: "tool_result",
                        tool_use_id: "read-1",
                        content: "file contents",
                    },
                ],
            },
        },
        delta("Done"),
        assistant([text("Done")]),
        result,
    ]);
    assert.deepEqual(
        events
            .filter((event) => event.type === "text")
            .map((event) => event.text),
        ["Hei 🦦", "Done"],
    );
    assert.ok(
        events.findIndex((event) => event.type === "tool_start") <
            events.findIndex((event) => event.type === "tool_result"),
    );
    assert.deepEqual(events.at(-1), { type: "session", id: "session-1" });
});

test("preserves non-streamed text and excludes nested agent text", async () => {
    const { events } = await run([
        assistant([text("nested")], { parent_tool_use_id: "agent-1" }),
        assistant([text("root")]),
        result,
    ]);
    assert.deepEqual(
        events.filter((event) => event.type === "text"),
        [{ type: "text", text: "root" }],
    );
});

test("uses Claude defaults, own executable and settings without Ovim tools or credentials", async () => {
    const { options } = await run([result]);
    assert.equal(options.systemPrompt.type, "preset");
    assert.equal(options.systemPrompt.preset, "claude_code");
    assert.match(options.systemPrompt.append, /mcp__ovim__workspace_context/);
    assert.match(options.systemPrompt.append, /mcp__ovim__explain_with_codebase/);
    // Repository settings load only when core vouches for the workspace.
    assert.deepEqual(options.settingSources, ["user"]);
    assert.equal(options.pathToClaudeCodeExecutable, "/bin/claude");
    assert.equal(options.permissionMode, "auto");
    for (const name of [
        "env",
        "model",
        "tools",
        "mcpServers",
        "allowDangerouslySkipPermissions",
    ])
        assert.equal(options[name], undefined);
});

test("loads repository settings only for a trusted, editable workspace", async () => {
    const sources = async (request) =>
        (await run([result], { request })).options.settingSources;
    assert.deepEqual(await sources({ projectSettings: true }), [
        "user",
        "project",
        "local",
    ]);
    // Untrusted editable chats and missing or non-boolean flags fail closed.
    for (const projectSettings of [false, undefined, null, "true", 1])
        assert.deepEqual(await sources({ projectSettings }), ["user"]);
    // Queries never load repository settings, even for a trusted workspace.
    assert.deepEqual(
        await sources({ allowEdits: false, projectSettings: true }),
        ["user"],
    );
});

test("passes each permission mode and only enables the dangerous bypass flag for bypass", async () => {
    for (const permissionMode of [
        "default",
        "acceptEdits",
        "plan",
        "auto",
        "dontAsk",
        "bypassPermissions",
    ]) {
        const { options } = await run([result], { request: { permissionMode } });
        assert.equal(options.permissionMode, permissionMode);
        assert.equal(
            options.allowDangerouslySkipPermissions,
            permissionMode === "bypassPermissions" ? true : undefined,
        );
    }
});

test("requires an explicit resolved permission mode before starting the SDK", async () => {
    for (const permissionMode of [undefined, null, "", " ", 42]) {
        await assert.rejects(
            runTurn(
                { permissionMode },
                () => assert.fail("Invalid requests must not start Claude"),
                () => {},
                () => {},
                new AbortController().signal,
            ),
            /Missing Claude permission mode/,
        );
    }
});

test("read-only chats stay restricted even when permission checks are bypassed", async () => {
    const { options } = await run([result], {
        request: {
            allowEdits: false,
            permissionMode: "bypassPermissions",
        },
    });
    assert.deepEqual(options.tools, ["Read", "Glob", "Grep"]);
    assert.equal(options.strictMcpConfig, true);
    assert.equal(options.disallowedTools, undefined);
    assert.equal(options.allowDangerouslySkipPermissions, true);
});

test("passes explicit model, effort, and native resume", async () => {
    const { options } = await run([result], {
        request: {
            model: "claude-fable-5-1",
            effort: "medium",
            resume: "previous-session",
        },
    });
    assert.equal(options.model, "claude-fable-5-1");
    assert.equal(options.effort, "medium");
    assert.equal(options.resume, "previous-session");
});

test("denials go to Claude and approvals cannot persist permission rules", async () => {
    const permission = { name: "Bash", input: { command: "example" } };
    const denied = await run([result], { permission });
    assert.equal(denied.decision.behavior, "deny");
    const allowed = await run([result], {
        permission,
        answer: { allow: true },
    });
    assert.deepEqual(allowed.decision, {
        behavior: "allow",
        updatedInput: permission.input,
    });
});

test("question answers return as updated tool input", async () => {
    const input = { questions: [{ question: "Which color?" }] };
    const updated = { ...input, answers: { "Which color?": "Blue" } };
    const { decision } = await run([result], {
        permission: { name: "AskUserQuestion", input },
        answer: { allow: true, input: updated },
    });
    assert.deepEqual(decision.updatedInput, updated);
});

test("reports API errors even when result subtype says success", async () => {
    await assert.rejects(
        run([{ ...result, is_error: true, result: "Account access disabled" }]),
        /Account access disabled/,
    );
    await assert.rejects(
        run([
            assistant([text("Organization disabled access")], {
                error: "oauth_org_not_allowed",
            }),
        ]),
        /Organization disabled access/,
    );
});

test("rejects early EOF, missing checkpoint and malformed assistant blocks", async () => {
    await assert.rejects(
        run([assistant([text("partial")])]),
        /without completing/,
    );
    await assert.rejects(run([{ ...result, session_id: "" }]), /no session ID/);
    await assert.rejects(run([{ type: "assistant", message: {} }]));
});
