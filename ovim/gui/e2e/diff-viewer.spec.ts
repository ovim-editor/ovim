import { readFile } from "node:fs/promises";
import { unzlibSync } from "fflate";
import { expect, test, type Page } from "@playwright/test";
import type { GuiDiffDocument, GuiDiffLine } from "../src/types";

const context = (
    count: number,
    oldStart: number,
    newStart: number,
): GuiDiffLine[] =>
    Array.from({ length: count }, (_, index) => ({
        kind: "context",
        text: `    processRequest(request, ${index});`,
        oldLine: oldStart + index,
        newLine: newStart + index,
        reviewLine: oldStart + index,
    }));

const review: GuiDiffDocument = {
    title: "Diff · main → payment-api",
    layout: "split",
    managed: true,
    custom: false,
    files: [
        {
            id: "src/payments/PaymentResource.java",
            path: "src/payments/PaymentResource.java",
            status: "modified",
            additions: 5,
            deletions: 24,
            binary: false,
            metadata: [],
            hunks: [
                {
                    header: "@@ -1200,104 +1200,85 @@ public Response acceptPayment()",
                    oldStart: 1200,
                    oldCount: 104,
                    newStart: 1200,
                    newCount: 85,
                    reviewLine: 2,
                    lines: [
                        ...context(4, 1200, 1200),
                        ...Array.from(
                            { length: 24 },
                            (_, index): GuiDiffLine => ({
                                kind: "removed",
                                text: `    paymentService.validateLegacyPayment(request, ${index});`,
                                oldLine: 1204 + index,
                                reviewLine: 6 + index,
                            }),
                        ),
                        ...Array.from(
                            { length: 5 },
                            (_, index): GuiDiffLine => ({
                                kind: "added",
                                text: `    paymentService.acceptPayment(request, ${index});`,
                                newLine: 1204 + index,
                                reviewLine: 30 + index,
                            }),
                        ),
                        ...context(76, 1228, 1209),
                    ],
                },
            ],
        },
        {
            id: "assets/receipt.png",
            path: "assets/receipt.png",
            status: "modified",
            additions: 0,
            deletions: 0,
            binary: true,
            metadata: [],
            hunks: [],
        },
    ],
};

async function setup(page: Page, document: GuiDiffDocument = review) {
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
    await page.goto("/");
    await expect(page.locator(".flow-diff")).toBeVisible();
}

const movedReview: GuiDiffDocument = {
    title: "Diff · Extract parser",
    layout: "split",
    managed: true,
    custom: true,
    files: [
        {
            id: "src/parser.ts",
            path: "src/parser.ts",
            status: "modified",
            additions: 3,
            deletions: 0,
            binary: false,
            metadata: [],
            hunks: [
                {
                    header: "@@ -1,4 +1,7 @@",
                    oldStart: 1,
                    oldCount: 4,
                    newStart: 1,
                    newCount: 7,
                    lines: [
                        {
                            kind: "added",
                            text: 'import { parseTokens } from "./tokens";',
                            newLine: 1,
                        },
                        {
                            kind: "context",
                            text: "// Parser helpers",
                            oldLine: 1,
                            newLine: 2,
                        },
                        { kind: "context", text: "", oldLine: 2, newLine: 3 },
                        {
                            kind: "added",
                            text: "export function parse(input: string) {",
                            newLine: 4,
                        },
                        {
                            kind: "added",
                            text: "  return parseTokens(input); }",
                            newLine: 5,
                        },
                        { kind: "context", text: "", oldLine: 3, newLine: 6 },
                        {
                            kind: "context",
                            text: "export const version = 2;",
                            oldLine: 4,
                            newLine: 7,
                        },
                    ],
                },
            ],
        },
        {
            id: "src/main.ts",
            path: "src/main.ts",
            status: "modified",
            additions: 0,
            deletions: 3,
            binary: false,
            metadata: [],
            hunks: [
                {
                    header: "@@ -42,7 +42,4 @@",
                    oldStart: 42,
                    oldCount: 7,
                    newStart: 42,
                    newCount: 4,
                    lines: [
                        ...context(2, 42, 42),
                        {
                            kind: "removed",
                            text: "function parse(input: string) {",
                            oldLine: 44,
                        },
                        {
                            kind: "removed",
                            text: "  return legacyParse(input);",
                            oldLine: 45,
                        },
                        { kind: "removed", text: "}", oldLine: 46 },
                        ...context(2, 47, 44),
                    ],
                },
            ],
        },
    ],
    guidedFiles: [
        {
            id: "pair_0",
            label: "Parser moved and simplified",
            path: "src/parser.ts",
            oldPath: "src/main.ts",
            status: "reassigned",
            additions: 2,
            deletions: 3,
            binary: false,
            metadata: [],
            hunks: [
                {
                    header: "Parser moved and simplified",
                    oldStart: 44,
                    oldCount: 3,
                    newStart: 4,
                    newCount: 2,
                    lines: [
                        {
                            kind: "removed",
                            text: "function parse(input: string) {",
                            oldLine: 44,
                        },
                        {
                            kind: "removed",
                            text: "  return legacyParse(input);",
                            oldLine: 45,
                        },
                        { kind: "removed", text: "}", oldLine: 46 },
                        {
                            kind: "added",
                            text: "export function parse(input: string) {",
                            newLine: 4,
                        },
                        {
                            kind: "added",
                            text: "  return parseTokens(input); }",
                            newLine: 5,
                        },
                    ],
                },
            ],
        },
        {
            id: "residual_0",
            path: "src/parser.ts",
            status: "modified",
            additions: 1,
            deletions: 0,
            binary: false,
            metadata: [],
            hunks: [
                {
                    header: "Remaining changes",
                    oldStart: 1,
                    oldCount: 0,
                    newStart: 1,
                    newCount: 1,
                    lines: [
                        {
                            kind: "added",
                            text: 'import { parseTokens } from "./tokens";',
                            newLine: 1,
                        },
                    ],
                },
            ],
        },
    ],
    moves: [
        {
            id: "pair_0",
            label: "Parser moved and simplified",
            old: {
                path: "src/main.ts",
                startLine: 44,
                lineCount: 3,
                contextComplete: true,
                contextWindows: [
                    {
                        startLine: 1,
                        lines: Array.from({ length: 90 }, (_, index) => ({
                            kind:
                                index >= 43 && index <= 45
                                    ? ("removed" as const)
                                    : ("context" as const),
                            oldLine: index + 1,
                            text:
                                index === 43
                                    ? "function parse(input: string) {"
                                    : index === 44
                                      ? "  return legacyParse(input);"
                                      : index === 45
                                        ? "}"
                                        : `// Main source context ${index + 1}`,
                        })),
                    },
                ],
            },
            new: {
                path: "src/parser.ts",
                startLine: 4,
                lineCount: 2,
                contextComplete: true,
                contextWindows: [
                    {
                        startLine: 1,
                        lines: Array.from({ length: 60 }, (_, index) => ({
                            kind:
                                index === 3 || index === 4
                                    ? ("added" as const)
                                    : ("context" as const),
                            newLine: index + 1,
                            text:
                                index === 3
                                    ? "export function parse(input: string) {"
                                    : index === 4
                                      ? "  return parseTokens(input); }"
                                      : `// Parser source context ${index + 1}`,
                        })),
                    },
                ],
            },
        },
    ],
};

