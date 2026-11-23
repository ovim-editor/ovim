import { expect, test, type Page } from "@playwright/test";
import type { GuiDiffDocument, GuiDiffLine } from "../src/types";

const context = (count: number, start: number): GuiDiffLine[] =>
    Array.from({ length: count }, (_, index) => ({
        kind: "context",
        text: `    keep_${start + index}();`,
        oldLine: start + index,
        newLine: start + index,
    }));

const file = (
    index: number,
    lines: GuiDiffLine[],
    extra: Partial<GuiDiffDocument["files"][number]["hunks"][number]> = {},
): GuiDiffDocument["files"][number] => ({
    id: `file-${index}`,
    path: `src/file-${index}.ts`,
    status: "modified",
    additions: 1,
    deletions: 1,
    binary: false,
    metadata: [],
    hunks: [
        {
            header: "@@",
            oldStart: 1,
            oldCount: lines.length,
            newStart: 1,
            newCount: lines.length,
            lines,
            ...extra,
        },
    ],
});

const swap = (index: number): GuiDiffLine[] => [
    { kind: "removed", oldLine: 1, text: `old fragment ${index}` },
    { kind: "added", newLine: 1, text: `new fragment ${index}` },
    ...context(60, 2),
];

async function setup(page: Page, document: GuiDiffDocument) {
    await page.route("**/src/mock.ts", async (route) => {
        const response = await route.fetch();
        await route.fulfill({
            response,
            body: `${await response.text()}
                delete mockSnapshot.aiChat;
                delete mockSnapshot.fileTree;
                mockSnapshot.fileName = ${JSON.stringify(document.title)};
                mockSnapshot.filePath = undefined;
                mockSnapshot.readOnly = true;
                mockSnapshot.panes = [{...mockSnapshot.panes[0], diffReview: ${JSON.stringify(document)}}];`,
        });
    });
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
        window.diffCommands = [];
        let listener;
        export const invoke = async (command, args) => {
            window.diffCommands.push({command, args});
            if (command === 'gui_subscribe') { listener = args.onEvent; listener.onmessage(mockSnapshot); }
        };`,
            }),
    );
    await page.goto("/");
    await expect(page.locator(".flow-diff")).toBeVisible();
}

const base = (files: GuiDiffDocument["files"]): GuiDiffDocument => ({
    title: "Diff · polish",
    layout: "split",
    managed: true,
    custom: false,
    files,
});

for (const layout of ["split", "unified"] as const)
    test(`file heading stays pinned while scrolling a long diff (${layout})`, async ({
        page,
    }) => {
        const document = base(
            Array.from({ length: 3 }, (_, index) => file(index, swap(index))),
        );
        document.layout = layout;
        await setup(page, document);
        const scroller = page.locator(
            layout === "split" ? ".flow-scroll.new" : ".flow-unified-scroll",
        );
        const heading = (index: number) =>
            scroller.locator(
                `.flow-file-group[data-file-id="file-${index}"] > .flow-file-heading`,
            );
        const scrollerTop = async () => (await scroller.boundingBox())!.y;
        // Deep inside file 1: its heading rides at the top of the scroller.
        await scroller.evaluate((element) => {
            const group = element.querySelector<HTMLElement>(
                '.flow-file-group[data-file-id="file-1"]',
            )!;
            element.scrollTop =
                group.getBoundingClientRect().top -
                element.getBoundingClientRect().top +
                element.scrollTop +
                200;
        });
        await expect
            .poll(async () =>
                Math.abs(
                    (await heading(1).boundingBox())!.y - (await scrollerTop()),
                ),
            )
            .toBeLessThan(2);
        await expect(heading(1)).toBeInViewport();
        // Scrolling into file 2 hands the top slot to its heading.
        await scroller.evaluate((element) => {
            const group = element.querySelector<HTMLElement>(
                '.flow-file-group[data-file-id="file-2"]',
            )!;
            element.scrollTop =
                group.getBoundingClientRect().top -
                element.getBoundingClientRect().top +
                element.scrollTop +
                200;
        });
        await expect
            .poll(async () =>
                Math.abs(
                    (await heading(2).boundingBox())!.y - (await scrollerTop()),
                ),
            )
            .toBeLessThan(2);
        await page.screenshot({
            path: test.info().outputPath(`sticky-heading-${layout}.png`),
        });
    });

test("an inserted top line does not make every replaced pair look fully changed", async ({
    page,
}) => {
    const lines: GuiDiffLine[] = [
        { kind: "removed", oldLine: 1, text: "const alpha = compute(first);" },
        { kind: "removed", oldLine: 2, text: "const beta = compute(second);" },
        { kind: "added", newLine: 1, text: "// a brand new comment" },
        {
            kind: "added",
            newLine: 2,
            text: "const alpha = compute(first, extra);",
        },
        {
            kind: "added",
            newLine: 3,
            text: "const beta = compute(second, extra);",
        },
    ];
    await setup(page, base([file(0, lines)]));
    const added = page.locator(".flow-scroll.new .flow-code-line");
    await expect(added).toHaveCount(3);
    // The inserted comment has no counterpart, so no word highlight at all.
    await expect(added.nth(0).locator(".flow-word-change")).toHaveCount(0);
    // The edited lines highlight only what changed, not the whole line.
    const changed = await added
        .nth(1)
        .locator(".flow-word-change")
        .allTextContents();
    expect(changed.join("")).toBe(", extra");
    const removed = page.locator(".flow-scroll.old .flow-code-line");
    await expect(removed.nth(0).locator(".flow-word-change")).toHaveCount(0);
});

test("the unchanged-lines label expands its whole gap and arrows keep stepping", async ({
    page,
}) => {
    const second = file(
        1,
        [
            { kind: "removed", oldLine: 30, text: "second_old();" },
            { kind: "added", newLine: 30, text: "second_new();" },
        ],
        {
            oldStart: 30,
            newStart: 30,
            oldCount: 1,
            newCount: 1,
            context: {
                id: "hunk:second",
                before: { old: [], new: [] },
                after: { old: [], new: [] },
                canExpandUp: true,
                canExpandDown: false,
            },
        },
    );
    const first = file(
        1,
        [{ kind: "removed", oldLine: 1, text: "first_old();" }],
        { oldCount: 1, newCount: 1 },
    );
    first.hunks.push(second.hunks[0]);
    first.hunks[0].lines.push({
        kind: "added",
        newLine: 1,
        text: "first_new();",
    });
    await setup(page, base([first]));
    const old = page.locator(".flow-scroll.old");
    const anchor = old.getByText("second_old();");
    const before = (await anchor.boundingBox())!;
    await old
        .getByRole("button", { name: "Expand 28 unchanged lines" })
        .click();
    await expect
        .poll(() =>
            page.evaluate(() =>
                (
                    window as unknown as {
                        diffCommands: Array<{
                            command: string;
                            args: { action?: string };
                        }>;
                    }
                ).diffCommands
                    .filter((entry) => entry.command === "gui_diff_action")
                    .map((entry) => entry.args.action),
            ),
        )
        .toEqual(["expand_up_all:hunk:second"]);
    // Nothing moved: the native side has not replied in this mock, and the
    // click must not scroll the reader away from the code they were reading.
    expect(Math.abs((await anchor.boundingBox())!.y - before.y)).toBeLessThan(
        2,
    );
    await old.getByRole("button", { name: "Show more context above" }).click();
    await expect
        .poll(() =>
            page.evaluate(() =>
                (
                    window as unknown as {
                        diffCommands: Array<{ args: { action?: string } }>;
                    }
                ).diffCommands
                    .map((entry) => entry.args.action)
                    .filter(Boolean),
            ),
        )
        .toContain("expand_up:hunk:second");
});
