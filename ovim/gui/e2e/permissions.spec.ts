import { expect, test } from "@playwright/test";

// The native bridge fixture projects state after each command. Core tests own
// persistence and provider validation; this exercises the real rendered flow.
for (const viewport of [
    { width: 1154, height: 1054 },
    { width: 900, height: 700 },
]) {
    test(`provider permissions remain accessible at ${viewport.width}px`, async ({
        page,
    }, testInfo) => {
        await page.setViewportSize(viewport);
        await page.route(
            "**/node_modules/.vite/deps/@tauri-apps_api_event.js*",
            (route) =>
                route.fulfill({
                    contentType: "application/javascript",
                    body: "export const listen = async () => () => {};",
                }),
        );
        await page.route(
            "**/node_modules/.vite/deps/@tauri-apps_api_core.js*",
            (route) =>
                route.fulfill({
                    contentType: "application/javascript",
                    body: `
import { mockSnapshot } from '/src/mock.ts';
export const isTauri = () => true;
export class Channel {}
window.guiCommands = [];
let emit;
let revision = mockSnapshot.revision;
let chat = {
    ...mockSnapshot.aiChat,
    profile: 'claude_code', model: 'opus', externalAgent: true,
    externalQuestion: false, activity: 'idle', waiting: false,
    approval: undefined, setup: undefined, codeExplanation: undefined,
    input: '', inputCursor: 0, pendingImages: [], queuedInputs: [],
    reasoningEffort: 'high', reasoningEffortSelection: 'default',
    reasoningEffortDefault: 'high', reasoningEfforts: ['default', 'low', 'medium', 'high'],
    profiles: [
        {id:'claude_code', label:'Claude Agent', provider:'claude_code', model:'opus'},
        {id:'codex_sol', label:'Codex', provider:'codex', model:'gpt-6-sol'},
        ...Array.from({length: 12}, (_, index) => ({id:'custom-' + index, label:'Custom ' + index, provider:'ollama', model:'local-' + index})),
    ],
    permissionMode: 'auto',
    permissionModes: [
        {id:'auto', label:'Auto', description:'Let Claude classify approval requests.'},
        {id:'default', label:'Manual', description:'Ask before protected operations.'},
        {id:'acceptEdits', label:'Accept edits', description:'Approve ordinary file edits.'},
        {id:'plan', label:'Plan', description:'Explore and plan; ask before changing files.'},
        {id:'dontAsk', label:"Don't ask", description:'Deny calls that require approval.'},
        {id:'bypassPermissions', label:'Bypass permissions', description:'Skip ordinary approval prompts.'},
    ],
};
const publish = () => emit?.({...mockSnapshot, revision: ++revision, aiChat: chat});
export const invoke = async (command, args) => {
    window.guiCommands.push({command, args});
    if (command === 'gui_subscribe') { emit = args.onEvent.onmessage; publish(); }
    if (command === 'gui_select_permission_mode') { chat = {...chat, permissionMode: args.mode}; publish(); }
    if (command === 'gui_select_ai_profile' && args.profile === 'codex_sol') {
        chat = {...chat, profile: 'codex_sol', model: 'gpt-6-sol', externalAgent: false,
            permissionMode: undefined, permissionModes: []};
        publish();
    }
};`,
                }),
        );
        await page.goto("/");
        const trigger = page.getByTitle(
            "Configure AI provider, model, and run settings",
        );
        await trigger.click();
        const dialog = page.getByRole("dialog", { name: "AI run settings" });
        const bounds = await dialog.boundingBox();
        expect(bounds).not.toBeNull();
        expect(bounds!.y + bounds!.height).toBeLessThanOrEqual(viewport.height);
        const permissions = page.getByLabel("Permission mode");
        await expect(permissions).toBeVisible();
        await expect(permissions.getByRole("option")).toHaveCount(6);
        await expect(permissions).toHaveValue("auto");
        for (const name of [
            "Auto",
            "Manual",
            "Accept edits",
            "Plan",
            "Don't ask",
            "Bypass permissions",
        ]) {
            const option = permissions.getByRole("option", {
                name,
            });
            await expect(option).toBeAttached();
        }
        await permissions.focus();
        await page.screenshot({
            path: testInfo.outputPath("permissions-auto.png"),
            fullPage: true,
        });

        await permissions.selectOption("plan");
        await page.getByRole("button", { name: "Done" }).click();
        await expect(
            page.getByRole("dialog", { name: "AI run settings" }),
        ).toHaveCount(0);
        await expect(page.getByLabel("AI chat input")).toBeFocused();
        await expect
            .poll(() =>
                page.evaluate(
                    () =>
                        (window as any).guiCommands
                            .filter(
                                (entry: any) =>
                                    entry.command ===
                                    "gui_select_permission_mode",
                            )
                            .at(-1)?.args.mode,
                ),
            )
            .toBe("plan");
        await trigger.click();
        await expect(permissions).toHaveValue("plan");
        await permissions.selectOption("bypassPermissions");
        await expect
            .poll(() =>
                page.evaluate(
                    () =>
                        (window as any).guiCommands
                            .filter(
                                (entry: any) =>
                                    entry.command ===
                                    "gui_select_permission_mode",
                            )
                            .at(-1)?.args.mode,
                ),
            )
            .toBe("bypassPermissions");
        await expect(permissions).toHaveValue("bypassPermissions");
        await page.screenshot({
            path: testInfo.outputPath("permissions-bypass.png"),
            fullPage: true,
        });
        await page.getByLabel("AI provider").selectOption("codex");
        await expect(permissions).toHaveCount(0);
    });
}
