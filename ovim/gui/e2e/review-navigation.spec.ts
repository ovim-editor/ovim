import { expect, test, type Page } from "@playwright/test";
import { readFile } from "node:fs/promises";

async function setup(page: Page, custom: boolean, layout: "split" | "unified") {
    await page.route("**/src/mock.ts", async route => {
        const response = await route.fetch();
        await route.fulfill({response, body: `${await response.text()}
        delete mockSnapshot.aiChat; delete mockSnapshot.fileTree;
        mockSnapshot.tabs = [{id: 1, index: 0, active: true, title: 'Diff', modified:false}, {id: 2, index: 1, active: false, title: 'source.rs', modified:false}];
        const files = ['src/日本語.rs', 'src/second.rs'].map((path, i) => ({id: path, path, status: 'modified', additions: 1, deletions: 1, binary: false, metadata: [], hunks: [{header: '@@ -1 +1 @@', oldStart: 1, oldCount: 1, newStart: 1, newCount: 1, lines: [{kind: 'removed', text: 'old_function();', oldLine: 1}, {kind: 'added', text: '/*😀*/ new_function();', newLine: 1}]}]}));
        mockSnapshot.panes = [{...mockSnapshot.panes[0], diffReview: {title: 'Review progress', layout: '${layout}', managed: true, custom: ${custom}, files, guidedFiles: ${custom} ? files.map((f,i) => ({...f, id:'pair_'+i, label:'Move '+i})) : undefined, overlay: ${custom} ? {mode:'saved'} : undefined}}];`});
    });
    await page.route("**/node_modules/.vite/deps/@tauri-apps_api_event.js*", route => route.fulfill({contentType: "application/javascript", body: "export const listen = async () => () => {};"}));
    await page.route("**/node_modules/.vite/deps/@tauri-apps_api_core.js*", route => route.fulfill({contentType: "application/javascript", body: `
        import { mockSnapshot } from '/src/mock.ts';
        export const isTauri = () => true;
        export class Channel {}
        window.reviewCommands = [];
        let listener, current = mockSnapshot;
        export const invoke = async (command, args) => {
            window.reviewCommands.push({command, args});
            if (command === 'gui_subscribe') { listener = args.onEvent; listener.onmessage(current); }
            if (command === 'gui_diff_action') {
                const next = structuredClone(current), doc = next.panes[0].diffReview;
                if (args.action === 'show_checked') doc.showChecked = !doc.showChecked;
                if (args.action.startsWith('check:')) {
                    const guided = args.action.startsWith('check:section:');
                    const id = args.action.slice(guided ? 14 : 11);
                    const file = (guided ? doc.guidedFiles : doc.files).find(f => f.id === id);
                    file.checked = !file.checked;
                }
                next.revision++; current = next; listener.onmessage(next);
            }
        };` }));
    await page.goto("/");
    await expect(page.getByRole("region", {name: "Diff review"})).toBeVisible();
}

for (const custom of [false, true]) for (const layout of ["split", "unified"] as const) {
    test(`review checks restore and gd carries symbol coordinates: ${custom ? "custom" : "regular"} ${layout}`, async ({page}, info) => {
        await setup(page, custom, layout);
        const first = custom ? 'Move 0' : 'src/日本語.rs';
        await page.getByRole('checkbox', {name: `Reviewed ${first}`, exact:true}).first().click();
        await expect(page.getByRole('checkbox', {name: `Reviewed ${first}`, exact:true})).toHaveCount(0);
        await page.getByRole('button', {name:'Show reviewed (1)'}).click();
        await expect(page.getByRole('checkbox', {name:`Reviewed ${first}`, exact:true}).first()).toBeChecked();
        await page.getByRole('checkbox', {name:`Reviewed ${first}`, exact:true}).first().click();
        await page.getByRole('button', {name:'Hide reviewed (0)'}).click();
        const code = page.locator('.flow-code-line.added .flow-line-text').first();
        await code.evaluate(element => {
            const walker = document.createTreeWalker(element, NodeFilter.SHOW_TEXT);
            let remaining = 9, node;
            while ((node = walker.nextNode())) {
                if (remaining <= node.textContent!.length) { window.getSelection()!.collapse(node, remaining); break; }
                remaining -= node.textContent!.length;
            }
            (element.closest('.flow-diff') as HTMLElement).focus();
        });
        await page.keyboard.press('g'); await page.keyboard.press('d');
        await expect.poll(() => page.evaluate(() => (window as any).reviewCommands.find((c:any) => c.command === 'gui_open_diff_source')?.args)).toMatchObject({path:'src/日本語.rs', line:1, side:'new', definitionColumn:9});
        await page.keyboard.press('Control+Tab');
        await expect.poll(() => page.evaluate(() => (window as any).reviewCommands.filter((c:any) => c.command === 'gui_select_tab').at(-1)?.args.index)).toBe(1);
        await page.screenshot({path: info.outputPath('review.png')});
    });
}

