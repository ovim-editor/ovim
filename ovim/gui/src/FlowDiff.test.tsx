// @vitest-environment jsdom
import { fireEvent, render, waitFor } from "@solidjs/testing-library";
import { createSignal } from "solid-js";
import { describe, expect, it, vi } from "vitest";
import FlowDiff from "./FlowDiff";
import {
    mappedScrollTop,
    replacementParts,
    sectionsForFile,
    sectionsWithMoves,
    type FlowDiffReview,
} from "./FlowDiffModel";

const review: FlowDiffReview = {
    title: "main → working tree",
    layout: "split",
    managed: false,
    custom: false,
    files: [
        {
            id: "src/uneven.ts",
            path: "src/uneven.ts",
            status: "modified",
            additions: 1,
            deletions: 3,
            binary: false,
            metadata: [],
            hunks: [
                {
                    header: "@@ -4,4 +4,2 @@",
                    oldStart: 4,
                    oldCount: 4,
                    newStart: 4,
                    newCount: 2,
                    reviewLine: 20,
                    lines: [
                        {
                            kind: "context",
                            text: "same",
                            oldLine: 4,
                            newLine: 4,
                        },
                        {
                            kind: "removed",
                            text: "long version one",
                            oldLine: 5,
                        },
                        {
                            kind: "removed",
                            text: "long version two",
                            oldLine: 6,
                        },
                        {
                            kind: "removed",
                            text: "long version three",
                            oldLine: 7,
                        },
                        {
                            kind: "added",
                            text: "short version",
                            newLine: 5,
                            reviewLine: 23,
                        },
                    ],
                },
                {
                    header: "@@ -20 +18 @@",
                    oldStart: 20,
                    oldCount: 1,
                    newStart: 18,
                    newCount: 1,
                    reviewLine: 40,
                    lines: [
                        {
                            kind: "context",
                            text: "tail",
                            oldLine: 20,
                            newLine: 18,
                        },
                    ],
                },
            ],
        },
        {
            id: "asset.png",
            path: "asset.png",
            status: "modified",
            additions: 0,
            deletions: 0,
            binary: true,
            metadata: [],
            hunks: [],
        },
    ],
};

describe("flow diff model", () => {
    it("keeps unequal changes compact and pairs context and omitted ranges", () => {
        const sections = sectionsForFile(review.files[0]);
        expect(
            sections.map((section) => [
                section.kind,
                section.left.length,
                section.right.length,
            ]),
        ).toEqual([
            ["gap", 0, 0],
            ["context", 1, 1],
            ["change", 3, 1],
            ["gap", 0, 0],
            ["context", 1, 1],
        ]);
        expect(sections[3].label).toBe("12 unchanged lines");
    });

    it("maps scroll progress through sections of different heights", () => {
        expect(
            mappedScrollTop(
                75,
                [
                    { top: 0, height: 50 },
                    { top: 50, height: 100 },
                ],
                [
                    { top: 0, height: 50 },
                    { top: 50, height: 20 },
                ],
            ),
        ).toBe(55);
    });

    it("marks only replacement substrings without splitting Unicode characters", () => {
        expect(replacementParts("hello 👋 earth", "hello 🌍 earth")).toEqual([
            { text: "hello ", changed: false },
            { text: "👋", changed: true },
            { text: " earth", changed: false },
        ]);
        expect(replacementParts("an added line")).toEqual([
            { text: "an added line", changed: false },
        ]);
    });

    it("splits neighboring moved ranges while retaining unrelated canonical lines", () => {
        const file = {
            ...review.files[0],
            hunks: [
                {
                    header: "@@ -1,3 +1,4 @@",
                    oldStart: 1,
                    oldCount: 3,
                    newStart: 1,
                    newCount: 4,
                    lines: [
                        { kind: "removed" as const, text: "old 1", oldLine: 1 },
                        { kind: "removed" as const, text: "old 2", oldLine: 2 },
                        { kind: "removed" as const, text: "old 3", oldLine: 3 },
                        { kind: "added" as const, text: "new 1", newLine: 1 },
                        { kind: "added" as const, text: "new 2", newLine: 2 },
                        { kind: "added" as const, text: "new 3", newLine: 3 },
                        { kind: "added" as const, text: "new 4", newLine: 4 },
                    ],
                },
            ],
        };
        const endpoint = (path: string, startLine: number) => ({
            path,
            startLine,
            lineCount: 1,
            contextWindows: [],
            contextComplete: false,
        });
        const moves = [
            {
                id: "a",
                old: endpoint("other-a.ts", 10),
                new: endpoint(file.path, 2),
            },
            {
                id: "b",
                old: endpoint("other-b.ts", 20),
                new: endpoint(file.path, 4),
            },
        ];
        const sections = sectionsWithMoves(file, moves, "old");
        expect(
            sections
                .filter((section) => section.kind === "change")
                .map((section) => section.move?.id),
        ).toEqual([undefined, "a", undefined, "b"]);
        expect(
            sections.flatMap((section) =>
                section.left.map((line) => line.text),
            ),
        ).toEqual(["old 1", "old 2", "old 3"]);
        expect(
            sections.flatMap((section) =>
                section.right.map((line) => line.text),
            ),
        ).toEqual(["new 1", "new 2", "new 3", "new 4"]);
    });
});

