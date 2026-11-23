import { fireEvent, render } from "@solidjs/testing-library";
import { describe, expect, it } from "vitest";
import FlowDiff from "./FlowDiff";
import { buildDiffExportImage } from "./FlowDiffExport";
import type { GuiDiffDocument } from "./types";

const file: GuiDiffDocument["files"][number] = {
    id: "source.ts",
    path: "source.ts",
    status: "modified",
    additions: 1,
    deletions: 1,
    binary: false,
    metadata: [],
    hunks: [
        {
            header: "@@ -1 +1 @@",
            oldStart: 1,
            oldCount: 1,
            newStart: 1,
            newCount: 1,
            lines: [
                { kind: "removed", text: "before", oldLine: 1, reviewLine: 1 },
                { kind: "added", text: "after", newLine: 1, reviewLine: 2 },
            ],
        },
    ],
};

const review: GuiDiffDocument = {
    title: "Review with explanation",
    layout: "split",
    managed: true,
    custom: true,
    files: [file],
    guidedFiles: [
        {
            ...file,
            id: "pair_0",
            label: "Rename value",
            status: "reassigned",
            message: "The new name explains the result.\nCheck both callers.",
        },
    ],
};

describe("agent notes in a guided diff", () => {
    it("toggles prose without changing section navigation or canonical Files", () => {
        const view = render(() => <FlowDiff review={review} />);
        expect(view.queryByRole("button", { name: "Notes" })).toBeNull();
        fireEvent.click(view.getByRole("button", { name: "Guided" }));
        const toggle = view.getByRole("button", { name: "Notes" });
        expect(toggle.getAttribute("aria-pressed")).toBe("true");
        expect(view.getAllByLabelText("Agent note")).toHaveLength(2);
        const count = view.getByText("1 / 1");
        fireEvent.keyDown(view.container.querySelector(".flow-diff")!, {
            key: "a",
        });
        expect(toggle.getAttribute("aria-pressed")).toBe("false");
        expect(view.queryByLabelText("Agent note")).toBeNull();
        expect(count.textContent).toBe("1 / 1");
        fireEvent.click(toggle);
        expect(view.getAllByLabelText("Agent note")).toHaveLength(2);
        fireEvent.click(view.getByRole("button", { name: "Files" }));
        expect(view.queryByRole("button", { name: "Notes" })).toBeNull();
        expect(view.queryByLabelText("Agent note")).toBeNull();
    });

    it("exports notes only when the selected Guided view shows them", () => {
        const options = {
            view: "guided" as const,
            reconstruction: "old" as const,
        };
        expect(buildDiffExportImage(review, options).svg).toContain(
            "Agent note",
        );
        expect(
            buildDiffExportImage(review, { ...options, showNotes: false }).svg,
        ).not.toContain("Agent note");
        expect(
            buildDiffExportImage(review, { ...options, view: "files" }).svg,
        ).not.toContain("Agent note");
    });
});
