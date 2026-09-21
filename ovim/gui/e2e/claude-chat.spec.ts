import { expect, test } from "@playwright/test";

// Render the actual chat components with representative runtime projections.
// Live Claude requests are deliberately not part of browser layout tests.
for (const state of ["approval", "question"] as const) {
    test(`Claude ${state} keeps the editor chat experience`, async ({ page }, testInfo) => {
        await page.route("**/src/mock.ts", async (route) => {
            const response = await route.fetch();
            await route.fulfill({ response, body: (await response.text()) + `
Object.assign(mockSnapshot.aiChat, {
    profile: "claude_code", externalAgent: true, externalQuestion: ${state === "question"},
    profiles: [
        {id: "claude_code", label: "Claude Agent", provider: "claude_code", model: "default"},
        {id: "codex_sol", provider: "codex", model: "gpt-5.6-sol"}
    ],
    reasoningEffort: "default", reasoningEfforts: ["default", "low", "medium", "high", "xhigh", "max"],
    activity: "waiting_tool_approval", waiting: true,
    input: "", inputCursor: 0, queuedInputs: [], pendingImages: [],
    approval: ${state === "approval" ? JSON.stringify("Claude Code: Bash\nRun npm test in the project folder?\nApproval applies to this invocation only.") : "undefined"},
    messages: [{id:"claude-message", role:"assistant", content:${JSON.stringify(state === "question" ? "Which language should the examples use?\n1. Norsk\n2. English\n\nType your answer below." : "I will verify the change with the project tests.")}, model:"Claude Agent", index:0, images:[], tools:[], selected:false}],
    streaming: undefined, streamingThinking: undefined, thinkingLive: false,
    codeExplanation: undefined, setup: undefined
});` });
        });
        await page.goto("/");
        await expect(page.getByRole("button", { name: /Claude Agent.*default/i })).toBeVisible();
        await expect(page.getByRole("button", { name: /YOLO|COMPREHENSION/ })).toHaveCount(0);
        if (state === "approval") {
            await expect(page.getByRole("button", { name: "Allow once" })).toBeVisible();
            await expect(page.getByRole("button", { name: "Deny", exact: true })).toBeVisible();
            await expect(page.getByText(/Run npm test/)).toBeVisible();
        } else {
            await expect(page.getByPlaceholder("Answer Claude’s question…")).toBeVisible();
            await expect(page.getByRole("button", { name: "Send message" })).toBeVisible();
            await page.getByLabel("AI chat input").fill("Norsk 🦦");
            await expect(page.getByLabel("AI chat input")).toHaveValue("Norsk 🦦");
        }
        await page.screenshot({ path: testInfo.outputPath(`claude-${state}.png`), fullPage: true });
        await page.getByRole("button", { name: /Claude Agent.*default/i }).click();
        await expect(page.getByRole("option", { name: /codex_sol/i })).toBeVisible();
        await expect(page.getByRole("option", { name: /Claude Agent/i })).toBeVisible();
    });
}

test("Claude walkthrough uses the existing interactive reader", async ({ page }, testInfo) => {
    await page.route("**/src/mock.ts", async (route) => {
        const response = await route.fetch();
        const walkthrough = {
            answerInProgress:false, current:1, total:2,
            page:{kind:"concept", title:"How editor context reaches Claude", body:"Ovim supplies the active file and cursor. Claude can refresh that context through an editor tool."},
            discussion:{state:"navigating", questionCount:0, latestFailed:false},
        };
        await route.fulfill({response, body:(await response.text()) + `\nObject.assign(mockSnapshot.aiChat, {externalAgent:true, profile:"claude_code", waiting:true, codeExplanation:${JSON.stringify(walkthrough)}});`});
    });
    await page.goto("/");
    await expect(page.getByText("How editor context reaches Claude", {exact:true})).toBeVisible();
    await expect(page.getByRole("button", {name:/next/i})).toBeVisible();
    await page.screenshot({path:testInfo.outputPath("claude-walkthrough.png"), fullPage:true});
});