describe("FlowDiff", () => {
    it("shows canonical files and follows a moved segment across reconstruction sides", () => {
        const open = vi.fn();
        const moved: FlowDiffReview = {
            ...review,
            custom: true,
            files: [
                {
                    id: "src/main.ts",
                    path: "src/main.ts",
                    status: "modified",
                    additions: 0,
                    deletions: 1,
                    binary: false,
                    metadata: [],
                    hunks: [
                        {
                            header: "@@ -20 +20,0 @@",
                            oldStart: 20,
                            oldCount: 1,
                            newStart: 20,
                            newCount: 0,
                            lines: [
                                {
                                    kind: "removed",
                                    text: "parse()",
                                    oldLine: 20,
                                },
                            ],
                        },
                    ],
                },
                {
                    id: "src/parser.ts",
                    path: "src/parser.ts",
                    status: "added",
                    additions: 1,
                    deletions: 0,
                    binary: false,
                    metadata: ["new mode 100644"],
                    hunks: [
                        {
                            header: "@@ -0,0 +3 @@",
                            oldStart: 0,
                            oldCount: 0,
                            newStart: 3,
                            newCount: 1,
                            lines: [
                                { kind: "added", text: "parse()", newLine: 3 },
                            ],
                        },
                    ],
                },
            ],
            moves: [
                {
                    id: "move-1",
                    label: "Extract parser",
                    old: {
                        path: "src/main.ts",
                        startLine: 20,
                        lineCount: 1,
                        contextComplete: true,
                        contextWindows: [
                            {
                                startLine: 19,
                                lines: [
                                    {
                                        kind: "context",
                                        text: "before",
                                        oldLine: 19,
                                    },
                                    {
                                        kind: "context",
                                        text: "parse()",
                                        oldLine: 20,
                                    },
                                    {
                                        kind: "context",
                                        text: "after",
                                        oldLine: 21,
                                    },
                                ],
                            },
                        ],
                    },
                    new: {
                        path: "src/parser.ts",
                        startLine: 3,
                        lineCount: 1,
                        contextComplete: true,
                        contextWindows: [
                            {
                                startLine: 2,
                                lines: [
                                    {
                                        kind: "context",
                                        text: "before",
                                        newLine: 2,
                                    },
                                    {
                                        kind: "context",
                                        text: "parse()",
                                        newLine: 3,
                                    },
                                    {
                                        kind: "context",
                                        text: "after",
                                        newLine: 4,
                                    },
                                ],
                            },
                        ],
                    },
                },
            ],
        };
        const result = render(() => (
            <FlowDiff review={moved} onOpenSource={open} />
        ));
        expect(result.getByText("2 files")).toBeTruthy();
        expect(
            result.container.querySelectorAll(
                ".flow-scroll.old .flow-file-group",
            ),
        ).toHaveLength(2);
        expect(result.getAllByText("new mode 100644")).toHaveLength(2);
        expect(result.getByText("Possible moved code")).toBeTruthy();
        expect(
            result.queryByRole("region", { name: "src/main.ts context" }),
        ).toBeNull();
        fireEvent.click(
            result.getByRole("button", { name: "Show moved-code matches" }),
        );
        expect(result.getByText("Possible match")).toBeTruthy();
        expect(result.getByText("src/main.ts:20")).toBeTruthy();
        expect(result.getByText("src/parser.ts:3")).toBeTruthy();
        expect(
            result.getByRole("region", { name: "src/main.ts context" }),
        ).toBeTruthy();
        fireEvent.click(
            result.getAllByRole("button", {
                name: "Before line 20, open source",
            })[0],
        );
        fireEvent.click(result.getByRole("button", { name: "After context" }));
        expect(
            result.container.querySelector(
                ".flow-section.change[data-file-id='src/main.ts'][data-active='true']",
            ),
        ).toBeTruthy();
        expect(
            result.getByText("After context · matched lines highlighted"),
        ).toBeTruthy();
        fireEvent.click(
            result.getAllByRole("button", {
                name: "After line 3, open source",
            })[0],
        );
        expect(open.mock.calls).toEqual([
            ["src/main.ts", 20, "old"],
            ["src/parser.ts", 3, "new"],
        ]);
    });

    it("shows independent compact streams and navigates source and changes", async () => {
        const navigate = vi.fn();
        const open = vi.fn();
        const result = render(() => (
            <FlowDiff
                review={review}
                onNavigateReviewLine={navigate}
                onOpenSource={open}
            />
        ));
        expect(
            result.container.querySelectorAll(
                ".flow-scroll.old .flow-code-line",
            ),
        ).toHaveLength(5);
        expect(
            result.container.querySelectorAll(
                ".flow-scroll.new .flow-code-line",
            ),
        ).toHaveLength(3);
        fireEvent.click(
            result.getByRole("button", { name: "Before line 5, open source" }),
        );
        expect(open).toHaveBeenCalledWith("src/uneven.ts", 5, "old");
        fireEvent.click(result.getByRole("button", { name: "Next change" }));
        await waitFor(() => expect(navigate).toHaveBeenCalledWith(23));
    });

    it("switches layout and handles binary files", () => {
        const layout = vi.fn();
        const result = render(() => (
            <FlowDiff review={review} onLayoutChange={layout} />
        ));
        fireEvent.click(result.getByRole("button", { name: "Unified" }));
        expect(layout).toHaveBeenCalledWith("unified");
        expect(
            result.container.querySelectorAll(
                ".flow-unified-scroll .flow-file-group",
            ),
        ).toHaveLength(2);
        expect(
            result.getByText("Binary file — no text diff to display."),
        ).toBeTruthy();
    });

    it("keeps review keys local and forwards command prefixes explicitly", () => {
        const action = vi.fn();
        const coreKey = vi.fn();
        const result = render(() => (
            <FlowDiff review={review} onAction={action} onCoreKey={coreKey} />
        ));
        const before = result.getByRole("region", { name: "Before changes" });
        fireEvent.keyDown(before, { key: " " });
        fireEvent.keyDown(before, { key: ":" });
        fireEvent.keyDown(before, { key: "q" });
        expect(coreKey.mock.calls).toEqual([[" "], [":"]]);
        expect(action).toHaveBeenCalledWith("q");
        fireEvent.click(result.getByRole("button", { name: "Refresh" }));
        expect(action).toHaveBeenCalledWith("r");
    });

    it("clicks the exact changed section and updates the counter on scroll", async () => {
        const navigate = vi.fn();
        const file = review.files[0];
        const document: FlowDiffReview = {
            ...review,
            files: [
                {
                    ...file,
                    hunks: [
                        {
                            ...file.hunks[0],
                            lines: [
                                {
                                    kind: "removed",
                                    text: "before a",
                                    oldLine: 4,
                                },
                                {
                                    kind: "added",
                                    text: "after a",
                                    newLine: 4,
                                    reviewLine: 23,
                                },
                                {
                                    kind: "context",
                                    text: "same",
                                    oldLine: 5,
                                    newLine: 5,
                                },
                                {
                                    kind: "removed",
                                    text: "before b",
                                    oldLine: 6,
                                },
                                {
                                    kind: "added",
                                    text: "after b",
                                    newLine: 6,
                                    reviewLine: 29,
                                },
                            ],
                        },
                    ],
                },
            ],
        };
        const result = render(() => (
            <FlowDiff review={document} onNavigateReviewLine={navigate} />
        ));
        const changes = result.container.querySelectorAll<HTMLElement>(
            ".flow-scroll.old .flow-section.change",
        );
        expect(changes).toHaveLength(2);
        Object.defineProperty(changes[1], "offsetTop", {
            configurable: true,
            value: 170,
        });
        const rightChange = result.container.querySelectorAll<HTMLElement>(
            ".flow-scroll.new .flow-section.change",
        )[1];
        Object.defineProperty(rightChange, "offsetTop", {
            configurable: true,
            value: 170,
        });
        Object.defineProperty(
            result.container.querySelector(".flow-bridge"),
            "clientHeight",
            {
                configurable: true,
                value: 300,
            },
        );
        const before = result.getByRole("region", { name: "Before changes" });
        await waitFor(() =>
            expect(
                result.container.querySelectorAll(".flow-ribbon"),
            ).toHaveLength(2),
        );
        fireEvent.click(result.container.querySelectorAll(".flow-ribbon")[1]);
        expect(before.scrollTop).toBe(162);
        expect(navigate).toHaveBeenCalledWith(29);
    });

    it("tracks the visible change while either pane scrolls", () => {
        const file = review.files[0];
        const result = render(() => (
            <FlowDiff
                review={{
                    ...review,
                    files: [
                        {
                            ...file,
                            hunks: [
                                file.hunks[0],
                                {
                                    ...file.hunks[1],
                                    lines: [
                                        {
                                            kind: "added",
                                            text: "another change",
                                            newLine: 18,
                                        },
                                    ],
                                },
                            ],
                        },
                    ],
                }}
            />
        ));
        const before = result.getByRole("region", { name: "Before changes" });
        const secondHunk = before.querySelector<HTMLElement>(
            ".flow-file-group .flow-section.change:last-child",
        )!;
        Object.defineProperties(secondHunk, {
            offsetTop: { configurable: true, value: 100 },
            offsetHeight: { configurable: true, value: 22 },
        });
        before.scrollTop = 100;
        fireEvent.scroll(before);
        expect(result.getByText("2 / 2")).toBeTruthy();
    });
    it("opens the active change with Enter and gf without swallowing modified keys", () => {
        const openSource = vi.fn();
        const refresh = vi.fn();
        const result = render(() => (
            <FlowDiff
                review={review}
                onOpenSource={openSource}
                onAction={refresh}
            />
        ));
        const surface = result.getByRole("region", { name: "Diff review" });
        fireEvent.keyDown(surface, { key: "Enter" });
        expect(openSource).toHaveBeenLastCalledWith("src/uneven.ts", 5, "new");
        fireEvent.keyDown(surface, { key: "g" });
        fireEvent.keyDown(surface, { key: "f" });
        expect(openSource).toHaveBeenCalledTimes(2);
        fireEvent.keyDown(surface, { key: "r", metaKey: true });
        expect(refresh).not.toHaveBeenCalled();
    });

    it("keeps guided descriptions available beside canonical files", () => {
        const guided = {
            ...review.files[0],
            id: "guided-parse",
            label: "Parser moved after validation",
        };
        const result = render(() => (
            <FlowDiff
                review={{
                    ...review,
                    custom: true,
                    overlay: { mode: "active" },
                    guidedFiles: [guided],
                }}
            />
        ));
        expect(
            result.container.querySelectorAll(
                ".flow-scroll.old .flow-file-group",
            ),
        ).toHaveLength(2);
        fireEvent.click(result.getByRole("button", { name: "Guided" }));
        expect(
            result.container.querySelectorAll(
                ".flow-scroll.old .flow-file-group",
            ),
        ).toHaveLength(1);
        expect(result.getByText("1 section")).toBeTruthy();
        expect(result.getByText("1 / 1")).toBeTruthy();
        expect(
            result.getAllByText(
                "Parser moved after validation · src/uneven.ts",
            ),
        ).toHaveLength(2);
    });

    it("opens saved custom slices in Guided and respects a manual Files choice", () => {
        const first = {
            ...review.files[0],
            id: "slice-first",
            label: "First slice",
            hunks: [review.files[0].hunks[0]],
        };
        const second = {
            ...review.files[0],
            id: "slice-second",
            label: "Second slice",
            hunks: [review.files[0].hunks[1]],
        };
        const saved: FlowDiffReview = {
            ...review,
            title: "Agent review",
            custom: true,
            provenance: {
                baseLabel: "HEAD",
                comparisonBaseOid: "base",
                snapshotId: "first-snapshot",
            },
            overlay: { mode: "saved" },
            guidedFiles: [first, second],
        };
        const [current, setCurrent] = createSignal(saved);
        const result = render(() => <FlowDiff review={current()} />);

        expect(
            result
                .getByRole("button", { name: "Guided" })
                .getAttribute("aria-pressed"),
        ).toBe("true");
        expect(result.getByText("2 sections")).toBeTruthy();
        expect(result.getAllByText("First slice · src/uneven.ts")).toHaveLength(
            2,
        );
        expect(
            result.getAllByText("Second slice · src/uneven.ts"),
        ).toHaveLength(2);

        fireEvent.click(result.getByRole("button", { name: "Files" }));
        setCurrent({ ...saved, layout: "unified" });
        expect(
            result
                .getByRole("button", { name: "Files" })
                .getAttribute("aria-pressed"),
        ).toBe("true");

        setCurrent({
            ...saved,
            provenance: { ...saved.provenance!, snapshotId: "second-snapshot" },
        });
        expect(
            result
                .getByRole("button", { name: "Guided" })
                .getAttribute("aria-pressed"),
        ).toBe("true");
    });

    it("switches an existing canonical viewer to Guided when a saved review opens", () => {
        const guided = {
            ...review.files[0],
            id: "split-review",
            label: "Agent slice",
        };
        const [current, setCurrent] = createSignal<FlowDiffReview>(review);
        const result = render(() => <FlowDiff review={current()} />);

        setCurrent({
            ...review,
            title: "Agent review",
            custom: true,
            overlay: { mode: "saved" },
            guidedFiles: [guided],
        });
        expect(
            result
                .getByRole("button", { name: "Guided" })
                .getAttribute("aria-pressed"),
        ).toBe("true");
        expect(result.getByText("1 section")).toBeTruthy();
    });

    it("exports canonical files after a guided review is replaced", async () => {
        const guided = {
            ...review.files[0],
            id: "guided-parse",
            label: "Parser moved after validation",
        };
        const [current, setCurrent] = createSignal<FlowDiffReview>({
            ...review,
            custom: true,
            guidedFiles: [guided],
        });
        const result = render(() => <FlowDiff review={current()} />);
        fireEvent.click(result.getByRole("button", { name: "Guided" }));
        expect(
            result.container.querySelectorAll(
                ".flow-scroll.old .flow-file-group",
            ),
        ).toHaveLength(1);

        setCurrent(review);
        expect(
            result.container.querySelectorAll(
                ".flow-scroll.old .flow-file-group",
            ),
        ).toHaveLength(2);

        const originalCreateObjectURL = Object.getOwnPropertyDescriptor(
            URL,
            "createObjectURL",
        );
        Object.defineProperty(URL, "createObjectURL", {
            configurable: true,
            value: () => "blob:review-test",
        });
        vi.stubGlobal(
            "Image",
            class {
                onerror?: () => void;
                set src(_url: string) {
                    queueMicrotask(() => this.onerror?.());
                }
            },
        );
        try {
            fireEvent.click(
                result.getByRole("button", { name: "Export image" }),
            );
            await waitFor(() =>
                expect(result.getByRole("alert").textContent).toContain(
                    "Could not render",
                ),
            );
        } finally {
            vi.unstubAllGlobals();
            if (originalCreateObjectURL)
                Object.defineProperty(
                    URL,
                    "createObjectURL",
                    originalCreateObjectURL,
                );
            else Reflect.deleteProperty(URL, "createObjectURL");
        }
    });

    it("navigates each custom fragment before crossing to the next file", async () => {
        const first = review.files[0];
        const open = vi.fn();
        const document: FlowDiffReview = {
            ...review,
            custom: true,
            files: [
                {
                    ...first,
                    hunks: [
                        {
                            ...first.hunks[0],
                            lines: [
                                { kind: "removed", text: "old a", oldLine: 4 },
                                { kind: "added", text: "new a", newLine: 4 },
                                {
                                    kind: "context",
                                    text: "between",
                                    oldLine: 5,
                                    newLine: 5,
                                },
                                { kind: "removed", text: "old b", oldLine: 6 },
                                { kind: "added", text: "new b", newLine: 6 },
                            ],
                        },
                    ],
                },
                {
                    ...first,
                    id: "src/next.ts",
                    path: "src/next.ts",
                    hunks: [
                        {
                            header: "@@ -1 +1 @@",
                            oldStart: 1,
                            oldCount: 1,
                            newStart: 1,
                            newCount: 1,
                            lines: [
                                { kind: "removed", text: "old c", oldLine: 1 },
                                { kind: "added", text: "new c", newLine: 1 },
                            ],
                        },
                    ],
                },
            ],
        };
        const result = render(() => (
            <FlowDiff review={document} onOpenSource={open} />
        ));
        const surface = result.getByRole("region", { name: "Diff review" });
        expect(result.getByText("1 / 3")).toBeTruthy();
        fireEvent.keyDown(surface, { key: "n" });
        expect(result.getByText("2 / 3")).toBeTruthy();
        fireEvent.keyDown(surface, { key: "Enter" });
        expect(open).toHaveBeenLastCalledWith("src/uneven.ts", 6, "new");
        const next = result.getByRole("button", { name: "Next change" });
        next.focus();
        fireEvent.keyDown(next, { key: "n" });
        await waitFor(() =>
            expect(
                result.container.querySelector(
                    ".flow-section.change[data-file-id='src/next.ts'][data-active='true']",
                ),
            ).toBeTruthy(),
        );
        expect(result.getByText("3 / 3")).toBeTruthy();
    });

    it("offers matching overlay controls and saved review recovery", () => {
        const cases = [
            {
                mode: "active",
                label: "Remove overlay",
                action: "toggle_overlay",
            },
            {
                mode: "available",
                label: "Apply overlay",
                action: "toggle_overlay",
            },
            {
                mode: "stale",
                label: "View saved review",
                action: "open_saved_overlay",
            },
            {
                mode: "saved",
                label: "Return to live diff",
                action: "return_to_live_diff",
            },
        ] as const;
        for (const item of cases) {
            const onAction = vi.fn();
            const result = render(() => (
                <FlowDiff
                    review={{
                        ...review,
                        overlay: { mode: item.mode, title: "Extract parser" },
                    }}
                    onAction={onAction}
                />
            ));
            fireEvent.click(result.getByRole("button", { name: item.label }));
            expect(onAction).toHaveBeenCalledWith(item.action);
            result.unmount();
        }
    });
});

