import { expect, test } from "@playwright/test";

for (const lines of [1, 8, 30]) {
    test(`typing in a ${lines}-line composer preserves the transcript viewport`, async ({
        page,
    }, testInfo) => {
        await page.route("**/src/mock.ts", async (route) => {
            const response = await route.fetch();
            await route.fulfill({
                response,
                body:
                    (await response.text()) +
                    `
Object.assign(mockSnapshot.aiChat, {
    activity: "idle", waiting: false, input: "line\\n".repeat(${lines - 1}) + "draft", inputCursor: ${(lines - 1) * 5 + 5},
    pendingImages: [], queuedInputs: [], approval: undefined, setup: undefined,
    codeExplanation: undefined, streaming: undefined, streamingThinking: undefined,
    messages: Array.from({length: 20}, (_, index) => ({
        id: "message-" + index, index, role: "assistant", selected: false,
        content: "Response " + index + "\\n\\nSome context to keep the transcript scrollable.",
        model: "test", tools: [], images: []
    }))
});`,
            });
        });
        await page.goto("/");
        const input = page.getByLabel("AI chat input");
        const transcript = page.locator(".chat-messages");
        await input.focus();
        await input.press("ControlOrMeta+End");
        for (const position of ["bottom", "history"] as const) {
            await transcript.evaluate((element, position) => {
                element.scrollTop =
                    position === "bottom" ? element.scrollHeight : 150;
            }, position);
            const before = await transcript.evaluate((element) => ({
                top: element.scrollTop,
                height: element.clientHeight,
            }));
            for (const character of "typing") {
                await input.press(character);
                await expect
                    .poll(() =>
                        transcript.evaluate((element) => ({
                            top: element.scrollTop,
                            height: element.clientHeight,
                        })),
                    )
                    .toEqual(before);
            }
        }
        await page.screenshot({ path: testInfo.outputPath("typing.png") });
        // Real content changes must still grow, cap and shrink the composer.
        await input.fill("short");
        const shortHeight = await input.evaluate(
            (element) => element.clientHeight,
        );
        await input.fill("new line\n".repeat(8));
        expect(
            await input.evaluate((element) => element.clientHeight),
        ).toBeGreaterThan(shortHeight);
        await input.fill("new line\n".repeat(30));
        expect(await input.evaluate((element) => element.clientHeight)).toBe(
            220,
        );
        await input.fill("short again");
        expect(await input.evaluate((element) => element.clientHeight)).toBe(
            shortHeight,
        );
    });
}