test("normal diffs scroll through all files and arrow keys step across fragments", async ({
    page,
}) => {
    const document = structuredClone(review);
    document.files = Array.from({ length: 4 }, (_, index) => ({
        ...structuredClone(review.files[0]),
        id: `file-${index}`,
        path: `src/file-${index}.ts`,
        additions: 1,
        deletions: 1,
        hunks: [
            {
                header: "@@ -1,51 +1,51 @@",
                oldStart: 1,
                oldCount: 51,
                newStart: 1,
                newCount: 51,
                lines: [
                    {
                        kind: "removed" as const,
                        oldLine: 1,
                        text: `old fragment ${index}`,
                    },
                    {
                        kind: "added" as const,
                        newLine: 1,
                        text: `new fragment ${index}`,
                    },
                    ...context(50, 2, 2),
                ],
            },
        ],
    }));
    await setup(page, document);
    await expect(page.getByRole("combobox")).toHaveCount(0);
    const left = page.locator(".flow-scroll.old");
    const right = page.locator(".flow-scroll.new");
    await expect(left).toHaveCount(1);
    await expect(right.locator(".flow-file-heading")).toHaveCount(4);
    await expect(right).toContainText("new fragment 3");

    await right.focus();
    const before = await right.evaluate((element) => element.scrollTop);
    await page.keyboard.press("ArrowDown");
    await expect
        .poll(() => right.evaluate((element) => element.scrollTop))
        .toBeGreaterThan(before);
    await page.keyboard.press("ArrowUp");
    await expect
        .poll(() => right.evaluate((element) => element.scrollTop))
        .toBe(before);
    await page.keyboard.press("ArrowRight");
    await expect(
        right.getByText("new fragment 1", { exact: true }),
    ).toBeInViewport();
    await page.keyboard.press("ArrowRight");
    await expect(
        right.getByText("new fragment 2", { exact: true }),
    ).toBeInViewport();
    await page.keyboard.press("ArrowLeft");
    await expect(
        right.getByText("new fragment 1", { exact: true }),
    ).toBeInViewport();
    await right.evaluate((element) => {
        const context = element.querySelector<HTMLElement>(
            '[data-file-id="file-2"].flow-file-group .flow-section.context',
        )!;
        element.scrollTop +=
            context.getBoundingClientRect().top -
            element.getBoundingClientRect().top +
            200;
    });
    await expect(page.locator(".flow-hunk-nav span")).toHaveText("3 / 4");
    await page.keyboard.press("ArrowRight");
    await expect(
        right.getByText("new fragment 3", { exact: true }),
    ).toBeInViewport();
    await right.evaluate((element) => {
        element.scrollTop = element.scrollHeight;
    });
    await expect(right.locator(".flow-file-heading").last()).toBeAttached();
    await expect
        .poll(() => left.evaluate((element) => element.scrollTop))
        .toBeGreaterThan(3000);

    await page.getByRole("button", { name: "Unified", exact: true }).click();
    const unified = page.locator(".flow-unified-scroll");
    await expect(unified.locator(".flow-file-heading")).toHaveCount(4);
    await unified.focus();
    await page.keyboard.press("ArrowLeft");
    await expect(
        unified.getByText("new fragment 2", { exact: true }),
    ).toBeInViewport();
    await page.screenshot({
        path: test.info().outputPath("continuous-review-unified.png"),
    });
});

test("custom review keeps every paired section and explanation in the scrollable diff", async ({
    page,
}) => {
    const document = structuredClone(movedReview);
    document.guidedFiles = Array.from({ length: 12 }, (_, index) => ({
        ...structuredClone(movedReview.guidedFiles![0]),
        id: `fragment-${index}`,
        label: `Section ${index + 1}: Extract parsing and validate the incoming request before processing`,
        path: "src/features/payments/processing/validation/request-parser.ts",
    }));
    await page.setViewportSize({ width: 760, height: 720 });
    await setup(page, document);
    await page.getByRole("button", { name: "Guided", exact: true }).click();
    const right = page.locator(".flow-scroll.new");
    await expect(page.getByRole("combobox")).toHaveCount(0);
    await expect(right.locator(".flow-file-heading")).toHaveCount(12);
    await right.focus();
    await page.keyboard.press("ArrowRight");
    await expect(
        right.locator('[data-file-id="fragment-1"][data-section]').first(),
    ).toBeInViewport();
    await page.keyboard.press("ArrowLeft");
    await expect(
        right.locator('[data-file-id="fragment-0"][data-section]').first(),
    ).toBeInViewport();
    await right.evaluate((element) => {
        element.scrollTop = element.scrollHeight;
    });
    await expect(right.locator(".flow-file-heading").last()).toBeInViewport();
    expect(
        await page
            .locator(".flow-diff")
            .evaluate((element) => element.getBoundingClientRect().right),
    ).toBeLessThanOrEqual(760);
    await page.screenshot({
        path: test.info().outputPath("continuous-custom-review-narrow.png"),
    });
});

