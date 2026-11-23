import { expect, test, type Page } from "@playwright/test";
import type { GuiDiffDocument } from "../src/types";

const review: GuiDiffDocument = {
    title: "Diff · main → worktree",
    provenance: {
        baseLabel: "main",
        comparisonBaseOid: "0123456789abcdef0123456789abcdef01234567",
    },
    layout: "unified",
    managed: true,
    custom: false,
    files: [
        {
            id: "src/example.ts",
            path: "src/example.ts",
            status: "modified",
            additions: 1,
            deletions: 1,
            binary: false,
            metadata: [],
            hunks: [
                {
                    header: "@@ -1 +1 @@",
                    oldStart: 1,
                    oldCount: 1,
                    newStart: 1,
                    newCount: 1,
                    lines: [
                        {
                            kind: "removed",
                            text: "export const value = 1;",
                            oldLine: 1,
                        },
                        {
                            kind: "added",
                            text: "export const value = 2;",
                            newLine: 1,
                        },
                    ],
                },
            ],
        },
    ],
};

async function setupNative(
    page: Page,
    outcome: "saved" | "cancelled" | "error",
) {
    await page.route("**/src/mock.ts", async (route) => {
        const response = await route.fetch();
        await route.fulfill({
            response,
            body: `${await response.text()}
                delete mockSnapshot.aiChat;
                delete mockSnapshot.fileTree;
                mockSnapshot.fileName = ${JSON.stringify(review.title)};
                mockSnapshot.filePath = undefined;
                mockSnapshot.readOnly = true;
                mockSnapshot.panes = [{...mockSnapshot.panes[0], diffReview: ${JSON.stringify(review)}}];`,
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
                window.diffExportCalls = [];
                export const invoke = async (command, data, options) => {
                    if (command === 'gui_subscribe') {
                        data.onEvent.onmessage(mockSnapshot);
                    }
                    if (command === 'gui_save_diff_export') {
                        window.diffExportCalls.push({
                            filename: options.headers['x-ovim-export-filename'],
                            length: data.byteLength,
                            signature: Array.from(data.slice(0, 8)),
                        });
                        if (${JSON.stringify(outcome)} === 'error') throw new Error('Disk full');
                        return ${JSON.stringify(outcome)} === 'saved';
                    }
                };`,
            }),
    );
    await page.goto("/");
    await expect(page.locator(".flow-diff")).toBeVisible();
}

test("native image export sends PNG bytes and a safe filename to the save command", async ({
    page,
}) => {
    await setupNative(page, "saved");
    await page
        .getByRole("button", { name: "Export image", exact: true })
        .click();
    await expect(page.locator(".flow-export-status")).toContainText("Saved");
    const calls = await page.evaluate(() => (window as any).diffExportCalls);
    expect(calls).toHaveLength(1);
    expect(calls[0].filename).toMatch(/^[A-Za-z0-9._-]+\.png$/);
    expect(calls[0].length).toBeGreaterThan(8);
    expect(calls[0].signature).toEqual([137, 80, 78, 71, 13, 10, 26, 10]);
});

test("cancelling the native save dialog leaves the export available", async ({
    page,
}) => {
    await setupNative(page, "cancelled");
    await page
        .getByRole("button", { name: "Export image", exact: true })
        .click();
    await expect(
        page.getByRole("button", { name: "Export image", exact: true }),
    ).toBeEnabled();
    await expect(page.locator(".flow-export-status")).toHaveCount(0);
});

test("native save errors are shown in the diff", async ({ page }) => {
    await setupNative(page, "error");
    await page
        .getByRole("button", { name: "Export image", exact: true })
        .click();
    await expect(page.getByRole("alert")).toContainText("Disk full");
    await expect(
        page.getByRole("button", { name: "Export image", exact: true }),
    ).toBeEnabled();
});
