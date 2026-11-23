import { expect, test, type Page } from "@playwright/test";
import type { GuiDiffDocument } from "../src/types";

const line = (kind: "removed" | "added", text: string) => ({
    kind,
    text,
    ...(kind === "removed" ? { oldLine: 1 } : { newLine: 1 }),
});

const canonical: GuiDiffDocument["files"][number] = {
    id: "src/result.ts",
    path: "src/result.ts",
    status: "modified",
    additions: 2,
    deletions: 2,
    binary: false,
    metadata: [],
    hunks: [
        {
            header: "@@ -1 +1 @@",
            oldStart: 1,
            oldCount: 2,
            newStart: 1,
            newCount: 2,
            lines: [
                line("removed", "const result = oldValue;"),
                {
                    kind: "removed",
                    text: "const count = oldCount;",
                    oldLine: 2,
                },
                line("added", "const result = newValue;"),
                { kind: "added", text: "const count = newCount;", newLine: 2 },
            ],
        },
    ],
};

const review: GuiDiffDocument = {
    title: "Diff · Explain the rename",
    layout: "split",
    managed: true,
    custom: true,
    overlay: { mode: "saved" },
    files: [canonical],
    guidedFiles: [
        {
            ...canonical,
            id: "pair_0",
            label: "Use the new result",
            status: "reassigned",
            message:
                "The value comes from the new parser.\nCheck the caller before removing the old path.",
            additions: 1,
            deletions: 1,
            hunks: [
                {
                    ...canonical.hunks[0],
                    oldCount: 1,
                    newCount: 1,
                    lines: [
                        canonical.hunks[0].lines[0],
                        canonical.hunks[0].lines[2],
                    ],
                },
            ],
        },
        {
            ...canonical,
            id: "pair_1",
            label: "Update the count",
            status: "reassigned",
            additions: 1,
            deletions: 1,
            hunks: [
                {
                    ...canonical.hunks[0],
                    oldStart: 2,
                    newStart: 2,
                    oldCount: 1,
                    newCount: 1,
                    lines: [
                        canonical.hunks[0].lines[1],
                        canonical.hunks[0].lines[3],
                    ],
                },
            ],
        },
    ],
};

async function setup(page: Page, document: GuiDiffDocument) {
    await page.route("**/src/mock.ts", async (route) => {
        const response = await route.fetch();
        await route.fulfill({
            response,
            body: `${await response.text()}\n
                delete mockSnapshot.aiChat;
                delete mockSnapshot.fileTree;
                mockSnapshot.fileName = ${JSON.stringify(document.title)};
                mockSnapshot.filePath = undefined;
                mockSnapshot.readOnly = true;
                mockSnapshot.panes = [{...mockSnapshot.panes[0], diffReview: ${JSON.stringify(document)}}];`,
        });
    });
    await page.goto("/");
}

test("agent notes appear in both layouts and can be hidden without losing the code", async ({
    page,
}, testInfo) => {
    await setup(page, review);
    await expect(page.getByRole("button", { name: "Guided" })).toHaveAttribute(
        "aria-pressed",
        "true",
    );
    await expect(
        page.getByText("Use the new result · src/result.ts"),
    ).toHaveCount(2);
    await expect(
        page.getByText("Update the count · src/result.ts"),
    ).toHaveCount(2);
    await expect(page.getByText("1 unchanged line")).toHaveCount(0);
    const note = page.getByLabel("Agent note");
    await expect(note).toHaveCount(2);
    await expect(note.first()).toContainText(
        "The value comes from the new parser.",
    );
    await page
        .locator(".flow-diff")
        .screenshot({ path: testInfo.outputPath("notes-split.png") });
    await page.getByRole("button", { name: "Notes" }).click();
    await expect(note).toHaveCount(0);
    await expect(
        page.getByText("const result = newValue;", { exact: true }),
    ).toBeVisible();
    await page.getByRole("button", { name: "Notes" }).click();
    await page.getByRole("button", { name: "Next change" }).click();
    await expect(
        page.locator(
            '.flow-scroll.old [data-file-id="pair_1"][data-active="true"]',
        ),
    ).toHaveCount(1);
    await page.getByRole("button", { name: "Notes" }).click();
    await expect(
        page.locator(
            '.flow-scroll.old [data-file-id="pair_1"][data-active="true"]',
        ),
    ).toHaveCount(1);
    const before = await page
        .locator('.flow-scroll.old .flow-file-group[data-file-id="pair_1"]')
        .boundingBox();
    const after = await page
        .locator('.flow-scroll.new .flow-file-group[data-file-id="pair_1"]')
        .boundingBox();
    expect(Math.abs((before?.y ?? 0) - (after?.y ?? 0))).toBeLessThan(2);
    await page.getByRole("button", { name: "Notes" }).click();
    await page.getByRole("button", { name: "Unified" }).click();
    await expect(note).toHaveCount(1);
    await page
        .locator(".flow-diff")
        .screenshot({ path: testInfo.outputPath("notes-unified.png") });
});

test("a long note does not move the selected later section or misalign split panes", async ({
    page,
}) => {
    const longReview = {
        ...review,
        guidedFiles: review.guidedFiles?.map((section, index) =>
            index === 0
                ? {
                      ...section,
                      message: Array(24)
                          .fill(
                              "The parser now returns a normalized result for callers.",
                          )
                          .join(" "),
                  }
                : section,
        ),
    };
    await setup(page, longReview);
    await page.getByRole("button", { name: "Next change" }).click();
    const selected = page.locator(
        '.flow-scroll.old [data-file-id="pair_1"][data-active="true"]',
    );
    await expect(selected).toHaveCount(1);
    await page.getByRole("button", { name: "Notes" }).click();
    await expect(selected).toHaveCount(1);
    await expect(selected).toBeInViewport();
    const before = await page
        .locator('.flow-scroll.old .flow-file-group[data-file-id="pair_1"]')
        .boundingBox();
    const after = await page
        .locator('.flow-scroll.new .flow-file-group[data-file-id="pair_1"]')
        .boundingBox();
    expect(Math.abs((before?.y ?? 0) - (after?.y ?? 0))).toBeLessThan(2);
});