describe("review checks", () => {
    for (const layout of ["split", "unified"] as const) {
        for (const guided of [false, true]) {
            it(`checks and restores ${guided ? "guided sections" : "files"} in ${layout}`, async () => {
                const [document, setDocument] = createSignal<FlowDiffReview>({
                    ...review,
                    managed: true,
                    layout,
                    custom: guided,
                    overlay: guided ? { mode: "saved" } : undefined,
                    guidedFiles: guided ? review.files : undefined,
                });
                const action = vi.fn((key: string) => {
                    if (key === "show_checked")
                        setDocument((d) => ({
                            ...d,
                            showChecked: !d.showChecked,
                        }));
                    else {
                        const prefix = `check:${guided ? "section" : "file"}:`;
                        expect(key.startsWith(prefix)).toBe(true);
                        const id = key.slice(prefix.length);
                        setDocument((d) => ({
                            ...d,
                            [guided ? "guidedFiles" : "files"]: (guided
                                ? d.guidedFiles!
                                : d.files
                            ).map((f) =>
                                f.id === id ? { ...f, checked: !f.checked } : f,
                            ),
                        }));
                    }
                });
                const result = render(() => (
                    <FlowDiff review={document()} onAction={action} />
                ));
                const checkbox = result.getAllByRole("checkbox", {
                    name: "Reviewed src/uneven.ts",
                })[0];
                fireEvent.click(checkbox);
                await waitFor(() =>
                    expect(
                        result.queryAllByRole("checkbox", {
                            name: "Reviewed src/uneven.ts",
                        }),
                    ).toHaveLength(0),
                );
                fireEvent.click(
                    result.getByRole("button", { name: "Show reviewed (1)" }),
                );
                const restored = result.getAllByRole("checkbox", {
                    name: "Reviewed src/uneven.ts",
                })[0] as HTMLInputElement;
                expect(restored.checked).toBe(true);
                fireEvent.click(restored);
                expect(
                    result.getByRole("button", { name: "Hide reviewed (0)" }),
                ).toBeTruthy();
                result.unmount();
            });
        }
    }
});

