import { expect, test } from "@playwright/test";

test("code lenses hover, focus and activate separately without selecting text", async ({
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
window.guiCommands = [];
delete mockSnapshot.aiChat;
mockSnapshot.panes[0].lines[1].codeLenses = [
    { generation: 1, index: 0, title: '▶ Run', mode: 'run', filePath: '/project/Main.java', version: 7 },
    { generation: 1, index: 0, title: '▶ Debug', mode: 'debug', filePath: '/project/Main.java', version: 7 },
    { generation: 1, index: 1, title: '2 implementations', mode: 'run', filePath: '/project/Main.java', version: 7 },
];
export const invoke = async (command, args) => {
    window.guiCommands.push({ command, args });
    if (command === 'gui_subscribe') args.onEvent.onmessage(mockSnapshot);
};`,
            }),
    );
    await page.goto("/");
    const run = page.getByRole("button", { name: "▶ Run", exact: true });
    const debug = page.getByRole("button", { name: "▶ Debug", exact: true });
    await expect(run).toBeVisible();
    const normalColor = await run.evaluate(
        (element) => getComputedStyle(element).color,
    );
    await run.hover();
    await expect(run).toHaveCSS("cursor", "pointer");
    await expect(run).toHaveCSS("text-decoration-line", "underline");
    expect(
        await run.evaluate((element) => getComputedStyle(element).color),
    ).not.toBe(normalColor);
    await run.click();
    await debug.click();
    const other = page.getByRole("button", {
        name: "2 implementations",
        exact: true,
    });
    await other.focus();
    await expect(other).toBeFocused();
    await page.keyboard.press("Enter");
    await expect
        .poll(() =>
            page.evaluate(
                () =>
                    (window as any).guiCommands.filter(
                        (entry: any) => entry.command === "gui_code_lens",
                    ).length,
            ),
        )
        .toBe(3);
    const messages = await page.evaluate(() => (window as any).guiCommands);
    expect(
        messages
            .filter((entry: any) => entry.command === "gui_code_lens")
            .map((entry: any) => entry.args),
    ).toMatchObject([
        {
            pane: 0,
            index: 0,
            mode: "run",
            filePath: "/project/Main.java",
            version: 7,
        },
        {
            pane: 0,
            index: 0,
            mode: "debug",
            filePath: "/project/Main.java",
            version: 7,
        },
        {
            pane: 0,
            index: 1,
            mode: "run",
            filePath: "/project/Main.java",
            version: 7,
        },
    ]);
    expect(
        messages.filter((entry: any) => entry.command === "gui_mouse"),
    ).toEqual([]);
});