test('embedded page forwards Ctrl+Tab from inputs with Vim disabled', async ({page}) => {
    await page.goto('/');
    await page.setContent('<input aria-label="Page input" />');
    await page.evaluate(() => { (window as any).bridgeUrls = []; window.open = ((url: string) => { (window as any).bridgeUrls.push(url); return null; }) as typeof window.open; });
    const parts = await Promise.all(['bootstrap', 'hints', 'chrome', 'keyboard'].map(part => readFile(`../src/gui/browser/key_bridge/${part}.js`, 'utf8')));
    await page.addScriptTag({content: parts.join('').replaceAll('__OVIM_BRIDGE_TOKEN__','abc123').replaceAll('__OVIM_STATE_TOKEN__','def456').replaceAll('__OVIM_VIM_KEYS_ENABLED__','false')});
    await page.getByRole('textbox').focus();
    await page.keyboard.press('Control+Tab');
    await page.keyboard.press('Control+Shift+Tab');
    await expect.poll(() => page.evaluate(() => (window as any).bridgeUrls)).toEqual([
        'ovim-browser://key/abc123/next_workbench_tab',
        'ovim-browser://key/abc123/previous_workbench_tab',
    ]);
});

test('browser command line owns focus, errors remain editable, and Ctrl+Tab leaves the browser', async ({page}, info) => {
    await page.route("**/node_modules/.vite/deps/@tauri-apps_api_event.js*", route => route.fulfill({contentType: "application/javascript", body: `window.browserListeners = {}; export const listen = async (name, callback) => { window.browserListeners[name] = callback; return () => {}; };`}));
    await page.route("**/node_modules/.vite/deps/@tauri-apps_api_core.js*", route => route.fulfill({contentType: "application/javascript", body: `
        import { mockSnapshot } from '/src/mock.ts';
        export const isTauri = () => true;
        export class Channel {}
        window.browserCommands = [];
        const browser = {revision:1, sessions:[{sessionId:'browser-1', title:'Example', url:'https://example.com/', visible:true, loading:false, documentId:1, vimKeysEnabled:true, keyMode:'normal'}], activeSessionId:'browser-1', maxSessions:8};
        export const invoke = async (command,args) => {
            window.browserCommands.push({command,args});
            if (command === 'gui_subscribe') args.onEvent.onmessage(mockSnapshot);
            if (command === 'gui_browser_subscribe') args.onEvent.onmessage(browser);
            if (command === 'gui_browser_activate') return browser;
        };` }));
    await page.goto('/');
    await page.getByRole('tab', {name:'Example'}).click();
    await page.evaluate(() => (window as any).browserListeners['ovim://browser-key']({payload:{sessionId:'browser-1', intent:'command'}}));
    const input = page.getByRole('textbox', {name:'browser command input'});
    await expect(input).toBeFocused();
    await input.pressSequentially('w');
    await input.press('Enter');
    await expect(page.getByRole('alert')).toContainText(':write is unavailable');
    await expect(input).toBeFocused();
    await input.fill('reload');
    await input.press('Enter');
    await expect(input).toHaveCount(0);
    await expect.poll(() => page.evaluate(() => (window as any).browserCommands.some((c:any) => c.command === 'gui_browser_toolbar' && c.args.action === 'reload'))).toBe(true);
    await page.keyboard.press(':');
    await expect(input).toBeFocused();
    await input.press('Control+Shift+Tab');
    await expect(input).toHaveCount(0);
    await expect(page.getByRole('tab', {name:'Example'})).toHaveAttribute('aria-selected','false');
    await page.screenshot({path:info.outputPath('browser-command-return.png')});
});
