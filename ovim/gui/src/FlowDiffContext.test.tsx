// @vitest-environment jsdom
import { fireEvent, render } from "@solidjs/testing-library";
import { createSignal } from "solid-js";
import { describe, expect, it, vi } from "vitest";
import FlowDiff from "./FlowDiff";
import { sectionsForFile, unifiedSections } from "./FlowDiffModel";
import type { GuiDiffDocument } from "./types";

const document = (): GuiDiffDocument => ({
    title: "Captured context",
    managed: true,
    custom: true,
    layout: "split",
    files: [
        {
            id: "paired",
            path: "new.ts",
            oldPath: "old.ts",
            status: "reassigned",
            additions: 1,
            deletions: 1,
            binary: false,
            metadata: [],
            hunks: [
                {
                    header: "Pair",
                    oldStart: 20,
                    oldCount: 1,
                    newStart: 40,
                    newCount: 1,
                    lines: [
                        { kind: "removed", oldLine: 20, text: "old_change();" },
                        { kind: "added", newLine: 40, text: "new_change();" },
                    ],
                    context: {
                        id: "section:pair",
                        before: { old: [], new: [] },
                        after: { old: [], new: [] },
                        canExpandUp: true,
                        canExpandDown: true,
                    },
                },
            ],
        },
    ],
});
function expanded(): GuiDiffDocument {
    const review = document();
    review.files[0].hunks[0].context!.before = {
        old: [{ number: 19, text: "old_before();" }],
        new: [{ number: 39, text: "new_before();" }],
    };
    review.files[0].hunks[0].context!.after = {
        old: [{ number: 21, text: "old_after();" }],
        new: [{ number: 41, text: "new_after();" }],
    };
    return review;
}
describe("captured diff context", () => {
    it("keeps stable change identities and both independent source streams", () => {
        const before = sectionsForFile(document().files[0]);
        const after = sectionsForFile(expanded().files[0]);
        expect(after.find((s) => s.kind === "change")!.id).toBe(before[0].id);
        expect(after.flatMap((s) => s.left).map((l) => l.text)).toEqual([
            "old_before();",
            "old_change();",
            "old_after();",
        ]);
        expect(after.flatMap((s) => s.right).map((l) => l.text)).toEqual([
            "new_before();",
            "new_change();",
            "new_after();",
        ]);
        const unified = unifiedSections(after);
        expect(unified).toHaveLength(1);
        expect(unified[0].left.map((l) => l.oldLine)).toEqual([19, 20, 21]);
        expect(unified[0].right.map((l) => l.newLine)).toEqual([39, 40, 41]);
    });
    it("dispatches gutter and keyboard expansion without adding text rows, then opens revealed source", async () => {
        const expand = vi.fn();
        const open = vi.fn();
        const [review, setReview] = createSignal(document());
        const view = render(() => (
            <FlowDiff
                review={review()}
                onExpandContext={expand}
                onOpenSource={open}
            />
        ));
        const root = view.container.querySelector(".flow-diff")!;
        const rows = view.container.querySelectorAll(".flow-code-line").length;
        await fireEvent.click(
            view.getAllByRole("button", { name: "Show more context above" })[0],
        );
        expect(expand).toHaveBeenLastCalledWith("section:pair", true, false);
        await fireEvent.keyDown(root, { key: "J" });
        expect(expand).toHaveBeenLastCalledWith("section:pair", false, false);
        expect(view.container.querySelectorAll(".flow-code-line")).toHaveLength(
            rows,
        );
        setReview(expanded());
        expect(view.getByText("old_before();")).toBeTruthy();
        await fireEvent.click(
            view.getByRole("button", { name: "Before line 19, open source" }),
        );
        expect(open).toHaveBeenCalledWith("old.ts", 19, "old");
        await fireEvent.keyDown(root, { key: "s" });
        const text = view.container.querySelector(
            ".flow-unified-scroll",
        )!.textContent!;
        expect(text.indexOf("old_before();")).toBeLessThan(
            text.indexOf("old_change();"),
        );
        expect(text.indexOf("old_after();")).toBeLessThan(
            text.indexOf("new_before();"),
        );
        expect(text.indexOf("new_change();")).toBeLessThan(
            text.indexOf("new_after();"),
        );
    });
    it("omits controls when context is unavailable", () => {
        const review = document();
        review.files[0].hunks[0].context = undefined;
        const view = render(() => (
            <FlowDiff review={review} onExpandContext={vi.fn()} />
        ));
        expect(
            view.queryByRole("button", { name: "Show more context above" }),
        ).toBeNull();
    });
    describe("unchanged-lines gap", () => {
        const gapped = (secondStart: number, canExpandUp = true) => {
            const review = document();
            review.custom = false;
            review.files[0].status = "modified";
            review.files[0].oldPath = undefined;
            const [first] = review.files[0].hunks;
            first.oldStart = 1;
            first.newStart = 1;
            first.lines = [
                { kind: "removed", oldLine: 1, text: "first_old();" },
                { kind: "added", newLine: 1, text: "first_new();" },
            ];
            review.files[0].hunks.push({
                ...structuredClone(first),
                header: "Second",
                oldStart: secondStart,
                newStart: secondStart,
                lines: [
                    { kind: "removed", oldLine: secondStart, text: "b_old();" },
                    { kind: "added", newLine: secondStart, text: "b_new();" },
                ],
                context: {
                    id: "hunk:second",
                    before: { old: [], new: [] },
                    after: { old: [], new: [] },
                    canExpandUp,
                    canExpandDown: false,
                },
            });
            return review;
        };
        it("opens a small gap in one click", async () => {
            const expand = vi.fn();
            const view = render(() => (
                <FlowDiff review={gapped(21)} onExpandContext={expand} />
            ));
            const buttons = view.getAllByRole("button", {
                name: "Expand 19 unchanged lines",
            });
            expect(buttons[0].textContent).toBe("19 unchanged lines");
            expect(
                view.queryByRole("button", { name: /Expand all/ }),
            ).toBeNull();
            await fireEvent.click(buttons[0]);
            expect(expand).toHaveBeenLastCalledWith("hunk:second", true, true);
        });
        it("offers expand all for a large gap and keeps its label as text", async () => {
            const expand = vi.fn();
            const view = render(() => (
                <FlowDiff review={gapped(300)} onExpandContext={expand} />
            ));
            expect(
                view.queryByRole("button", { name: /^Expand \d+ unchanged/ }),
            ).toBeNull();
            const buttons = view.getAllByRole("button", {
                name: "Expand all 298 unchanged lines",
            });
            await fireEvent.click(buttons[0]);
            expect(expand).toHaveBeenLastCalledWith("hunk:second", true, true);
        });
        it("is plain text when the source is not captured", () => {
            const view = render(() => (
                <FlowDiff
                    review={gapped(21, false)}
                    onExpandContext={vi.fn()}
                />
            ));
            expect(
                view.queryByRole("button", { name: /unchanged lines/ }),
            ).toBeNull();
            expect(
                view.getAllByText(/19 unchanged lines/).length,
            ).toBeGreaterThan(0);
        });
    });
});
