import { expect, test, type Page } from "@playwright/test";

// Popups and the IME sink must follow the rendered cursor cell, not a
// line * LINE_HEIGHT guess. The cursor sits on the third visual row of a
// wrapped line (absolute displayColumn 130), which the guess cannot express.
const mockScript = (layout: string, focused: number) => `
mockSnapshot.mode = "INSERT";
mockSnapshot.wrap = true;
const seg = (text, cursor = false) => ({ text, cells: text.length, cursor, selected: false, searchMatch: false });
const rows = [
    { number: 1, continuation: false, displayStart: 0, current: true, segments: [seg("x".repeat(60))] },
    { number: 1, continuation: true, displayStart: 60, current: true, segments: [seg("y".repeat(60))] },
    { number: 1, continuation: true, displayStart: 120, current: true, segments: [seg("z".repeat(10)), seg("q", true), seg("z".repeat(10))] },
    ...Array.from({ length: 12 }, (_, index) => ({ number: index + 2, continuation: false, displayStart: 0, current: false, segments: [seg("line " + (index + 2))] })),
];
mockSnapshot.cursor = { line: 0, column: 130, displayColumn: 130 };
mockSnapshot.lines = rows;
mockSnapshot.firstLine = 0;
mockSnapshot.panes = [0, 1].slice(0, ${layout === '{ kind: "pane", pane: 0 }' ? 1 : 2}).map((index) => ({
    index, bufferId: index + 1, focused: index === ${focused}, fileName: "file" + index + ".rs", modified: false,
    cursor: mockSnapshot.cursor, firstLine: 0, scrollSubrow: 0, horizontalOffset: 0, totalLines: 14, lines: rows,
}));
mockSnapshot.layout = ${layout};
delete mockSnapshot.aiChat;
mockSnapshot.completion = { selected: 0, items: [{ index: 0, label: "frobnicate" }] };
mockSnapshot.signatureHelp = { before: "f(", active: "a: i32", after: ")", signatureIndex: 0, signatureCount: 1, line: 0, displayColumn: 130 };
mockSnapshot.hover = { content: "docs", line: 0, displayColumn: 130 };
`;

const layouts: Array<[string, string, number]> = [
    ["a single wrapped pane", '{ kind: "pane", pane: 0 }', 0],
    [
        "the right pane of a vertical split",
        '{ kind: "split", direction: "vertical", ratio: 0.5, first: { kind: "pane", pane: 0 }, second: { kind: "pane", pane: 1 } }',
        1,
    ],
    [
        "the lower pane of a horizontal split",
        '{ kind: "split", direction: "horizontal", ratio: 0.5, first: { kind: "pane", pane: 0 }, second: { kind: "pane", pane: 1 } }',
        1,
    ],
];

const rect = (page: Page, selector: string) =>
    page.locator(selector).evaluate((element) => {
        const { left, top, right, bottom } = element.getBoundingClientRect();
        return { left, top, right, bottom };
    });

for (const [name, layout, focused] of layouts) {
    test(`popups and IME sink anchor to the cursor cell in ${name}`, async ({
        page,
    }) => {
        await page.route("**/src/mock.ts", async (route) => {
            const response = await route.fetch();
            await route.fulfill({
                response,
                body:
                    (await response.text()) +
                    `\n${mockScript(layout, focused)}`,
            });
        });
        await page.goto("/");
        const cursorSelector = ".editor-pane.focused .code-segment.cursor";
        await expect(page.locator(cursorSelector)).toHaveText("q");
        await expect(page.locator(".completion-popover")).toBeVisible();
        await expect(page.locator(".signature-popover")).toBeVisible();
        await expect(page.locator(".hover-popover")).toBeVisible();

        const check = async () => {
            const cursor = await rect(page, cursorSelector);
            const sink = await rect(page, ".input-sink");
            const bodyRight = (await rect(page, ".editor-body")).right;
            expect(Math.abs(sink.left - cursor.left)).toBeLessThan(1);
            expect(Math.abs(sink.top - cursor.top)).toBeLessThan(1);

            // Completion and hover open adjacent to the cursor cell (below,
            // or flipped above when they do not fit), starting at its left
            // edge, and never cover it.
            for (const selector of [".completion-popover", ".hover-popover"]) {
                const popup = await rect(page, selector);
                const gap =
                    popup.top >= cursor.bottom - 1
                        ? popup.top - cursor.bottom
                        : cursor.top - popup.bottom;
                expect(gap).toBeGreaterThanOrEqual(-1);
                // Hover reserves its 340px preferred height even when its
                // content is shorter, so a flipped hover floats that far up.
                expect(gap).toBeLessThan(
                    selector === ".hover-popover" ? 340 : 16,
                );
                // Only the right edge of the editor may push it sideways.
                expect(
                    Math.abs(
                        popup.left -
                            Math.min(
                                cursor.left,
                                bodyRight - (popup.right - popup.left) - 8,
                            ),
                    ),
                ).toBeLessThan(1);
            }
            // Parameter hints sit directly above the cursor line.
            const signature = await rect(page, ".signature-popover");
            expect(signature.bottom).toBeLessThanOrEqual(cursor.top + 1);
            expect(cursor.top - signature.bottom).toBeLessThan(16);
            // The hint's box is sized by its text but clamped as if 560px wide.
            expect(
                Math.abs(
                    signature.left - Math.min(cursor.left, bodyRight - 560 - 8),
                ),
            ).toBeLessThan(1);
        };
        await check();

        // Moving the cursor cell (window resize reflows the panes) must
        // re-anchor without waiting for another snapshot.
        await page.setViewportSize({ width: 1180, height: 900 });
        await expect
            .poll(async () => {
                const cursor = await rect(page, cursorSelector);
                const sink = await rect(page, ".input-sink");
                return Math.abs(sink.left - cursor.left);
            })
            .toBeLessThan(1);
        await check();
    });
}
