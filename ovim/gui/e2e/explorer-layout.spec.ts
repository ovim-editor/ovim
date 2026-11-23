import { expect, test, type Page } from "@playwright/test";

// Exercise the real component with deterministic core projections and actions.
// Every filename here is synthetic public test data.
const harness = `
import { render } from '/node_modules/.vite/deps/solid-js_web.js';
import { createSignal } from '/node_modules/.vite/deps/solid-js.js';
import FileExplorer from '/src/FileExplorer.tsx';
import { themeVariables } from '/src/theme.ts';
import { mockSnapshot } from '/src/mock.ts';
for (const [key, value] of Object.entries(themeVariables(mockSnapshot.theme))) document.documentElement.style.setProperty(key, value);
import '/src/styles.css';
import '/src/tokens.css';
const items = Array.from({length: 420}, (_, index) => ({
    index, name: index === 350 ? 'beginning-' + 'long-module-'.repeat(12) + '-ending.test.ts' : 'module-' + index + '.ts',
    path: '/workspace/module-' + index, depth: index >= 300 ? 24 : 1, directory: false, expanded: false
}));
const [tree, setTree] = createSignal({root: 'workspace', selected: 340, revealGeneration: 0, items});
const [width, setWidth] = createSignal(320);
window.explorer = {
    select: selected => setTree(previous => ({...previous, selected})),
    reveal: () => setTree(previous => ({...previous, revealGeneration: previous.revealGeneration + 1})),
    refresh: () => setTree(previous => ({...previous})),
};
render(() => FileExplorer({get tree() { return tree() }, get width() { return width() }, active: true,
    onWidthChange: setWidth, onSelect: selected => window.explorer.select(selected)
}), document.getElementById('root'));
`;

async function setup(page: Page) {
    await page.route("**/explorer-harness", (route) =>
        route.fulfill({
            contentType: "text/html",
            body: '<html><body><div id="root" style="height:600px;display:flex"></div><script type="module" src="/explorer-harness.js"></script></body></html>',
        }),
    );
    await page.route("**/explorer-harness.js", (route) =>
        route.fulfill({ contentType: "application/javascript", body: harness }),
    );
    await page.goto("/explorer-harness");
    await expect(
        page.getByRole("treeitem", { name: "module-340.ts", exact: true }),
    ).toBeInViewport();
}
async function action(
    page: Page,
    name: "select" | "reveal" | "refresh",
    selected?: number,
) {
    await page.evaluate(
        ({ name, selected }) => (window as any).explorer[name](selected),
        { name, selected },
    );
}
async function expectFullLabel(page: Page) {
    await expect
        .poll(() =>
            page
                .locator('[aria-selected="true"] .tree-name-full')
                .evaluate((label) => {
                    const list = document.querySelector(".tree-list")!;
                    const bounds = list.getBoundingClientRect();
                    const name = label.getBoundingClientRect();
                    return (
                        name.left >= bounds.left &&
                        name.right <= bounds.left + list.clientWidth
                    );
                }),
        )
        .toBe(true);
}

test("reveal and navigation position deep filenames on both axes without fighting manual scroll", async ({
    page,
}) => {
    await setup(page);
    await expectFullLabel(page);
    const list = page.locator(".tree-list");
    await list.evaluate((element) => {
        element.scrollTop = 0;
        element.scrollLeft = 0;
    });
    await action(page, "refresh");
    await expect
        .poll(() =>
            list.evaluate((element) => [element.scrollLeft, element.scrollTop]),
        )
        .toEqual([0, 0]);
    await action(page, "reveal");
    await expect(
        page.getByRole("treeitem", { name: "module-340.ts", exact: true }),
    ).toBeInViewport();
    await expectFullLabel(page);
    await action(page, "select", 419);
    await expect(
        page.getByRole("treeitem", { name: "module-419.ts", exact: true }),
    ).toBeInViewport();
    await expectFullLabel(page);
    await page.screenshot({
        path: test.info().outputPath("deep-selection.png"),
    });
});

test("oversized selection shows both endpoints and manual scrolling exposes the full name", async ({
    page,
}) => {
    await setup(page);
    await action(page, "select", 350);
    const summary = page.locator(".tree-name-summary");
    await expect(summary).toHaveText(/^beginning-.*….*-ending\.test\.ts$/);
    await expect
        .poll(() =>
            summary.evaluate((label) => {
                const viewport = document
                    .querySelector(".tree-list")!
                    .getBoundingClientRect();
                const bounds = label.getBoundingClientRect();
                return (
                    bounds.left >= viewport.left &&
                    bounds.right <= viewport.right
                );
            }),
        )
        .toBe(true);
    await page.screenshot({
        path: test.info().outputPath("long-selection.png"),
    });
    await page.locator(".tree-list").evaluate((element) => {
        element.scrollLeft += 100;
    });
    await expect(summary).toHaveCount(0);
    await expect(
        page.locator('[aria-selected="true"] .tree-name-full'),
    ).toBeVisible();
    await action(page, "reveal");
    await expect(summary).toBeVisible();
});