it("gd routes the selected diff symbol with its UTF-16 source column", () => {
    const definition = vi.fn();
    const result = render(() => (
        <FlowDiff review={review} onDefinition={definition} />
    ));
    const code = result.container.querySelector(
        ".flow-code-line.added .flow-line-text",
    )!;
    const node = code.firstChild!.firstChild!;
    const selection = window.getSelection()!;
    selection.collapse(node, 3);
    const panel = result.getByRole("region", { name: "Diff review" });
    fireEvent.keyDown(panel, { key: "g" });
    fireEvent.keyDown(panel, { key: "d" });
    expect(definition).toHaveBeenCalledWith("src/uneven.ts", 5, "new", 3);
    selection.removeAllRanges();
    fireEvent.keyDown(panel, { key: "g" });
    fireEvent.keyDown(panel, { key: "d" });
    expect(result.getByRole("status").textContent).toContain("Click a symbol");
    result.unmount();
});

it("adds a multiline diff comment above a selected line and retains a failed save", async () => {
    const save = vi
        .fn()
        .mockRejectedValueOnce(new Error("Source changed"))
        .mockResolvedValue(undefined);
    const ui = render(() => (
        <FlowDiff review={{ ...review, managed: true }} onComment={save} />
    ));
    const source = ui.container.querySelector(
        '[data-source-side="new"][data-source-line="5"]',
    )!;
    fireEvent.click(source);
    fireEvent.keyDown(ui.getByRole("region", { name: "Diff review" }), {
        key: "O",
    });
    const input = ui.getByRole("textbox", {
        name: "Diff comment",
    }) as HTMLTextAreaElement;
    fireEvent.input(input, { target: { value: "Why shorter?\n第二个问题" } });
    fireEvent.keyDown(input, { key: "Escape" });
    await waitFor(() =>
        expect(ui.getByRole("alert").textContent).toContain("Source changed"),
    );
    expect(input.value).toBe("Why shorter?\n第二个问题");
    fireEvent.click(ui.getByRole("button", { name: "Save comment" }));
    await waitFor(() =>
        expect(ui.queryByRole("textbox", { name: "Diff comment" })).toBeNull(),
    );
    expect(save).toHaveBeenLastCalledWith({
        path: "src/uneven.ts",
        line: 5,
        side: "new",
        before: true,
        text: "Why shorter?\n第二个问题",
        id: undefined,
    });
});

it("renders saved user comments separately from agent notes and clears explicitly", () => {
    const action = vi.fn();
    const ui = render(() => (
        <FlowDiff
            review={{
                ...review,
                managed: true,
                comments: [
                    {
                        id: 1,
                        path: "src/uneven.ts",
                        line: 5,
                        side: "new",
                        before: false,
                        text: "Check the boundary",
                        excerpt: "short version",
                    },
                ],
            }}
            onComment={vi.fn()}
            onAction={action}
        />
    ));
    expect(ui.getByText("Check the boundary")).toBeTruthy();
    fireEvent.click(ui.getByRole("button", { name: "Delete" }));
    expect(action).toHaveBeenCalledWith("remove_comment:1");
    fireEvent.click(ui.getByRole("button", { name: "Clear comments (1)" }));
    expect(action).toHaveBeenCalledWith("clear_comments");
});