test("fragment navigation reaches separate edits within one hunk", async ({
    page,
}) => {
    const document = structuredClone(review);
    document.files = [document.files[0]];
    document.files[0].hunks = [
        {
            header: "@@ -1,52 +1,52 @@",
            oldStart: 1,
            oldCount: 52,
            newStart: 1,
            newCount: 52,
            lines: [
                { kind: "removed", oldLine: 1, text: "first old edit" },
                { kind: "added", newLine: 1, text: "first new edit" },
                ...context(50, 2, 2),
                { kind: "removed", oldLine: 52, text: "second old edit" },
                { kind: "added", newLine: 52, text: "second new edit" },
            ],
        },
    ];
    await setup(page, document);
    const right = page.locator(".flow-scroll.new");
    await right.focus();
    await page.keyboard.press("ArrowRight");
    await expect(
        right.getByText("second new edit", { exact: true }),
    ).toBeInViewport();
    await page.getByRole("button", { name: "Unified", exact: true }).click();
    const unified = page.locator(".flow-unified-scroll");
    await unified.focus();
    await page.keyboard.press("ArrowLeft");
    await expect(
        unified.getByText("first new edit", { exact: true }),
    ).toBeInViewport();
});

test("moved matches can be traced across files with independent context scrolling", async ({
    page,
}) => {
    const errors: string[] = [];
    page.on("pageerror", (error) => errors.push(error.message));
    await setup(page, movedReview);
    await expect(page.getByRole("combobox")).toHaveCount(0);
    await expect(
        page.locator(".flow-scroll.new .flow-file-heading"),
    ).toHaveCount(2);
    await expect(
        page
            .locator(".flow-scroll.new")
            .getByText('import { parseTokens } from "./tokens";', {
                exact: true,
            }),
    ).toBeVisible();
    await expect(page.locator(".flow-move-overlay")).toHaveCount(0);
    await page.screenshot({
        path: test.info().outputPath("custom-diff-file-changes.png"),
    });
    await page.getByRole("button", { name: "Show moved-code matches" }).click();
    const overlay = page.getByRole("complementary", {
        name: "Possible moved-code match: Parser moved and simplified",
    });
    await expect(
        page.locator(".flow-scroll.old").locator(".flow-move-overlay"),
    ).toBeVisible();
    await expect(overlay.getByText(/src\/main\.ts:\d+/)).toBeVisible();
    const scroller = overlay.locator(".flow-move-scroll");
    await expect
        .poll(() => scroller.evaluate((element) => element.scrollTop))
        .toBeGreaterThan(0);
    await expect
        .poll(() =>
            scroller.evaluate((element) => {
                const first = element.querySelector(".flow-move-paired");
                return (
                    first!.getBoundingClientRect().top -
                    element.getBoundingClientRect().top
                );
            }),
        )
        .toBeGreaterThanOrEqual(0);
    await page.screenshot({
        path: test.info().outputPath("custom-diff-overlay-initial.png"),
    });
    const outerTop = await page
        .locator(".flow-scroll.old")
        .evaluate((element) => element.scrollTop);
    await scroller.focus();
    await page.keyboard.press("k");
    await scroller.evaluate((element) => {
        element.scrollTop = 0;
    });
    await expect(
        overlay.getByText("// Main source context 1", { exact: true }),
    ).toBeVisible();
    expect(
        await page
            .locator(".flow-scroll.old")
            .evaluate((element) => element.scrollTop),
    ).toBe(outerTop);
    await page.screenshot({
        path: test.info().outputPath("custom-diff-overlay-old.png"),
    });
    await scroller.focus();
    await page.keyboard.press("n");
    await page.getByRole("button", { name: "After context" }).click();
    await expect(
        page.locator(".flow-scroll.new .flow-move-overlay"),
    ).toBeVisible();
    await expect(overlay.getByText(/src\/parser\.ts:\d+/)).toBeVisible();
    await page.screenshot({
        path: test.info().outputPath("custom-diff-overlay-new.png"),
    });
    await page.setViewportSize({ width: 760, height: 720 });
    const fits = await page
        .locator(".flow-diff")
        .evaluate(
            (element) =>
                element.getBoundingClientRect().right <= window.innerWidth,
        );
    expect(fits).toBe(true);
    const overlayFits = await overlay.evaluate((element) => {
        const pane = element.closest(".flow-scroll")!.getBoundingClientRect();
        const card = element.getBoundingClientRect();
        return card.left >= pane.left && card.right <= pane.right;
    });
    expect(overlayFits).toBe(true);
    await expect(overlay.locator(".flow-move-label")).toHaveAttribute(
        "title",
        /Parser moved and simplified/,
    );
    await page.screenshot({
        path: test.info().outputPath("custom-diff-overlay-narrow.png"),
    });
    expect(errors).toEqual([]);
});

test("guided view keeps explanations visible and supports stepping through sections", async ({
    page,
}) => {
    await setup(page, movedReview);
    await page.getByRole("button", { name: "Guided", exact: true }).click();
    await expect(page.getByRole("combobox")).toHaveCount(0);
    await expect(
        page.locator(".flow-scroll.new .flow-file-heading strong").first(),
    ).toContainText("Parser moved and simplified");
    await expect(
        page
            .locator(".flow-scroll.old")
            .getByText("  return legacyParse(input);", { exact: true }),
    ).toBeVisible();
    // Clicking Guided leaves a native button focused; review shortcuts still work.
    await page.keyboard.press("n");
    await expect(page.locator(".flow-hunk-nav span")).toContainText("2");
    await page.keyboard.press("N");
    await expect(page.locator(".flow-hunk-nav span")).toContainText("1");
    await page.screenshot({
        path: test.info().outputPath("custom-diff-guided.png"),
    });
    await page.getByRole("button", { name: "Files", exact: true }).click();
    await expect(
        page.locator(".flow-scroll.new .flow-file-heading"),
    ).toHaveCount(2);
    await expect(page.locator(".flow-move-overlay")).toHaveCount(0);
    await page.getByRole("button", { name: "Show moved-code matches" }).click();
    await expect(page.locator(".flow-move-overlay")).toBeVisible();
});

