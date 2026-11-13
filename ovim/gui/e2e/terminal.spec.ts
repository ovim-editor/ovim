import { expect, test } from "@playwright/test";

test("the activity bar and shortcut toggle the terminal", async ({
    page,
}, testInfo) => {
    await page.goto("/");
    const shortcut = await page.evaluate(() =>
        /Mac|iPhone|iPad/.test(navigator.platform)
            ? "Meta+Shift+T"
            : "Control+Shift+T",
    );
    const button = page.getByRole("button", { name: "Terminal", exact: true });

    await button.click();
    await expect(
        page.getByText("Terminal is available in the desktop app."),
    ).toBeVisible();
    await page.screenshot({
        path: testInfo.outputPath("terminal-preview.png"),
    });

    await page.keyboard.press(shortcut);
    await expect(page.locator(".workbench")).toHaveClass(/context-collapsed/);
    await page.keyboard.press(shortcut);
    await expect(
        page.getByText("Terminal is available in the desktop app."),
    ).toBeVisible();
    await expect(
        page.getByRole("button", { name: "Diff review", exact: true }),
    ).toHaveCount(0);
});
