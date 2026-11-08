// @vitest-environment jsdom
import { fireEvent, render, waitFor } from "@solidjs/testing-library";
import { afterEach, describe, expect, it, vi } from "vitest";
import FlowDiff from "./FlowDiff";
import type { FlowDiffReview } from "./FlowDiffModel";

vi.mock("./FlowDiffExport", () => ({
    buildDiffExportImage: () => ({}),
    rasterizeDiffExportImage: async () => ({ filename: "review.png" }),
}));

const review: FlowDiffReview = {
    title: "review",
    layout: "split",
    managed: true,
    custom: false,
    files: [
        {
            id: "src/a.ts",
            path: "src/a.ts",
            status: "modified",
            additions: 2,
            deletions: 2,
            binary: false,
            metadata: [],
            hunks: [
                {
                    header: "@@ -1,3 +1,3 @@",
                    oldStart: 1,
                    oldCount: 3,
                    newStart: 1,
                    newCount: 3,
                    lines: [
                        {
                            kind: "context",
                            text: "needle ctx",
                            oldLine: 1,
                            newLine: 1,
                        },
                        { kind: "removed", text: "needle old", oldLine: 2 },
                        { kind: "added", text: "needle new", newLine: 2 },
                        {
                            kind: "context",
                            text: "tail ctx",
                            oldLine: 3,
                            newLine: 3,
                        },
                    ],
                },
                {
                    header: "@@ -30,2 +30,2 @@",
                    oldStart: 30,
                    oldCount: 2,
                    newStart: 30,
                    newCount: 2,
                    lines: [
                        { kind: "removed", text: "second old", oldLine: 30 },
                        { kind: "added", text: "second new", newLine: 30 },
                        {
                            kind: "context",
                            text: "second ctx",
                            oldLine: 31,
                            newLine: 31,
                        },
                    ],
                },
            ],
        },
    ],
};

const panelOf = (ui: ReturnType<typeof render>) =>
    ui.getByRole("region", { name: "Diff review" });
const editorLabel = (ui: ReturnType<typeof render>) =>
    ui.container.querySelector(".flow-comment-editor label")?.textContent;
const activeSections = (ui: ReturnType<typeof render>) =>
    [
        ...ui.container.querySelectorAll<HTMLElement>(
            ".flow-scroll.new [data-active=true]",
        ),
    ].map((element) => element.dataset.section);

afterEach(() => vi.useRealTimers());

describe("diff comment target (o / O)", () => {
    it("defaults to the first added line of the active change", () => {
        const ui = render(() => (
            <FlowDiff review={review} onComment={vi.fn()} />
        ));
        fireEvent.keyDown(panelOf(ui), { key: "o" });
        expect(editorLabel(ui)).toBe("Comment below src/a.ts:2");
        ui.unmount();
    });

    it("does not reuse a line selected in an earlier change", () => {
        const ui = render(() => (
            <FlowDiff review={review} onComment={vi.fn()} />
        ));
        fireEvent.click(
            ui.container.querySelector(
                '.flow-scroll.new [data-source-side="new"][data-source-line="1"]',
            )!,
        );
        fireEvent.keyDown(panelOf(ui), { key: "]" });
        fireEvent.keyDown(panelOf(ui), { key: "c" });
        fireEvent.keyDown(panelOf(ui), { key: "o" });
        expect(editorLabel(ui)).toBe("Comment below src/a.ts:30");
        ui.unmount();
    });

    it("ignores a selection outside the active change but honors one inside it", () => {
        const ui = render(() => (
            <FlowDiff review={review} onComment={vi.fn()} />
        ));
        // Line 31 is context of the second hunk; the first change is active.
        fireEvent.click(
            ui.container.querySelector(
                '.flow-scroll.new [data-source-side="new"][data-source-line="31"]',
            )!,
        );
        fireEvent.keyDown(panelOf(ui), { key: "O" });
        expect(editorLabel(ui)).toBe("Comment above src/a.ts:2");
        ui.unmount();
    });
});