test("stale restructuring can be recovered through native diff actions", async ({
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
            window.diffCommands = [];
            let listener;
            let current = mockSnapshot;
            const savedDocument = ${JSON.stringify(movedReview)};
            export const invoke = async (command, args) => {
                window.diffCommands.push({command, args});
                if (command === 'gui_subscribe') {
                    listener = args.onEvent;
                    listener.onmessage(mockSnapshot);
                }
                if (command === 'gui_diff_action') {
                    const document = current.panes[0].diffReview;
                    const mode = args.action === 'open_saved_overlay' ? 'saved'
                        : args.action === 'return_to_live_diff' ? 'stale'
                        : document.overlay.mode === 'active' ? 'available' : 'active';
                    const nextDocument = mode === 'saved' ? savedDocument
                        : {...savedDocument, custom: false, moves: [], guidedFiles: []};
                    current = {...current, revision: current.revision + 1,
                        panes: [{...current.panes[0], diffReview: {...nextDocument,
                            overlay: {...document.overlay, mode}}}]};
                    listener.onmessage(current);
                }
            };`,
            }),
    );
    await setup(page, {
        ...movedReview,
        custom: false,
        moves: [],
        guidedFiles: [],
        overlay: { mode: "stale", title: "Extract parser" },
    });
    await expect(
        page.getByText("Saved restructuring cannot be verified against this comparison."),
    ).toBeVisible();
    await page.screenshot({
        path: test.info().outputPath("custom-diff-stale.png"),
    });
    await expect(page.locator(".flow-move-overlay")).toHaveCount(0);
    await page.getByRole("button", { name: "View saved review" }).click();
    await expect(page.locator(".flow-move-overlay")).toHaveCount(0);
    // Frozen reviews open in Guided; move overlays belong to the Files view.
    await expect(page.getByRole("button", { name: "Guided", exact: true })).toHaveAttribute("aria-pressed", "true");
    await page.getByRole("button", { name: "Files", exact: true }).click();
    await page.getByRole("button", { name: "Show moved-code matches" }).click();
    await expect(page.locator(".flow-move-overlay")).toBeVisible();
    await expect(
        page.getByRole("button", { name: "Return to live diff" }),
    ).toBeVisible();
    await page.getByRole("button", { name: "Return to live diff" }).click();
    await expect(
        page.getByRole("button", { name: "View saved review" }),
    ).toBeVisible();
    await expect(page.locator(".flow-move-overlay")).toHaveCount(0);
    const actions = await page.evaluate(() =>
        (window as any).diffCommands
            .filter((item: any) => item.command === "gui_diff_action")
            .map((item: any) => item.args),
    );
    expect(actions).toEqual([
        expect.objectContaining({
            pane: 0,
            bufferId: expect.any(Number),
            action: "open_saved_overlay",
        }),
        expect.objectContaining({
            pane: 0,
            bufferId: expect.any(Number),
            action: "return_to_live_diff",
        }),
    ]);
});

test("compact diff panes stay centered and draw unequal change connectors", async ({
    page,
}) => {
    const errors: string[] = [];
    page.on("pageerror", (error) => errors.push(error.message));
    await setup(page);
    await expect(page.locator(".flow-diff svg path").first()).toBeVisible();
    await page.screenshot({ path: test.info().outputPath("diff-desktop.png") });
    const geometry = await page.locator(".flow-columns").evaluate((element) => {
        const left = element
            .querySelector(".flow-scroll.old")!
            .getBoundingClientRect();
        const right = element
            .querySelector(".flow-scroll.new")!
            .getBoundingClientRect();
        const bridge = element
            .querySelector(".flow-bridge")!
            .getBoundingClientRect();
        const bounds = element.getBoundingClientRect();
        return {
            widthDifference: Math.abs(left.width - right.width),
            centerDifference: Math.abs(
                (bridge.left + bridge.right) / 2 -
                    (bounds.left + bounds.right) / 2,
            ),
        };
    });
    expect(geometry.widthDifference).toBeLessThanOrEqual(1);
    expect(geometry.centerDifference).toBeLessThanOrEqual(1);
    await expect(page.locator(".flow-scroll.old .flow-code-line")).toHaveCount(
        104,
    );
    await expect(page.locator(".flow-scroll.new .flow-code-line")).toHaveCount(
        85,
    );
    const left = page.locator(".flow-scroll.old");
    const right = page.locator(".flow-scroll.new");
    await left.evaluate((element) => {
        element.scrollTop = 850;
    });
    await expect
        .poll(() => right.evaluate((element) => element.scrollTop))
        .toBeGreaterThan(200);
    await page.getByRole("button", { name: "Unified", exact: true }).click();
    await expect(page.locator(".flow-unified-scroll")).toBeVisible();
    await page
        .getByRole("button", { name: "Side by side", exact: true })
        .click();
    await expect(page.locator(".flow-diff svg path").first()).toBeVisible();
    await page.locator(".flow-scroll.new").evaluate((element) => {
        element.scrollTop = element.scrollHeight;
    });
    await expect(
        page
            .locator(".flow-scroll.new")
            .getByText("Binary file — no text diff to display."),
    ).toBeInViewport();
    await expect(
        page
            .locator(".flow-scroll.old")
            .getByText("Binary file — no text diff to display."),
    ).toBeInViewport();
    await right.focus();
    await page.keyboard.press("]");
    await page.keyboard.press("f");
    await expect(right.locator(".flow-file-heading").last()).toBeInViewport();
    await page.keyboard.press("[");
    await page.keyboard.press("f");
    await expect(right.locator(".flow-file-heading").first()).toBeInViewport();
    expect(errors).toEqual([]);
});

test("diff viewer fits a narrow workbench without page overflow", async ({
    page,
}) => {
    await page.setViewportSize({ width: 760, height: 720 });
    await setup(page);
    const fits = await page.locator(".flow-diff").evaluate((element) => {
        const box = element.getBoundingClientRect();
        return box.left >= 0 && box.right <= window.innerWidth;
    });
    expect(fits).toBe(true);
    await page.screenshot({ path: test.info().outputPath("diff-narrow.png") });
});

test("native diff controls carry the pane and buffer identity", async ({
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
                window.diffCommands = [];
                let listener;
                export const invoke = async (command, args) => {
                    window.diffCommands.push({command, args});
                    if (command === 'gui_terminal_open') return 1;
                    if (command === 'gui_subscribe') {
                        listener = args.onEvent;
                        listener.onmessage(mockSnapshot);
                    }
                    if (command === 'gui_open_diff_source') listener.onmessage({
                        ...mockSnapshot,
                        revision: mockSnapshot.revision + 1,
                        panes: [{ ...mockSnapshot.panes[0], diffReview: undefined }],
                    });
                };`,
            }),
    );
    await setup(page);
    await expect(page.locator(".flow-diff")).toBeFocused();
    await page.keyboard.press("j");
    await expect
        .poll(() =>
            page
                .locator(".flow-scroll.old")
                .evaluate((element) => element.scrollTop),
        )
        .toBeGreaterThan(0);
    const scrollBeforeTerminal = await page
        .locator(".flow-scroll.old")
        .evaluate((element) => element.scrollTop);
    const terminalShortcut = await page.evaluate(() =>
        /Mac|iPhone|iPad/.test(navigator.platform)
            ? "Meta+Shift+T"
            : "Control+Shift+T",
    );
    await page.keyboard.press(terminalShortcut);
    const shellInput = page.locator(".terminal-panel .xterm-helper-textarea");
    await expect(shellInput).toBeFocused();
    const toolbarFits = await page
        .locator(".flow-toolbar")
        .evaluate((toolbar) => {
            const bounds = toolbar.getBoundingClientRect();
            return Array.from(toolbar.querySelectorAll("button, select")).every(
                (control) => {
                    const rect = control.getBoundingClientRect();
                    return (
                        rect.left >= bounds.left && rect.right <= bounds.right
                    );
                },
            );
        });
    expect(toolbarFits).toBe(true);
    await page.keyboard.type("pwd");
    await expect
        .poll(() =>
            page.evaluate(() =>
                (window as any).diffCommands
                    .filter(
                        (item: any) => item.command === "gui_terminal_write",
                    )
                    .map((item: any) => item.args.data)
                    .join(""),
            ),
        )
        .toBe("pwd");
    expect(
        await page.evaluate(() =>
            (window as any).diffCommands.some(
                (item: any) => item.command === "gui_key",
            ),
        ),
    ).toBe(false);
    await page.screenshot({
        path: test.info().outputPath("diff-with-terminal.png"),
    });
    await page.keyboard.press(terminalShortcut);
    await expect(page.locator(".flow-diff")).toBeFocused();
    expect(
        await page
            .locator(".flow-scroll.old")
            .evaluate((element) => element.scrollTop),
    ).toBe(scrollBeforeTerminal);
    await page.keyboard.press(":");
    await expect(
        page.getByRole("textbox", { name: "Ovim editor input" }),
    ).toBeFocused();
    await expect
        .poll(() =>
            page.evaluate(() =>
                (window as any).diffCommands.some(
                    (item: any) =>
                        item.command === "gui_key" &&
                        item.args?.input?.key === ":",
                ),
            ),
        )
        .toBe(true);
    await page.getByRole("button", { name: "Unified", exact: true }).click();
    const action = await page.evaluate(
        () =>
            (window as any).diffCommands.find(
                (item: any) => item.command === "gui_diff_action",
            )?.args,
    );
    expect(action).toMatchObject({ pane: 0, action: "unified" });
    expect(action.bufferId).toEqual(expect.any(Number));
    await page
        .getByRole("button", {
            name: "After line 1204, open source",
            exact: true,
        })
        .click();
    await expect
        .poll(() =>
            page.evaluate(
                () =>
                    (window as any).diffCommands.find(
                        (item: any) => item.command === "gui_open_diff_source",
                    )?.args,
            ),
        )
        .toMatchObject({
            pane: 0,
            path: "src/payments/PaymentResource.java",
            line: 1204,
            side: "new",
        });
    await expect(page.locator(".flow-diff")).toHaveCount(0);
    await expect(
        page.getByRole("textbox", { name: "Ovim editor input" }),
    ).toBeFocused();
});

