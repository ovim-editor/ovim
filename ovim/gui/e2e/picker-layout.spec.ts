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
