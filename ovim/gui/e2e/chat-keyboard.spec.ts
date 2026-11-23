import { expect, test, type Page } from "@playwright/test";

const harness = `
import { render } from '/node_modules/.vite/deps/solid-js_web.js';
import { createSignal } from '/node_modules/.vite/deps/solid-js.js';
import ChatComposer from '/src/ChatComposer.tsx';
import { mockSnapshot } from '/src/mock.ts';
import '/src/styles.css';
import '/src/tokens.css';

const input = 'one two 🌍 three';
const [chat, setChat] = createSignal({
    ...mockSnapshot.aiChat,
    activity: 'idle',
    waiting: false,
    input,
    inputCursor: new TextEncoder().encode(input).length,
    pendingImages: [],
});
window.chatHarness = {
    snapshot: () => setChat(current => ({ ...current, activity: current.activity === 'idle' ? 'working' : 'idle' })),
};
render(() => ChatComposer({
    get chat() { return chat(); },
    onUpdate: async update => setChat(current => ({ ...current, input: update.input, inputCursor: update.cursor })),
}), document.getElementById('root'));
`;

async function setup(page: Page) {
    await page.route("**/chat-keyboard-harness", (route) =>
        route.fulfill({
            contentType: "text/html",
            body: '<html><body><div id="root"></div><script type="module" src="/chat-keyboard-harness.js"></script></body></html>',
        }),
    );
    await page.route("**/chat-keyboard-harness.js", (route) =>
        route.fulfill({ contentType: "application/javascript", body: harness }),
    );
    await page.goto("/chat-keyboard-harness");
    return page.getByLabel("AI chat input");
}

test("word selection survives snapshots and arrows at text boundaries", async ({
    page,
}) => {
    const wordModifier = process.platform === "darwin" ? "Alt" : "Control";
    const input = await setup(page);
    await input.focus();
    await input.press("ControlOrMeta+End");
    await input.press(`${wordModifier}+Shift+ArrowLeft`);
    await expect
        .poll(() =>
            input.evaluate((element: HTMLTextAreaElement) => [
                element.selectionStart,
                element.selectionEnd,
            ]),
        )
        .toEqual([11, 16]);

    await page.evaluate(() => (window as any).chatHarness.snapshot());
    await expect
        .poll(() =>
            input.evaluate((element: HTMLTextAreaElement) => [
                element.selectionStart,
                element.selectionEnd,
            ]),
        )
        .toEqual([11, 16]);

    for (let index = 0; index < 8; index++) {
        await input.press(`${wordModifier}+Shift+ArrowLeft`);
    }
    await expect(input).toHaveValue("one two 🌍 three");
    await expect
        .poll(() =>
            input.evaluate(
                (element: HTMLTextAreaElement) => element.selectionStart,
            ),
        )
        .toBe(0);

    await input.press("ArrowRight");
    for (let index = 0; index < 8; index++) {
        await input.press(`${wordModifier}+ArrowRight`);
        await input.press("ArrowRight");
        await input.press("ArrowDown");
    }
    await expect(input).toHaveValue("one two 🌍 three");
    await expect
        .poll(() =>
            input.evaluate(
                (element: HTMLTextAreaElement) => element.selectionEnd,
            ),
        )
        .toBe(16);

    await input.press("ControlOrMeta+Home");
    for (let index = 0; index < 8; index++) {
        await input.press(`${wordModifier}+ArrowLeft`);
        await input.press("ArrowLeft");
        await input.press("ArrowUp");
    }
    await expect(input).toHaveValue("one two 🌍 three");
    await expect
        .poll(() =>
            input.evaluate(
                (element: HTMLTextAreaElement) => element.selectionStart,
            ),
        )
        .toBe(0);
});