for (const view of ["Files", "Guided"]) {
    test(`exports a complete ${view} review as a PNG`, async ({
        page,
    }, testInfo) => {
        await setup(page, movedReview);
        await page.getByRole("button", { name: view, exact: true }).click();
        const pending = page.waitForEvent("download");
        await page
            .getByRole("button", { name: "Export image", exact: true })
            .click();
        const download = await pending;
        expect(download.suggestedFilename()).toMatch(/\.png$/);
        const destination = testInfo.outputPath("review.png");
        await download.saveAs(destination);
        const bytes = await readFile(destination);
        expect([...bytes.subarray(0, 8)]).toEqual([
            137, 80, 78, 71, 13, 10, 26, 10,
        ]);
        expect(bytes.readUInt32BE(16)).toBe(1600);
        expect(bytes.readUInt32BE(20)).toBeGreaterThan(420);
        await expect(page.locator(".flow-export-status")).toContainText(
            "Download started",
        );
    });
}

test("exports a tall review as one dark PNG beyond browser canvas limits", async ({
    page,
}, testInfo) => {
    test.setTimeout(120_000);
    const longReview = structuredClone(review);
    const file = longReview.files[0];
    file.additions = 2500;
    file.deletions = 0;
    file.hunks = [
        {
            oldStart: 0,
            oldCount: 0,
            newStart: 1,
            newCount: 2500,
            header: "@@ -0,0 +1,2500 @@",
            lines: Array.from({ length: 2500 }, (_, i) => ({
                kind: "added",
                text: `export const value${i} = ${i};`,
                newLine: i + 1,
            })),
        },
    ];
    await setup(page, longReview);
    const pending = page.waitForEvent("download");
    await page
        .getByRole("button", { name: "Export image", exact: true })
        .click();
    const download = await pending;
    expect(download.suggestedFilename()).toMatch(/\.png$/);
    const destination = testInfo.outputPath("review.png");
    await download.saveAs(destination);
    const png = await readFile(destination);
    expect([...png.subarray(0, 8)]).toEqual([137, 80, 78, 71, 13, 10, 26, 10]);
    expect(png.readUInt32BE(16)).toBe(1600);
    const height = png.readUInt32BE(20);
    expect(height).toBeGreaterThan(62500);
    const data: Uint8Array[] = [];
    for (let offset = 8; offset < png.length;) {
        const length = png.readUInt32BE(offset);
        if (png.toString("ascii", offset + 4, offset + 8) === "IDAT") data.push(png.subarray(offset + 8, offset + 8 + length));
        offset += length + 12;
    }
    const pixels = unzlibSync(Buffer.concat(data));
    expect(pixels.length).toBe(height * (1600 * 4 + 1));
    // Sub-filter first pixel is literal RGBA. Check the final strip as well.
    expect([...pixels.subarray(1, 5)]).toEqual([17, 24, 39, 255]);
    expect([...pixels.subarray((height - 1) * 6401 + 1, (height - 1) * 6401 + 5)]).toEqual([17, 24, 39, 255]);
});

