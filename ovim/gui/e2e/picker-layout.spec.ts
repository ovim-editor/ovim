import { expect, test } from "@playwright/test";

for (const fileFilter of ["", "*.rs"]) {
    for (const count of [0, 1, 80]) {
        test(`picker keeps ${count} results visible with filter ${JSON.stringify(fileFilter)}`, async ({
            page,
        }) => {
            await page.route("**/src/mock.ts", async (route) => {
                const response = await route.fetch();
                await route.fulfill({
                    response,
                    body:
                        (await response.text()) +
                        `
delete mockSnapshot.aiChat;
mockSnapshot.picker = {
    title: "Search project", query: "result", fileFilter: ${JSON.stringify(fileFilter)},
    selected: 0, total: ${count}, items: Array.from({length: ${count}}, (_, index) => ({
        index, display: "src/result_" + index + ".rs", location: "src", detail: "A matching result", matched: []
    }))
};`,
                });
            });
            await page.goto("/");
            const picker = page.getByRole("dialog", {
                name: "result",
                exact: true,
            });
            await expect(picker).toBeVisible();
            const results = picker.getByRole("listbox");
            const bounds = await results.boundingBox();
            const footer = await picker.locator("footer").boundingBox();
            const dialog = await picker.boundingBox();
            expect(bounds!.height).toBeGreaterThan(30);
            expect(bounds!.y + bounds!.height).toBeLessThanOrEqual(
                footer!.y + 1,
            );
            expect(footer!.y + footer!.height).toBeLessThanOrEqual(
                dialog!.y + dialog!.height,
            );
            if (count) {
                await expect(
                    picker.getByRole("option").first(),
                ).toBeInViewport();
                await picker
                    .getByRole("option")
                    .last()
                    .scrollIntoViewIfNeeded();
                await expect(
                    picker.getByRole("option").last(),
                ).toBeInViewport();
            } else {
                await expect(
                    picker.getByText("No matching results"),
                ).toBeInViewport();
            }
            await page.screenshot({
                path: test.info().outputPath("picker.png"),
            });
        });
    }
}

test("startup dashboard renders the splash and shortcuts", async ({ page }) => {
    await page.route("**/src/mock.ts", async (route) => {
        const response = await route.fetch();
        await route.fulfill({
            response,
            body:
                (await response.text()) +
                "\nmockSnapshot.dashboard = true; delete mockSnapshot.aiChat; delete mockSnapshot.fileTree;",
        });
    });
    await page.goto("/");
    await expect(page.locator(".dashboard")).toBeInViewport();
    await expect(
        page.getByRole("button", { name: "␠sf Find a file" }),
    ).toBeVisible();
    await expect(page.locator(".dashboard-logo strong")).toHaveText("ovim");
    await page.screenshot({ path: test.info().outputPath("dashboard.png") });
});

test("live grep shows the code around the selected match", async ({ page }) => {
    await page.route("**/src/mock.ts", async (route) => {
        const response = await route.fetch();
        await route.fulfill({
            response,
            body:
                (await response.text()) +
                `
delete mockSnapshot.aiChat;
mockSnapshot.picker = {
    title: "Search project", query: "render", selected: 0, total: 1, showsPreview: true,
    items: [{ index: 0, display: "src/app.rs:60:8", location: "/repo/src/app.rs", detail: "fn render(frame: &mut Frame)", matched: [] }],
    preview: {
        path: "src/app.rs", targetLine: 60, stale: false,
        lines: Array.from({length: 80}, (_, index) => ({
            number: index + 20, target: index + 20 === 60,
            segments: index + 20 === 60
                ? [{ text: "fn", token: "keyword" }, { text: " render(frame: &mut Frame) {" }]
                : [{ text: "    let value = " + (index + 20) + ";" }],
        })),
    },
};`,
        });
    });
    await page.goto("/");
    const picker = page.getByRole("dialog", { name: "render", exact: true });
    const preview = picker.getByRole("complementary", { name: "Preview" });
    await expect(preview.getByText("src/app.rs")).toBeVisible();
    await expect(preview.getByText("line 60")).toBeVisible();
    const target = preview.locator(".picker-preview-line.target");
    await expect(target).toContainText("fn render(frame: &mut Frame) {");
    await expect(target).toBeInViewport();
    await expect(target.getByText("fn", { exact: true })).toHaveCSS(
        "color",
        "rgb(192, 153, 255)",
    );

    // The matched line sits in the middle of the pane, not at an edge.
    const pane = await preview.locator(".picker-preview-code").boundingBox();
    const line = await target.boundingBox();
    const middle = pane!.y + pane!.height / 2;
    expect(Math.abs(line!.y + line!.height / 2 - middle)).toBeLessThan(
        line!.height,
    );
    await page.screenshot({
        path: test.info().outputPath("picker-preview.png"),
    });
});