test("divider supports drag, keyboard, reset, and narrow windows", async ({
    page,
}) => {
    await setup(page);
    const divider = page.getByRole("separator", { name: "Explorer width" });
    const bounds = (await divider.boundingBox())!;
    await page.mouse.move(bounds.x + 3, bounds.y + 100);
    await page.mouse.down();
    await page.mouse.move(bounds.x + 153, bounds.y + 100);
    await page.mouse.up();
    await expect(divider).toHaveAttribute("aria-valuenow", "470");
    await divider.focus();
    await page.keyboard.press("ArrowLeft");
    await expect(divider).toHaveAttribute("aria-valuenow", "460");
    await page.keyboard.press("Enter");
    await expect(divider).toHaveAttribute("aria-valuenow", "320");
    await page.keyboard.press("End");
    await page.setViewportSize({ width: 720, height: 768 });
    await expect(divider).toHaveAttribute("aria-valuenow", "660");
    await expectFullLabel(page);
    await page.setViewportSize({ width: 1440, height: 900 });
    await expect(divider).toHaveAttribute("aria-valuenow", "800");
});

test("workbench remembers resized width across reloads", async ({ page }) => {
    await page.route("**/src/mock.ts", async (route) => {
        const response = await route.fetch();
        await route.fulfill({
            response,
            body: (await response.text()) + "\ndelete mockSnapshot.aiChat;",
        });
    });
    await page.goto("/");
    const divider = page.getByRole("separator", { name: "Explorer width" });
    await expect(divider).toHaveAttribute("aria-valuenow", "320");
    await divider.focus();
    await page.keyboard.press("Shift+ArrowRight");
    await expect(divider).toHaveAttribute("aria-valuenow", "360");
    await page.reload();
    await expect(divider).toHaveAttribute("aria-valuenow", "360");
    await page.screenshot({ path: test.info().outputPath("workbench.png") });
});

for (const initiallyOpen of [false, true]) {
    test(`native snapshots reveal the Explorer over another dock (initially open: ${initiallyOpen})`, async ({
        page,
    }) => {
        await page.setViewportSize({ width: 1280, height: 800 });
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
                window.guiCommands = [];
                export const invoke = async (command, args) => {
                    window.guiCommands.push({command, args});
                    if (command !== 'gui_subscribe') return;
                    let revision = mockSnapshot.revision + 1;
                    args.onEvent.onmessage({...mockSnapshot, revision, fileTree: ${initiallyOpen} ? mockSnapshot.fileTree : undefined});
                    window.revealExplorer = () => args.onEvent.onmessage({...mockSnapshot, revision: ++revision,
                        fileTree: {...mockSnapshot.fileTree, revealGeneration: revision}});
                };
            `,
                }),
        );
        await page.goto("/");
        await expect(page.locator(".workbench")).toHaveClass(
            /active-context-dock/,
        );
        await page.evaluate(() => (window as any).revealExplorer());
        await expect(
            page.getByRole("tree", { name: "Project files" }),
        ).toBeVisible();
        await expect(page.locator(".workbench")).toHaveClass(
            /active-explorer-dock/,
        );
        const selected = page.locator(
            '[role="treeitem"][aria-selected="true"]',
        );
        await expect(selected).toBeInViewport();
        await selected.dblclick();
        await expect
            .poll(() =>
                page.evaluate(
                    () =>
                        (window as any).guiCommands
                            .filter(
                                (entry: any) =>
                                    entry.command === "gui_select_file_tree",
                            )
                            .at(-1)?.args.activate,
                ),
            )
            .toBe(true);
        const initialColumns = await page.evaluate(
            () =>
                (window as any).guiCommands
                    .filter((entry: any) => entry.command === "gui_snapshot")
                    .at(-1)?.args.columns,
        );
        await page.getByRole("separator", { name: "Explorer width" }).focus();
        await page.keyboard.press("Shift+ArrowRight");
        await expect
            .poll(() =>
                page.evaluate(
                    () =>
                        (window as any).guiCommands
                            .filter(
                                (entry: any) =>
                                    entry.command === "gui_snapshot",
                            )
                            .at(-1)?.args.columns,
                ),
            )
            .toBeLessThan(initialColumns);
    });
}

test("maximum Explorer width reserves space for the editor beside a context dock", async ({
    page,
}) => {
    await page.goto("/");
    await expect(page.locator(".side-dock")).toBeVisible();
    const divider = page.getByRole("separator", { name: "Explorer width" });
    await divider.focus();
    await page.keyboard.press("End");
    await expect
        .poll(
            async () => (await page.locator(".pane-tree").boundingBox())!.width,
        )
        .toBeGreaterThanOrEqual(490);
    await expect(page.locator(".side-dock")).toBeVisible();
});