for (const width of [1440, 760]) {
    test(`hide equal agent pairings across hunks at ${width}px`, async ({
        page,
    }, testInfo) => {
        await page.setViewportSize({ width, height: 900 });
        const same: GuiDiffDocument["files"][number] = {
            id: "src/same.ts",
            path: "src/same.ts",
            status: "modified",
            additions: 1,
            deletions: 1,
            binary: false,
            metadata: [],
            hunks: [
                {
                    header: "@@ -10 +10,0 @@",
                    oldStart: 10,
                    oldCount: 1,
                    newStart: 10,
                    newCount: 0,
                    lines: [
                        {
                            kind: "removed",
                            text: "sameFilePair();",
                            oldLine: 10,
                        },
                    ],
                },
                {
                    header: "@@ -40,0 +40 @@",
                    oldStart: 40,
                    oldCount: 0,
                    newStart: 40,
                    newCount: 1,
                    lines: [
                        {
                            kind: "added",
                            text: "  sameFilePair( );",
                            newLine: 40,
                        },
                    ],
                },
            ],
        };
        const cross: GuiDiffDocument["files"][number] = {
            ...same,
            id: "cross",
            path: "src/new.ts",
            oldPath: "src/old.ts",
            status: "reassigned",
            hunks: [
                {
                    header: "Moved between files",
                    oldStart: 5,
                    oldCount: 1,
                    newStart: 20,
                    newCount: 1,
                    lines: [
                        {
                            kind: "removed",
                            text: "crossFilePair();",
                            oldLine: 5,
                        },
                        {
                            kind: "added",
                            text: "crossFilePair();",
                            newLine: 20,
                        },
                    ],
                },
            ],
        };
        const removed = {
            ...cross,
            id: "old",
            path: "src/old.ts",
            oldPath: undefined,
            status: "deleted",
            additions: 0,
            hunks: [
                {
                    ...cross.hunks[0],
                    newCount: 0,
                    lines: cross.hunks[0].lines.slice(0, 1),
                },
            ],
        };
        const added = {
            ...cross,
            id: "new",
            oldPath: undefined,
            status: "added",
            deletions: 0,
            hunks: [
                {
                    ...cross.hunks[0],
                    oldCount: 0,
                    lines: cross.hunks[0].lines.slice(1),
                },
            ],
        };
        await setup(page, {
            title: "Diff · Agent paired equal code",
            layout: "split",
            managed: true,
            custom: true,
            files: [same, removed, added],
            guidedFiles: [
                {
                    ...same,
                    status: "reassigned",
                    oldPath: same.path,
                    label: "Same-file pair",
                    hunks: [
                        {
                            ...same.hunks[0],
                            newStart: 40,
                            newCount: 1,
                            lines: same.hunks.flatMap((h) => h.lines),
                        },
                    ],
                },
                { ...cross, label: "Cross-file move" },
            ],
            moves: [
                {
                    id: "same",
                    old: {
                        path: same.path,
                        startLine: 10,
                        lineCount: 1,
                        contextWindows: [],
                        contextComplete: false,
                    },
                    new: {
                        path: same.path,
                        startLine: 40,
                        lineCount: 1,
                        contextWindows: [],
                        contextComplete: false,
                    },
                },
            ],
        });
        const diff = page.getByRole("region", {
            name: "Diff review",
            exact: true,
        });
        const equalLines = page
            .locator(".flow-code-line")
            .filter({ hasText: "sameFilePair" });
        const movedLines = page
            .locator(".flow-code-line")
            .filter({ hasText: "crossFilePair" });
        await expect(equalLines).toHaveCount(2);
        await diff.focus();
        await page.keyboard.press("w");
        const toggle = page.getByRole("button", { name: "Hide equal changes" });
        await expect(toggle).toHaveAttribute("aria-pressed", "true");
        await expect(equalLines).toHaveCount(0);
        await expect(movedLines).toHaveCount(2);
        await page.getByRole("button", { name: "Guided", exact: true }).click();
        await expect(equalLines).toHaveCount(0);
        await expect(movedLines).toHaveCount(0);
        await expect(page.getByText("0 / 0", { exact: true })).toBeVisible();
        await diff.focus();
        await page.keyboard.press("s");
        await expect(page.getByText("No unequal changes", {exact: true})).toBeVisible();
        await expect(equalLines).toHaveCount(0);
        await expect(movedLines).toHaveCount(0);
        await page.screenshot({
            path: testInfo.outputPath("equal-pairs-hidden.png"),
        });
        await toggle.click();
        await expect(equalLines).toHaveCount(2);
        await expect(movedLines).toHaveCount(2);
    });
}

test("an entirely equal curated review has a visible empty state and can be restored", async ({
    page,
}, testInfo) => {
    await page.setViewportSize({ width: 760, height: 700 });
    const file: GuiDiffDocument["files"][number] = {
        id: "equal",
        path: "src/equal.ts",
        oldPath: "src/equal.ts",
        status: "reassigned",
        additions: 1,
        deletions: 1,
        binary: false,
        metadata: [],
        hunks: [
            {
                header: "Equal pair",
                oldStart: 1,
                oldCount: 1,
                newStart: 10,
                newCount: 1,
                lines: [
                    { kind: "removed", text: "same();", oldLine: 1 },
                    { kind: "added", text: " same( );", newLine: 10 },
                ],
            },
        ],
    };
    await setup(page, {
        title: "Diff · Equal pair",
        layout: "split",
        managed: true,
        custom: true,
        files: [file],
        guidedFiles: [file],
    });
    const toggle = page.getByRole("button", { name: "Hide equal changes" });
    await toggle.click();
    await expect(page.locator(".flow-empty")).toHaveText("No unequal changes");
    await expect(page.locator(".flow-empty")).toBeVisible();
    await expect(
        page.getByRole("button", { name: "Next change" }),
    ).toBeDisabled();
    const download = page.waitForEvent("download");
    await page.getByRole("button", { name: "Export image" }).click();
    expect((await download).suggestedFilename()).toMatch(/\.png$/);
    await page.screenshot({
        path: testInfo.outputPath("equal-review-empty.png"),
    });
    await toggle.click();
    await expect(page.locator(".flow-code-line")).toHaveCount(2);
});

