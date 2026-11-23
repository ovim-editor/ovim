import { expect, test } from "@playwright/test";

test("closing a walkthrough shows the AI chat again beside the terminal", async ({
    page,
}) => {
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
        let listener;
        const state = structuredClone(mockSnapshot);
        state.mode = 'AI_CHAT';
        const publish = () => { state.revision++; listener?.onmessage(structuredClone(state)); };
        window.setWalkthrough = (open) => {
            state.aiChat.codeExplanation = open ? {
                current: 1, total: 1, answerInProgress: false,
                page: { kind: 'concept', title: 'Summing', body: 'Adds numbers.' },
                discussion: { state: 'navigating', questionCount: 0, exchangeTotal: 0, latestFailed: false },
            } : undefined;
            publish();
        };
        export const invoke = async (command, args) => {
            if (command === 'gui_subscribe') { listener = args.onEvent; publish(); }
            if (command === 'gui_terminal_spawn') throw new Error('no shell in tests');
        };
        `,
            }),
    );
    await page.goto("/");
    await page.getByRole("button", { name: "Terminal", exact: true }).click();
    const chatTab = page.getByRole("tab", { name: "AI chat" });
    await chatTab.click();
    await expect(chatTab).toHaveAttribute("aria-selected", "true");

    await page.evaluate(() => (window as any).setWalkthrough(true));
    await expect(page.getByText("Adds numbers.")).toBeVisible();
    await page.evaluate(() => (window as any).setWalkthrough(false));

    await expect(page.getByText("Adds numbers.")).toHaveCount(0);
    await expect(chatTab).toHaveAttribute("aria-selected", "true");
});
