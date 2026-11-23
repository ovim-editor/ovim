import { expect, test } from "@playwright/test";

test("Command+Q can be cancelled or confirmed", async ({ page }) => {
    await page.goto("/");

    await page.keyboard.press("Meta+q");
    const confirmation = page.getByRole("dialog", {
        name: "Save changes before leaving?",
    });
    await expect(confirmation).toBeVisible();

    await page.keyboard.press("Escape");
    await expect(confirmation).toBeHidden();

    await page.keyboard.press("Meta+q");
    await expect(confirmation).toBeVisible();
    await confirmation
        .getByRole("button", { name: "Quit Without Saving" })
        .click();
    await expect(confirmation).toBeHidden();
});
