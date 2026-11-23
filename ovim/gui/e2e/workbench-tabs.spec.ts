import { expect, test } from "@playwright/test";

test("tab close buttons appear on hover and active tabs without covering titles", async ({
    page,
}) => {
    await page.goto("/");
    const list = page.getByRole("tablist", { name: "Open tabs" });
    const tabs = list.getByRole("tab");
    await expect(tabs).toHaveCount(2);
    // The restore and + buttons are outside the tablist.
    await expect(list.getByRole("button")).toHaveCount(2);
    await expect(
        page.getByRole("button", { name: "New browser tab" }),
    ).toBeVisible();
    await expect(
        list.getByRole("button", { name: "New browser tab" }),
    ).toHaveCount(0);

    const inactive = list.locator(".tab-item:not(.active)").first();
    const close = inactive.getByRole("button", { name: /^Close / });
    await expect(close).toHaveCSS("opacity", "0");
    await inactive.hover();
    await expect(close).toHaveCSS("opacity", "1");

    const item = (await inactive.boundingBox())!;
    const box = (await close.boundingBox())!;
    const title = (await inactive.locator(".tab > span").boundingBox())!;
    expect(box.x).toBeGreaterThanOrEqual(item.x);
    expect(box.x + box.width).toBeLessThanOrEqual(item.x + item.width);
    expect(title.x + title.width).toBeLessThanOrEqual(box.x);

    const active = list.locator(".tab-item.active");
    await expect(active.getByRole("button", { name: /^Close / })).toHaveCSS(
        "opacity",
        "1",
    );
    await page.screenshot({
        path: test.info().outputPath("tab-close-buttons.png"),
    });
});
