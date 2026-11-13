import { expect, test, type Page } from "@playwright/test";

// Exercise the real workbench and xterm surface. Only IPC is substituted;
// native PTY command execution and cleanup are covered by the Rust host tests.
async function attachNativeBridge(page: Page) {
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
                let snapshots, requests, nextId = 0, session;
                const state = structuredClone(mockSnapshot);
                const chat = structuredClone(state.aiChat);
                state.aiChat = undefined;
                state.mode = 'NORMAL';
                const calls = window.terminalCalls = [];
                const publish = () => { state.revision++; snapshots?.onmessage(structuredClone(state)); };
                window.setChatOpen = (open) => { state.aiChat = open ? structuredClone(chat) : undefined; state.mode = open ? 'AI_CHAT' : 'NORMAL'; publish(); };
                window.launchTerminal = (command) => requests.onmessage({command});
                window.exitTerminal = () => { const old = session; session = undefined; old.events.onmessage({type:'exit', id:old.id, code:0}); };
                export const invoke = async (command, args) => {
                    calls.push({command, id:args?.id, text:args?.command, data:args?.data});
                    if (command === 'gui_subscribe') { snapshots = args.onEvent; requests = args.onTerminalRequest; publish(); }
                    if (command === 'gui_open_ai_chat') window.setChatOpen(true);
                    if (command === 'gui_terminal_open') {
                        if (session) throw new Error('A terminal is already open');
                        session = {id:++nextId, events:args.onEvent};
                        args.onEvent.onmessage({type:'data', id:session.id, data:Array.from(new TextEncoder().encode('workspace shell ready\\r\\n$ '))});
                        return session.id;
                    }
                    if (command === 'gui_terminal_close' && session?.id === args.id) session = undefined;
                };
            `,
            }),
    );
    await page.goto("/");
    await expect
        .poll(() => page.evaluate(() => typeof (window as any).launchTerminal))
        .toBe("function");
}

for (const width of [1280, 1600]) {
    test(`closing chat leaves the terminal hidden at ${width}px`, async ({
        page,
    }, testInfo) => {
        await page.setViewportSize({ width, height: 900 });
        await attachNativeBridge(page);
        const terminal = page.getByRole("region", {
            name: "Terminal",
            exact: true,
        });
        const activity = page.getByRole("button", {
            name: "Terminal",
            exact: true,
        });
        await activity.click();
        await expect(
            terminal.getByText("running", { exact: true }),
        ).toBeVisible();
        await page.screenshot({ path: testInfo.outputPath("terminal-open.png") });
        await page
            .getByRole("button", { name: "AI chat", exact: true })
            .click();
        await expect(page.getByLabel("AI chat input")).toBeVisible();
        await expect(terminal).toBeHidden();
        await page.evaluate(() => (window as any).setChatOpen(false));
        await expect(terminal).toBeHidden();
        await expect(page.locator(".side-dock")).toBeHidden();
        await page.screenshot({ path: testInfo.outputPath("chat-closed.png") });
        await activity.click();
        await expect(terminal).toBeVisible();
        const opens = await page.evaluate(() =>
            (window as any).terminalCalls.filter(
                (call: any) => call.command === "gui_terminal_open",
            ),
        );
        expect(opens).toHaveLength(1);
        // The existing shell and xterm input survive a trip through chat.
        await terminal
            .locator(".xterm-helper-textarea")
            .pressSequentially("echo hello");
        await expect
            .poll(() =>
                page.evaluate(() =>
                    (window as any).terminalCalls
                        .filter(
                            (call: any) =>
                                call.command === "gui_terminal_write",
                        )
                        .map((call: any) => call.data)
                        .join(""),
                ),
            )
            .toBe("echo hello");
    });
}

test("closing or exiting the terminal removes it and reopening creates a fresh shell", async ({
    page,
}) => {
    await attachNativeBridge(page);
    const activity = page.getByRole("button", {
        name: "Terminal",
        exact: true,
    });
    const terminal = page.getByRole("region", {
        name: "Terminal",
        exact: true,
    });
    await activity.click();
    await expect(terminal.getByText("running", { exact: true })).toBeVisible();
    await terminal.getByRole("button", { name: "Close terminal" }).click();
    await expect(terminal).toHaveCount(0);
    await page.getByRole("button", { name: "AI chat", exact: true }).click();
    await page.evaluate(() => (window as any).setChatOpen(false));
    await expect(terminal).toHaveCount(0);
    await activity.click();
    await expect(terminal.getByText("running", { exact: true })).toBeVisible();
    await page.evaluate(() => (window as any).exitTerminal());
    await expect(terminal).toHaveCount(0);
    await expect(page.getByText("exited", { exact: true })).toHaveCount(0);
    await activity.click();
    await expect(terminal.getByText("running", { exact: true })).toBeVisible();
    await activity.click();
    await expect(terminal).toHaveCount(0);
    const calls = await page.evaluate(() => (window as any).terminalCalls);
    expect(
        calls.filter((call: any) => call.command === "gui_terminal_open"),
    ).toHaveLength(3);
    expect(
        calls
            .filter((call: any) => call.command === "gui_terminal_close")
            .map((call: any) => call.id),
    ).toEqual([1, 3]);
});

test(":term requests reveal the shell and command tails replace it in order", async ({
    page,
}) => {
    await attachNativeBridge(page);
    const terminal = page.getByRole("region", {
        name: "Terminal",
        exact: true,
    });
    await page.evaluate(() => (window as any).launchTerminal());
    await expect(terminal.getByText("running", { exact: true })).toBeVisible();
    await page.evaluate(() => (window as any).launchTerminal());
    await page.evaluate(() =>
        (window as any).launchTerminal("printf 'hello world' | cat"),
    );
    await expect
        .poll(() =>
            page.evaluate(
                () =>
                    (window as any).terminalCalls.filter(
                        (call: any) => call.command === "gui_terminal_open",
                    ).length,
            ),
        )
        .toBe(2);
    await expect(terminal.getByText("running", { exact: true })).toBeVisible();
    const calls = await page.evaluate(() =>
        (window as any).terminalCalls.filter((call: any) =>
            ["gui_terminal_open", "gui_terminal_close"].includes(call.command),
        ),
    );
    expect(calls.map((call: any) => call.command)).toEqual([
        "gui_terminal_open",
        "gui_terminal_close",
        "gui_terminal_open",
    ]);
    expect(calls[2].text).toBe("printf 'hello world' | cat");
});

test("a background shell exit leaves the active chat alone", async ({
    page,
}) => {
    await attachNativeBridge(page);
    await page.getByRole("button", { name: "Terminal", exact: true }).click();
    await expect(page.getByText("running", { exact: true })).toBeVisible();
    await page.getByRole("button", { name: "AI chat", exact: true }).click();
    await page.evaluate(() => (window as any).exitTerminal());
    await expect(
        page.getByRole("region", { name: "Terminal", exact: true }),
    ).toHaveCount(0);
    await expect(page.getByLabel("AI chat input")).toBeVisible();
});