for (const curated of [false, true]) test(`context expansion preserves code position and adds only source rows (${curated ? "curated unified" : "ordinary split"})`, async ({ page }) => {
    const document = structuredClone(review);
    document.files = [document.files[0]];
    if (curated) {
        document.custom = true;
        document.layout = "unified";
        document.files[0].status = "reassigned";
        document.files[0].oldPath = "src/legacy/PaymentResource.java";
        document.files[0].hunks[0].lines = document.files[0].hunks[0].lines.filter(line => line.kind !== "context");
    }
    document.files[0].hunks[0].context = {
        id: "hunk:8", before: { old: [], new: [] }, after: { old: [], new: [] }, canExpandUp: true, canExpandDown: true,
    };
    await page.route("**/node_modules/.vite/deps/@tauri-apps_api_event.js*", route => route.fulfill({ contentType: "application/javascript", body: "export const listen = async () => () => {};" }));
    await page.route("**/node_modules/.vite/deps/@tauri-apps_api_core.js*", route => route.fulfill({ contentType: "application/javascript", body: `
        import { mockSnapshot } from '/src/mock.ts';
        export const isTauri = () => true;
        export class Channel {}
        window.diffCommands = [];
        let listener, current = mockSnapshot;
        export const invoke = async (command, args) => {
            window.diffCommands.push({command, args});
            if (command === 'gui_subscribe') { listener = args.onEvent; listener.onmessage(current); }
            if (command === 'gui_diff_action') {
                const next = structuredClone(current);
                const doc = next.panes[0].diffReview;
                const context = doc.files[0].hunks[0].context;
                if (args.action.startsWith('expand_up:')) {
                    context.before = { old: Array.from({length: 10}, (_, i) => ({number: 1190+i, text: 'captured_before_' + i + '();'})), new: Array.from({length: 10}, (_, i) => ({number: 1190+i, text: 'captured_before_' + i + '();'})) };
                    context.canExpandUp = false;
                } else if (args.action.startsWith('expand_down:')) {
                    context.after = { old: [{number: 1304, text: 'captured_after();'}], new: [{number: 1285, text: 'captured_after();'}] };
                    context.canExpandDown = false;
                } else if (args.action === 'unified') doc.layout = 'unified';
                next.revision++;
                current = next;
                listener.onmessage(next);
            }
        };` }));
    await setup(page, document);
    const old = page.locator(curated ? ".flow-unified-scroll" : ".flow-scroll.old");
    const anchor = old.getByRole("button", { name: `Before line ${curated ? 1204 : 1200}, open source`, exact: true });
    await page.screenshot({ path: "/tmp/ovim-context-controls-" + test.info().project.name + (curated ? "-curated" : "") + ".png" });
    const before = (await anchor.boundingBox())!;
    const count = await old.locator(".flow-code-line").count();
    await old.getByRole("button", { name: "Show more context above" }).click();
    await expect(old.locator(".flow-code-line")).toHaveCount(count + (curated ? 20 : 10));
    await expect.poll(async () => Math.abs((await anchor.boundingBox())!.y - before.y)).toBeLessThan(2);
    await expect(old.getByRole("button", { name: "Show more context above" })).toHaveCount(0);
    await page.locator(".flow-diff").press("J");
    await expect(old.getByText("captured_after();")).toHaveCount(curated ? 2 : 1);
    const rows = await old.locator(".flow-code-line").evaluateAll(elements => elements.map(element => {
        const rect = element.getBoundingClientRect(); return { top: rect.top, bottom: rect.bottom };
    }));
    expect(rows.every((row, index) => !index || Math.abs(row.top - rows[index - 1].bottom) < 1)).toBe(true);
    if (!curated) await page.locator(".flow-diff").press("s");
    const unified = page.locator(".flow-unified-scroll");
    await expect(unified).toBeVisible();
    await expect(unified.getByText("captured_before_0();", {exact: true})).toHaveCount(curated ? 2 : 1);
    await expect(unified.getByText("captured_after();", {exact: true})).toHaveCount(curated ? 2 : 1);
    await page.screenshot({ path: '/tmp/ovim-context-' + test.info().project.name + '.png' });
});

test("one-line diff context controls fit the gutter without overlapping", async ({ page }) => {
    await page.setViewportSize({ width: 760, height: 600 });
    const document = structuredClone(review);
    document.files = [document.files[0]];
    document.files[0].hunks = [{ header: "Pair", oldStart: 10, oldCount: 1, newStart: 10, newCount: 1,
        lines: [{ kind: "removed", text: "before();", oldLine: 10 }, { kind: "added", text: "after();", newLine: 10 }],
        context: { id: "hunk:1", before: { old: [], new: [] }, after: { old: [], new: [] }, canExpandUp: true, canExpandDown: true },
    }];
    await setup(page, document);
    const old = page.locator(".flow-scroll.old");
    const up = (await old.getByRole("button", { name: "Show more context above" }).boundingBox())!;
    const down = (await old.getByRole("button", { name: "Show more context below" }).boundingBox())!;
    const code = (await old.locator(".flow-line-text").boundingBox())!;
    expect(up.y + up.height).toBeLessThanOrEqual(down.y);
    expect(up.x + up.width).toBeLessThanOrEqual(code.x);
    expect(down.x + down.width).toBeLessThanOrEqual(code.x);
    // This case measures layout. Use the visible control and synchronize its
    // state; keyboard navigation is exercised in the dedicated shortcut cases.
    const unified = page.getByRole("button", { name: "Unified", exact: true });
    await unified.click();
    await expect(unified).toHaveAttribute("aria-pressed", "true");
    const heading = (await page.locator(".flow-unified-heading").boundingBox())!;
    const unifiedUp = (await page.locator(".flow-unified-scroll").getByRole("button", { name: "Show more context above" }).boundingBox())!;
    expect(unifiedUp.y).toBeGreaterThanOrEqual(heading.y + heading.height);
});


