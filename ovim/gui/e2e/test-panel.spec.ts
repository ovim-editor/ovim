import { expect, test } from "@playwright/test";

for (const viewportWidth of [1600, 1200, 800]) {
    test(`test output resizes and restores its width at ${viewportWidth}px`, async ({
        page,
    }, testInfo) => {
        await page.setViewportSize({ width: viewportWidth, height: 900 });
        await page.route("**/src/mock.ts", async (route) => {
            const response = await route.fetch();
            await route.fulfill({
                response,
                body:
                    (await response.text()) +
                    `
                delete mockSnapshot.aiChat;
                mockSnapshot.testPanel = {scope: "file", directory: "sample", command: "pytest test_example.py", status: "failed", elapsedMs: 1200, truncated: 0,
                    lines: Array.from({length: 80}, (_, i) => i + " 测试 / æøå: assertion expected true but received false " + "long output ".repeat(12)), summary: "1 failed, 2 passed"};
            `,
            });
        });
        await page.goto("/");
        const output = page.getByRole("region", { name: "Test output" });
        const dock = page.locator(".side-dock");
        const separator = page.getByRole("separator", {
            name: "Context panel width",
        });
        await expect(output).toBeVisible();
        await expect(
            page.getByRole("region", { name: "Run console" }),
        ).toHaveCount(0);
        const before = (await dock.boundingBox())!;
        const handle = (await separator.boundingBox())!;
        await page.mouse.move(handle.x + handle.width / 2, handle.y + 200);
        await page.mouse.down();
        await page.mouse.move(
            handle.x + handle.width / 2 - 70,
            handle.y + 200,
            { steps: 8 },
        );
        await page.mouse.up();
        await expect
            .poll(async () => (await dock.boundingBox())!.width)
            .toBe(before.width + 70);
        await page.reload();
        await expect
            .poll(async () => (await dock.boundingBox())!.width)
            .toBe(before.width + 70);
        await separator.focus();
        await page.keyboard.press("ArrowRight");
        await expect
            .poll(async () => (await dock.boundingBox())!.width)
            .toBe(before.width + 60);
        await expect(
            output.getByRole("button", { name: "Rerun", exact: true }),
        ).toBeInViewport();
        await page.screenshot({ path: testInfo.outputPath("test-output.png") });
    });
}