describe("layout and search", () => {
    it("s keeps the active change in view like the toolbar buttons", async () => {
        const offsetTop = Object.getOwnPropertyDescriptor(
            HTMLElement.prototype,
            "offsetTop",
        );
        const ui = render(() => <FlowDiff review={review} />);
        fireEvent.keyDown(panelOf(ui), { key: "]" });
        fireEvent.keyDown(panelOf(ui), { key: "c" });
        // Let stepChange's own queued scroll finish before switching layout.
        await Promise.resolve();
        const [second] = activeSections(ui);
        Object.defineProperty(HTMLElement.prototype, "offsetTop", {
            configurable: true,
            get(this: HTMLElement) {
                return this.dataset.section === second &&
                    this.classList.contains("flow-unified-section")
                    ? 400
                    : 0;
            },
        });
        try {
            fireEvent.keyDown(panelOf(ui), { key: "s" });
            await Promise.resolve();
            expect(
                ui.getByRole("region", { name: "Unified changes" }).scrollTop,
            ).toBe(392);
            ui.unmount();
        } finally {
            if (offsetTop)
                Object.defineProperty(
                    HTMLElement.prototype,
                    "offsetTop",
                    offsetTop,
                );
        }
    });

    const search = async (ui: ReturnType<typeof render>, text: string) => {
        fireEvent.keyDown(panelOf(ui), { key: "/" });
        fireEvent.input(ui.getByRole("searchbox", { name: "Find in diff" }), {
            target: { value: text },
        });
        await waitFor(() =>
            expect(ui.getByRole("status", { name: "" })).toBeTruthy(),
        );
    };
    const currentText = (ui: ReturnType<typeof render>) =>
        ui.container.querySelector(".flow-search-current .flow-line-text")
            ?.textContent;

    it("split search counts shared context once, in document order", async () => {
        const ui = render(() => <FlowDiff review={review} />);
        await search(ui, "needle");
        await waitFor(() =>
            expect(ui.getByText("1 / 3 matching lines")).toBeTruthy(),
        );
        const seen = [currentText(ui)];
        const input = ui.getByRole("searchbox", { name: "Find in diff" });
        for (let i = 0; i < 2; i++) {
            fireEvent.keyDown(input, { key: "Enter" });
            seen.push(currentText(ui));
        }
        expect(seen).toEqual(["needle ctx", "needle old", "needle new"]);
        ui.unmount();
    });

    it("n and N step changes again once search is closed", async () => {
        const ui = render(() => <FlowDiff review={review} />);
        await search(ui, "needle");
        await waitFor(() =>
            expect(ui.getByText("1 / 3 matching lines")).toBeTruthy(),
        );
        const input = ui.getByRole("searchbox", { name: "Find in diff" });
        fireEvent.keyDown(input, { key: "Escape" });
        expect(ui.queryByRole("searchbox")).toBeNull();
        await waitFor(() => expect(currentText(ui)).toBeUndefined());
        const before = activeSections(ui);
        fireEvent.keyDown(panelOf(ui), { key: "n" });
        expect(activeSections(ui)).not.toEqual(before);
        ui.unmount();
    });
});

describe("old-side comments on shared context in unified layout", () => {
    const comment = {
        id: 3,
        path: "src/a.ts",
        line: 1,
        side: "old" as const,
        before: false,
        text: "about the old line",
        excerpt: "needle ctx",
    };

    it("renders the comment in split layout", () => {
        const ui = render(() => (
            <FlowDiff
                review={{ ...review, comments: [comment] }}
                onComment={vi.fn()}
            />
        ));
        expect(ui.getByText("about the old line")).toBeTruthy();
        expect(
            ui.container.querySelector(".flow-unplaced-comments"),
        ).toBeNull();
        ui.unmount();
    });

    it("lists it as unplaced when unified layout hides the old context line", () => {
        const ui = render(() => (
            <FlowDiff
                review={{ ...review, layout: "unified", comments: [comment] }}
                onComment={vi.fn()}
            />
        ));
        const unplaced = ui.container.querySelector(".flow-unplaced-comments")!;
        expect(unplaced.textContent).toContain("about the old line");
        ui.unmount();
    });

    it("lets the unplaced summary toggle from the keyboard", () => {
        const ui = render(() => (
            <FlowDiff
                review={{ ...review, layout: "unified", comments: [comment] }}
                onComment={vi.fn()}
            />
        ));
        const summary = ui.container.querySelector("summary")!;
        expect(fireEvent.keyDown(summary, { key: "Enter" })).toBe(true);
        expect(fireEvent.keyDown(summary, { key: " " })).toBe(true);
        ui.unmount();
    });
});

describe("mouse affordances", () => {
    it("keeps line numbers and gutter buttons out of the tab order", () => {
        const ui = render(() => (
            <FlowDiff
                review={{
                    ...review,
                    comments: [],
                }}
                onComment={vi.fn()}
                onExpandContext={vi.fn()}
            />
        ));
        for (const selector of [
            ".flow-line-number",
            ".flow-add-comment",
            ".flow-expand-context",
        ]) {
            const buttons = ui.container.querySelectorAll(selector);
            // Expand arrows need hunk context; the others always exist.
            if (selector !== ".flow-expand-context")
                expect(buttons.length).toBeGreaterThan(0);
            for (const button of buttons)
                expect(button.getAttribute("tabindex")).toBe("-1");
        }
        ui.unmount();
    });
});

describe("export status", () => {
    it("clears a success message after a few seconds", async () => {
        vi.useFakeTimers({ toFake: ["setTimeout", "clearTimeout"] });
        const ui = render(() => (
            <FlowDiff review={review} onExport={async () => true} />
        ));
        fireEvent.click(ui.getByRole("button", { name: "Export image" }));
        await vi.advanceTimersByTimeAsync(0);
        expect(ui.getByRole("status").textContent).toBe("Saved review.png");
        await vi.advanceTimersByTimeAsync(5000);
        expect(ui.queryByText("Saved review.png")).toBeNull();
        ui.unmount();
    });
});
