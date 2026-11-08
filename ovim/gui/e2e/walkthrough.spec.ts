import { expect, test } from "@playwright/test";
for (const kind of ["code", "diff"] as const) {
    for (const long of [false, true]) {
        for (const width of [1440, 760]) {
            test(`${kind} walkthrough reading and containment at ${width}px (${long ? "long" : "short"})`, async ({
                page,
            }, testInfo) => {
                await page.setViewportSize({ width, height: 900 });
                const path = `packages/editor/src/navigation/${"LongFileName".repeat(20)}.test.ts`;
                await page.route("**/src/mock.ts", async (route) => {
                    const response = await route.fetch();
                    await route.fulfill({
                        response,
                        body: `${await response.text()}
                mockSnapshot.aiChat.codeExplanation = ${JSON.stringify({
                    current: 1,
                    total: 2,
                    answerInProgress: false,
                    page: {
                        kind,
                        title: "Replace the navigation guard",
                        oldPath: path,
                        newPath: path,
                        path,
                        startLine: 10,
                        endLine: 14,
                        comment:
                            `The file \`${path}\` owns navigation.\n\n` +
                            "Readable explanation with useful context.\n\n".repeat(
                                long ? 50 : 1,
                            ),
                    },
                    discussion: {
                        state: "navigating",
                        questionCount: 0,
                        latestFailed: false,
                    },
                })};
                if (${JSON.stringify(kind)} === "diff") {
                    const patch = ["--- src/navigation.ts", "+++ src/navigation.ts", "@@ -10,1 +10,1 @@", "-if (legacyGuard) return;", "+if (sharedGuard) return;"];
                    mockSnapshot.readOnly = true;
                    delete mockSnapshot.fileTree;
                    mockSnapshot.panes[0].lines = patch.map((text, index) => ({
                        number: index + 1, continuation: false, displayStart: 0,
                        current: false,
                        segments: [{text, cells: text.length, selected: false, cursor: false, searchMatch: false}],
                    }));
                    mockSnapshot.panes[0].totalLines = patch.length;
                }`,
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
                window.walkthroughCalls = [];
                export const invoke = async (command, data) => {
                    window.walkthroughCalls.push({command, data});
                    if (command === 'gui_subscribe') data.onEvent.onmessage(mockSnapshot);
                };
            `,
                        }),
                );
                await page.goto("/");
                const card = page.locator(".walkthrough-card");
                await expect(card).toBeVisible();
                const title = page.locator("#walkthrough-title");
                if (kind === "code") {
                    await expect(title).toHaveAttribute(
                        "title",
                        `${path}:10–14`,
                    );
                    await expect(title).toContainText(".test.ts:10–14");
                } else {
                    await expect(title).toContainText(
                        "Replace the navigation guard",
                    );
                    await expect(
                        page.locator(".walkthrough-layer.diff"),
                    ).toBeVisible();
                    await expect(page.locator(".code-viewport")).toContainText(
                        "-if (legacyGuard) return;",
                    );
                    await expect(page.locator(".code-viewport")).toContainText(
                        "+if (sharedGuard) return;",
                    );
                }
                for (const selector of [
                    ".walkthrough-card",
                    ".walkthrough-teaching",
                    ".walkthrough-card header",
                    ".walkthrough-card footer",
                ]) {
                    expect(
                        await page
                            .locator(selector)
                            .evaluate(
                                (el) => el.scrollWidth <= el.clientWidth + 1,
                            ),
                    ).toBe(true);
                }
                await expect
                    .poll(() =>
                        page.evaluate(
                            () =>
                                (window as any).walkthroughCalls.filter(
                                    (c: any) =>
                                        c.command ===
                                        "gui_position_walkthrough",
                                ).length,
                        ),
                    )
                    .toBeGreaterThan(0);
                const surface = await page
                    .locator(".walkthrough-layer")
                    .boundingBox();
                await page.mouse.move(surface!.x + 30, surface!.y + 25);
                await page.mouse.wheel(0, 100);
                await expect
                    .poll(() =>
                        page.evaluate(
                            () =>
                                (window as any).walkthroughCalls.filter(
                                    (c: any) =>
                                        c.command === "gui_scroll" &&
                                        c.data.lines > 0,
                                ).length,
                        ),
                    )
                    .toBeGreaterThan(0);
                await page.mouse.wheel(0, -100);
                await expect
                    .poll(() =>
                        page.evaluate(
                            () =>
                                (window as any).walkthroughCalls.filter(
                                    (c: any) =>
                                        c.command === "gui_scroll" &&
                                        c.data.lines < 0,
                                ).length,
                        ),
                    )
                    .toBeGreaterThan(0);
                if (long) {
                    const teaching = page.locator(".walkthrough-teaching");
                    const calls = await page.evaluate(
                        () =>
                            (window as any).walkthroughCalls.filter(
                                (c: any) =>
                                    c.command === "gui_key" ||
                                    c.command === "gui_scroll",
                            ).length,
                    );
                    await teaching.hover();
                    await page.mouse.wheel(0, 500);
                    await expect
                        .poll(() => teaching.evaluate((el) => el.scrollTop))
                        .toBeGreaterThan(0);
                    expect(
                        await page.evaluate(
                            () =>
                                (window as any).walkthroughCalls.filter(
                                    (c: any) =>
                                        c.command === "gui_key" ||
                                        c.command === "gui_scroll",
                                ).length,
                        ),
                    ).toBe(calls);
                }
                await page.screenshot({
                    path: testInfo.outputPath("walkthrough.png"),
                });
            });
        }
    }
}