test("guided diff search owns slash and find typing, wraps matches and keeps shortcuts out", async ({page}, testInfo) => {
    await setup(page, movedReview);
    await page.getByRole("button", {name: "Guided", exact: true}).click();
    const panel = page.getByRole("region", {name: "Diff review", exact: true});
    await panel.focus();
    await page.keyboard.press("/");
    const input = page.getByRole("searchbox", {name: "Find in diff"});
    await expect(input).toBeFocused();
    await page.keyboard.type("parse");
    await expect(input).toHaveValue("parse");
    await expect(page.locator(".flow-search-current")).toHaveCount(1);
    const first = await page.locator(".flow-search-current").textContent();
    await input.press("Enter");
    await expect(page.locator(".flow-search-current")).not.toHaveText(first!);
    await input.press("Shift+Enter");
    await expect(page.locator(".flow-search-current")).toHaveText(first!);
    await input.fill("jkswr");
    await expect(page.getByRole("button", {name: "Hide equal changes"})).toHaveAttribute("aria-pressed", "false");
    await expect(page.getByRole("button", {name: "Side by side", exact: true})).toHaveAttribute("aria-pressed", "true");
    await expect(page.getByText("0 / 0 matching lines", {exact: true})).toBeVisible();
    await input.press("Escape");
    await expect(input).toHaveCount(0);
    await expect(panel).toBeFocused();
    await page.keyboard.press("Control+f");
    await expect(input).toBeFocused();
    await input.press("Escape");
    await page.keyboard.press("Meta+f");
    await expect(input).toBeFocused();
    await input.fill("parseTokens");
    await expect(page.locator(".flow-search-current")).toHaveCount(1);
    await page.screenshot({path: testInfo.outputPath("guided-search.png")});
    await input.press("Escape");
    await page.getByRole("button", {name: "Unified", exact: true}).click();
    await panel.focus();
    await page.keyboard.press("/");
    await expect(input).toBeFocused();
    await expect(page.locator(".flow-search-current")).toHaveCount(1);
});

// Set OVIM_SECTION_QA_FIXTURE to the JSON emitted by the native projection test
// to run this same rendered flow against a freshly restored Rust review.
for (const width of [1440, 760]) test(`labelled deletions and referenced copies stay separate at ${width}px when equal pairs hide`, async ({page}, testInfo) => {
    await page.setViewportSize({width, height: 900});
    const oldLines: GuiDiffLine[] = [
        {kind: "removed", text: "duplicate_check();", oldLine: 120},
        {kind: "removed", text: "respondent();", oldLine: 121},
    ];
    const newLines: GuiDiffLine[] = [
        {kind: "added", text: "respondent();", newLine: 300},
        {kind: "added", text: "respondent();", newLine: 301},
    ];
    const file = (id: string, path: string, status: string, lines: GuiDiffLine[], oldPath?: string, label?: string): GuiDiffDocument["files"][number] => ({
        id, path, status, oldPath, label, binary: false, metadata: [],
        additions: lines.filter(l => l.kind === "added").length,
        deletions: lines.filter(l => l.kind === "removed").length,
        hunks: [{header: "Selected source slice", oldStart: lines.find(l => l.oldLine)?.oldLine || 0, oldCount: lines.filter(l => l.oldLine).length,
            newStart: lines.find(l => l.newLine)?.newLine || 0, newCount: lines.filter(l => l.newLine).length, lines}],
    });
    const fallback: GuiDiffDocument = {title: "Explain deletion and copy", layout: "split", managed: true, custom: true,
        files: [file("old", "old.rs", "deleted", oldLines), file("new", "new.rs", "added", newLines)],
        guidedFiles: [
            file("pair_0", "old.rs", "deletion", [oldLines[0]], "old.rs", "Remove duplicate permission check"),
            file("pair_1", "new.rs", "reassigned", [oldLines[1], newLines[0]], "old.rs", "Move respondent lookup"),
            file("pair_2", "new.rs", "addition", [newLines[1]], undefined, "Additional respondent copy · Related source (before): old.rs:121–121"),
        ]};
    const document: GuiDiffDocument = process.env.OVIM_SECTION_QA_FIXTURE
        ? JSON.parse(await readFile(process.env.OVIM_SECTION_QA_FIXTURE, "utf8")) : fallback;
    await setup(page, document);
    await page.getByRole("button", {name: "Guided", exact: true}).click();
    const code = page.locator(".flow-code-line");
    await expect(code.filter({hasText: "respondent();"})).toHaveCount(3);
    await expect(code.filter({hasText: "duplicate_check();"})).toHaveCount(1);
    await page.getByRole("button", {name: "Hide equal changes", exact: true}).click();
    for (const layout of ["Side by side", "Unified"]) {
        await page.getByRole("button", {name: layout, exact: true}).click();
        await expect(code.filter({hasText: "respondent();"})).toHaveCount(1);
        await expect(code.filter({hasText: "duplicate_check();"})).toHaveCount(1);
        await expect(page.locator(".flow-status.deletion").first()).toBeVisible();
        await expect(page.locator(".flow-status.addition").first()).toBeVisible();
        await expect(page.locator(".flow-file-heading").filter({hasText: "Related source (before): old.rs:"}).first()).toBeVisible();
        await expect(page.locator(".flow-gap").filter({hasText: /\d+ unchanged lines/})).toHaveCount(0);
        const overflow = await page.locator(".flow-diff").evaluate(element => element.scrollWidth > element.clientWidth + 1);
        expect(overflow).toBe(false);
        const pending = page.waitForEvent("download");
        await page.getByRole("button", {name: "Export image", exact: true}).click();
        const download = await pending;
        expect(download.suggestedFilename()).toMatch(/guided\.png$/);
        await download.saveAs(testInfo.outputPath(`annotated-${layout}.png`));
        await page.screenshot({path: testInfo.outputPath(`annotated-${layout}-gui.png`)});
    }
});
