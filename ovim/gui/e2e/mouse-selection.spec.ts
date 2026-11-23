import { expect, test, type Page } from "@playwright/test";

// A recording native bridge: real browser mouse input goes through the real
// rendered editor, and the `gui_mouse` commands that Ovim core would receive
// are checked. Selection semantics are tested in Rust (ovim-core, gui::tests).
async function open(page: Page) {
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
export const invoke = async (command, args) => {
    window.guiCommands.push({ command, args });
    if (command === 'gui_subscribe') args.onEvent.onmessage(mockSnapshot);
};`,
            }),
    );
    await page.goto("/");
    await expect(page.locator(".code-line").first()).toBeVisible();
    await page.evaluate(() => document.fonts.ready);
}

const commands = (page: Page, name: string) =>
    page.evaluate(
        (command) =>
            (window as any).guiCommands
                .filter((entry: any) => entry.command === command)
                .map((entry: any) => entry.args),
        name,
    );

// Centre of display column `column` on the nth rendered row.
async function cellPoint(page: Page, row: number, column: number) {
    return page
        .locator(".code-line")
        .nth(row)
        .locator(".line-content")
        .evaluate((element, wanted) => {
            const box = element.getBoundingClientRect();
            const probe = document.createElement("span");
            probe.textContent = "0".repeat(10);
            probe.style.cssText =
                "position:absolute;visibility:hidden;white-space:pre";
            element.append(probe);
            const cell = probe.getBoundingClientRect().width / 10;
            probe.remove();
            return {
                x: box.left + (wanted + 0.5) * cell,
                y: box.top + box.height / 2,
            };
        }, column);
}

test("press, drag and release stream one gui_mouse gesture with the cell under the pointer", async ({
    page,
}) => {
    await open(page);
    const start = await cellPoint(page, 1, 2);
    const end = await cellPoint(page, 3, 6);
    await page.mouse.move(start.x, start.y);
    await page.mouse.down();
    await page.mouse.move((start.x + end.x) / 2, (start.y + end.y) / 2, {
        steps: 4,
    });
    await page.mouse.move(end.x, end.y, { steps: 4 });
    await page.mouse.up();

    await expect
        .poll(async () => (await commands(page, "gui_mouse")).at(-1)?.kind)
        .toBe("up");
    const sent = await commands(page, "gui_mouse");
    expect(sent[0]).toMatchObject({
        pane: 0,
        line: 1,
        displayColumn: 2,
        kind: "down",
        clickCount: 1,
        shift: false,
    });
    expect(sent.slice(1, -1).every((m: any) => m.kind === "drag")).toBe(true);
    expect(sent.at(-1)).toMatchObject({
        line: 3,
        displayColumn: 6,
        kind: "up",
    });
    // Drags are throttled per frame, so none repeats the same cell.
    const cells = sent
        .slice(1, -1)
        .map((m: any) => `${m.line}:${m.displayColumn}`);
    expect(new Set(cells).size).toBe(cells.length);
    expect(await commands(page, "gui_set_cursor")).toEqual([]);
});

test("double-click reports its click count and shift is forwarded", async ({
    page,
}) => {
    await open(page);
    const point = await cellPoint(page, 2, 4);
    await page.mouse.dblclick(point.x, point.y);
    await expect
        .poll(async () =>
            (await commands(page, "gui_mouse"))
                .filter((m: any) => m.kind === "down")
                .map((m: any) => m.clickCount),
        )
        .toEqual([1, 2]);
    await page.keyboard.down("Shift");
    await page.mouse.click(point.x + 40, point.y);
    await page.keyboard.up("Shift");
    await expect
        .poll(async () => (await commands(page, "gui_mouse")).at(-1)?.shift)
        .toBe(true);
});

test("a drag that leaves the window keeps following the pointer and releases outside", async ({
    page,
}) => {
    await open(page);
    const start = await cellPoint(page, 2, 3);
    await page.mouse.move(start.x, start.y);
    await page.mouse.down();
    await page.mouse.move(start.x, 4, { steps: 6 });
    await page.mouse.move(start.x + 20, 1, { steps: 2 });
    // While the button is held outside, the selection follows the pointer.
    // Above the first row the cell clamps to row 0.
    await expect
        .poll(async () =>
            (await commands(page, "gui_mouse"))
                .filter((m: any) => m.kind === "drag")
                .map((m: any) => m.line),
        )
        .toContain(0);
    await page.mouse.up();
    await expect
        .poll(async () => (await commands(page, "gui_mouse")).at(-1))
        .toMatchObject({ kind: "up", line: 0 });
});

test("right and middle buttons do not move the cursor or select", async ({
    page,
}) => {
    await open(page);
    const point = await cellPoint(page, 1, 3);
    await page.mouse.click(point.x, point.y, { button: "right" });
    await page.mouse.click(point.x, point.y, { button: "middle" });
    await page.mouse.move(point.x + 30, point.y);
    expect(await commands(page, "gui_mouse")).toEqual([]);
    expect(await commands(page, "gui_set_cursor")).toEqual([]);
});
