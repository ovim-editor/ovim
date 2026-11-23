import { readFile } from "node:fs/promises";
import { expect, test } from "@playwright/test";

for (const width of [1440, 760])
    test(`diff comments compose, edit, attach and clear at ${width}px`, async ({
        page,
    }, info) => {
        const native = process.env.OVIM_COMMENT_QA_FIXTURE
            ? JSON.parse(
                  await readFile(process.env.OVIM_COMMENT_QA_FIXTURE, "utf8"),
              )
            : null;
        await page.setViewportSize({ width, height: 900 });
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
        let listener;
        const chat = {...mockSnapshot.aiChat, input: '', inputCursor: 0, messages: [], waiting: false, activity: 'idle'};
        delete mockSnapshot.aiChat; delete mockSnapshot.fileTree;
        mockSnapshot.readOnly = true; mockSnapshot.mode = 'NORMAL';
        const document = {title: 'main → payment validation', layout: 'split', managed: true, custom: false, comments: [], files: [{id: 'file:payment.ts', path: 'payment.ts', status: 'modified', additions: 1, deletions: 1, binary: false, metadata: [], hunks: [{header: '@@ -1,3 +1,3 @@', oldStart: 1, oldCount: 3, newStart: 1, newCount: 3, lines: [
            {kind:'context',text:'function acceptPayment(amount: number) {',oldLine:1,newLine:1},
            {kind:'removed',text:'  return amount >= 0;',oldLine:2},
            {kind:'added',text:'  return amount > 0;',newLine:2},
            {kind:'context',text:'}',oldLine:3,newLine:3}
        ]}]}]};
        mockSnapshot.panes = [{...mockSnapshot.panes[0], diffReview: document}];
        const native = ${JSON.stringify(native)};
        if (native) mockSnapshot.panes[0].diffReview = native.initial;
        const state = structuredClone(mockSnapshot);
        const review = state.panes[0].diffReview;
        window.commentCommands = [];
        const publish = () => { state.revision++; listener?.onmessage(structuredClone(state)); };
        export const invoke = async (command, args) => {
            window.commentCommands.push({command,args});
            if (command === 'gui_subscribe') { listener = args.onEvent; publish(); }
            if (command === 'gui_diff_comment') {
                const c = args.comment;
                if (c.id) review.comments = review.comments.map(old => old.id === c.id ? {...old,...c} : old);
                else review.comments.push({...c,id:review.comments.length+1,excerpt:c.side === 'old' ? '  return amount >= 0;' : '  return amount > 0;'});
                if (native) Object.assign(review, structuredClone(c.id ? native.edited : native.added));
                publish();
            }
            if (command === 'gui_diff_action') {
                if (args.action === 'clear_comments') review.comments = [];
                else if (args.action.startsWith('remove_comment:')) review.comments = review.comments.filter(c => c.id !== Number(args.action.split(':')[1]));
                else if (['split','unified'].includes(args.action)) review.layout = args.action;
                publish();
            }
            if (command === 'gui_open_ai_chat') {
                state.mode = 'AI_CHAT'; state.aiChat = chat;
                chat.pendingDiffComments = native?.attachment ?? review.comments.map(c => c.path+':'+c.line+' ('+c.side+' side)\\n'+c.excerpt+'\\n'+c.text).join('\\n'); publish();
            }
            if (command === 'gui_editor_command' && args.command === 'DiffCommentsDetach') {chat.pendingDiffComments = null;publish();}
            if (command === 'gui_update_chat_input') {
                chat.input = args.input; chat.inputCursor = args.cursor;
                if (args.action?.key === 'Enter') { window.sentComments = chat.pendingDiffComments; review.comments = []; chat.pendingDiffComments = null; chat.input = ''; chat.inputCursor = 0; }
                publish();
            }
        };
    `,
                }),
        );
        await page.goto("/");
        const changed = page.locator(
            '[data-source-side="new"][data-source-line="2"]',
        );
        await changed.click();
        await page.locator(".flow-diff").press("O");
        const input = page.getByRole("textbox", { name: "Diff comment" });
        await expect(input).toBeFocused();
        await input.fill("Should zero still be valid?\nHva med refusjoner?");
        await input.press("Escape");
        await expect(input).toHaveCount(0);
        await info.attach("commands", {
            body: JSON.stringify(
                await page.evaluate(() => (window as any).commentCommands),
            ),
            contentType: "application/json",
        });
        await expect
            .poll(() =>
                page.evaluate(
                    () =>
                        (window as any).commentCommands.filter(
                            (c: any) => c.command === "gui_diff_comment",
                        ).length,
                ),
            )
            .toBe(1);
        await expect(
            page.getByText("Should zero still be valid?\nHva med refusjoner?", {
                exact: true,
            }),
        ).toBeVisible();
        await page.getByRole("button", { name: "Edit", exact: true }).click();
        await input.fill("Why reject zero?\n第二个问题：负数呢？");
        await input.press("Control+Enter");
        await page.screenshot({ path: info.outputPath("diff-comments.png") });
        await page
            .getByRole("button", { name: "Unified", exact: true })
            .click();
        await expect(
            page.getByText("Why reject zero?\n第二个问题：负数呢？", {
                exact: true,
            }),
        ).toBeVisible();
        await page
            .getByRole("button", { name: "AI chat", exact: true })
            .click();
        await page.locator(".chat-diff-comments summary").click();
        await expect(page.locator(".chat-diff-comments pre")).toContainText(
            native
                ? "payment.ts:2 (new side, before line)"
                : "payment.ts:2 (new side)",
        );
        await expect(page.locator(".chat-diff-comments pre")).toContainText(
            "Why reject zero?",
        );
        await page.screenshot({
            path: info.outputPath("diff-comments-attached.png"),
        });
        const question = page.getByRole("textbox", { name: "AI chat input" });
        await question.fill("Explain these changes");
        await question.press("Enter");
        await expect(page.locator(".chat-diff-comments")).toHaveCount(0);
        await expect(page.locator(".flow-user-comment")).toHaveCount(0);
        expect(
            await page.evaluate(() => (window as any).sentComments),
        ).toContain("Why reject zero?");
        expect(
            await page.evaluate(
                () =>
                    (window as any).commentCommands.find(
                        (c: any) => c.command === "gui_diff_comment",
                    ).args,
            ),
        ).toMatchObject({
            pane: 0,
            bufferId: expect.any(Number),
            comment: { path: "payment.ts", line: 2, side: "new", before: true },
        });
    });
