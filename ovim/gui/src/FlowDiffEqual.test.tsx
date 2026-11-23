// @vitest-environment jsdom
import { fireEvent, render } from "@solidjs/testing-library";
import { describe, expect, it, vi } from "vitest";
import FlowDiff from "./FlowDiff";
import { buildDiffExportImage } from "./FlowDiffExport";
import {
    hideEqualChanges,
    isEqualOnly,
    sectionsForFile,
    type FlowDiffFile,
    type FlowDiffReview,
} from "./FlowDiffModel";

function paired(
    before: string[],
    after: string[],
    path = "src/parser.ts",
    oldPath = path,
): FlowDiffFile {
    return {
        id: path,
        path,
        oldPath,
        status: "reassigned",
        additions: after.length,
        deletions: before.length,
        binary: false,
        metadata: [],
        hunks: [
            {
                header: "Paired by agent",
                oldStart: 10,
                oldCount: before.length,
                newStart: 40,
                newCount: after.length,
                lines: [
                    ...before.map((text, i) => ({
                        kind: "removed" as const,
                        text,
                        oldLine: 10 + i,
                        reviewLine: 100 + i,
                    })),
                    ...after.map((text, i) => ({
                        kind: "added" as const,
                        text,
                        newLine: 40 + i,
                        reviewLine: 200 + i,
                    })),
                ],
            },
        ],
    };
}
const equal = paired(
    ["same();", "  spaced( value );"],
    ["same();", "\tspaced(value); "],
);
const cross = paired(["moved();"], ["moved();"], "src/new.ts", "src/old.ts");
const review: FlowDiffReview = {
    title: "Curated comparison",
    custom: true,
    managed: true,
    layout: "split",
    files: [equal, cross],
    guidedFiles: [equal, cross],
};

describe("hiding equal curated changes", () => {
    it("hides same-file equal and whitespace-only pairs including cross-file pairs", () => {
        const sections = hideEqualChanges(equal, sectionsForFile(equal));
        expect(isEqualOnly(equal, sections)).toBe(true);
        expect(sections.every((section) => section.kind === "gap")).toBe(true);
        expect(
            isEqualOnly(cross, hideEqualChanges(cross, sectionsForFile(cross))),
        ).toBe(true);
        expect(equal.hunks[0].lines).toHaveLength(4);
    });

    it("preserves real edits and source locations inside a mixed pair with unequal lengths", () => {
        const mixed = paired(
            ["same();", "old();", "end();"],
            ["same( );", "new();", "extra();", " end();"],
        );
        const changes = hideEqualChanges(mixed, sectionsForFile(mixed)).filter(
            (section) => section.kind === "change",
        );
        expect(changes).toHaveLength(1);
        expect(changes[0].left.map((line) => line.text)).toEqual(["old();"]);
        expect(changes[0].right.map((line) => line.text)).toEqual([
            "new();",
            "extra();",
        ]);
        expect(changes[0].right[0]).toMatchObject({
            newLine: 41,
            reviewLine: 201,
        });
    });

    it("keeps insertions, deletions, metadata, binary changes and reordered lines", () => {
        for (const file of [paired([], ["  "]), paired(["delete();"], [])]) {
            expect(hideEqualChanges(file, sectionsForFile(file))).toEqual(
                sectionsForFile(file),
            );
        }
        for (const file of [
            { ...equal, binary: true },
            { ...equal, metadata: ["old mode 100644", "new mode 100755"] },
        ]) {
            expect(
                isEqualOnly(
                    file,
                    hideEqualChanges(file, sectionsForFile(file)),
                ),
            ).toBe(false);
        }
        const reordered = paired(["a();", "b();"], ["b();", "a();"]);
        expect(
            hideEqualChanges(reordered, sectionsForFile(reordered)).some(
                (section) => section.kind === "change",
            ),
        ).toBe(true);
    });

    it("bounds expensive comparisons without hiding unverified changes", () => {
        const file = paired(
            Array.from({ length: 501 }, (_, i) => `old${i}`),
            Array.from({ length: 501 }, (_, i) => `new${i}`),
        );
        expect(hideEqualChanges(file, sectionsForFile(file))).toEqual(
            sectionsForFile(file),
        );
    });

    it("toggles with w and the button across layouts and guided view, retaining navigation", () => {
        const onOpenSource = vi.fn();
        const result = render(() => (
            <FlowDiff review={review} onOpenSource={onOpenSource} />
        ));
        const panel = result.getByRole("region", { name: "Diff review" });
        const toggle = result.getByRole("button", {
            name: "Hide equal changes",
        });
        fireEvent.keyDown(panel, { key: "w" });
        expect(toggle.getAttribute("aria-pressed")).toBe("true");
        expect(result.queryAllByText("same();")).toHaveLength(0);
        expect(result.getByText("0 / 0")).toBeTruthy();
        fireEvent.click(result.getByRole("button", { name: "Guided" }));
        expect(result.queryAllByText("same();")).toHaveLength(0);
        fireEvent.keyDown(panel, { key: "s" });
        expect(result.getByText("No unequal changes")).toBeTruthy();
        fireEvent.keyDown(panel, { key: "Enter" });
        expect(onOpenSource).not.toHaveBeenCalled();
        fireEvent.click(toggle);
        expect(result.getAllByText("same();")).toHaveLength(2);
        expect(result.getByText("1 / 2")).toBeTruthy();
    });

    it("shows an explicit empty state and can restore an entirely equal review", () => {
        const result = render(() => (
            <FlowDiff
                review={{ ...review, files: [equal], guidedFiles: [equal] }}
            />
        ));
        const toggle = result.getByRole("button", {
            name: "Hide equal changes",
        });
        fireEvent.click(toggle);
        expect(result.getByText("No unequal changes")).toBeTruthy();
        expect(
            result
                .getByRole("button", { name: "Next change" })
                .hasAttribute("disabled"),
        ).toBe(true);
        fireEvent.click(toggle);
        expect(result.getByText("1 / 1")).toBeTruthy();
    });

    it("exports the selected filter in Files and Guided without changing coverage accounting", () => {
        for (const view of ["files", "guided"] as const) {
            const filtered = buildDiffExportImage(review, {
                view,
                reconstruction: "old",
                hideEqual: true,
            }).svg;
            expect(filtered).not.toContain("same();");
            expect(filtered).not.toContain("moved();");
            expect(filtered).toContain("Equal paired changes hidden");
            expect(filtered).not.toContain("src/parser.ts");
            const complete = buildDiffExportImage(review, {
                view,
                reconstruction: "old",
            }).svg;
            expect(complete).toContain("same();");
        }
    });
});

it("follows same-file agent pairings across separate hunks in Files view", () => {
    const file = paired(["same();", "old();"], [" same( );", "new();"]);
    const [removed, added] = [
        file.hunks[0].lines.slice(0, 2),
        file.hunks[0].lines.slice(2),
    ];
    file.hunks = [
        { ...file.hunks[0], newCount: 0, lines: removed },
        { ...file.hunks[0], oldStart: 40, oldCount: 0, lines: added },
    ];
    const move = {
        id: "same-file-pair",
        old: {
            path: file.path,
            startLine: 10,
            lineCount: 2,
            contextWindows: [{ startLine: 10, lines: removed }],
            contextComplete: false,
        },
        new: {
            path: file.path,
            startLine: 40,
            lineCount: 2,
            contextWindows: [{ startLine: 40, lines: added }],
            contextComplete: false,
        },
    };
    const filtered = hideEqualChanges(file, sectionsForFile(file), [move]);
    expect(
        filtered.flatMap((section) => section.left).map((line) => line.text),
    ).toEqual(["old();"]);
    expect(
        filtered.flatMap((section) => section.right).map((line) => line.text),
    ).toEqual(["new();"]);
    for (const traceMoves of [false, true]) {
        const exported = buildDiffExportImage(
            { ...review, files: [file], moves: [move] },
            {
                view: "files",
                reconstruction: "old",
                hideEqual: true,
                traceMoves,
            },
        ).svg;
        expect(exported).not.toContain("same();");
        expect(exported).toContain("old();");
        expect(exported).toContain("new();");
    }
});

it("exports an explicit empty state when all curated pairs are equal", () => {
    const image = buildDiffExportImage(
        { ...review, files: [equal], guidedFiles: [equal] },
        {
            view: "guided",
            reconstruction: "old",
            hideEqual: true,
        },
    );
    expect(image.svg).toContain("No unequal changes");
    expect(image.svg).not.toContain("same();");
});

it("hides cross-file pair endpoints in Files view without hiding unrelated coordinates", () => {
    const guided = paired(
        ["same();", "old();"],
        ["same( );", "new();"],
        "new.ts",
        "old.ts",
    );
    const oldLines = guided.hunks[0].lines.filter((l) => l.kind === "removed");
    const newLines = guided.hunks[0].lines.filter((l) => l.kind === "added");
    const before = {
        ...guided,
        id: "old",
        path: "old.ts",
        oldPath: undefined,
        hunks: [{ ...guided.hunks[0], lines: oldLines }],
    };
    const after = {
        ...guided,
        id: "new",
        oldPath: undefined,
        hunks: [{ ...guided.hunks[0], lines: newLines }],
    };
    const move = {
        id: "cross",
        old: {
            path: "old.ts",
            startLine: 10,
            lineCount: 2,
            contextComplete: true,
            contextWindows: [{ startLine: 10, lines: oldLines }],
        },
        new: {
            path: "new.ts",
            startLine: 40,
            lineCount: 2,
            contextComplete: true,
            contextWindows: [{ startLine: 40, lines: newLines }],
        },
    };
    for (const file of [before, after, guided]) {
        const sections = hideEqualChanges(file, sectionsForFile(file), [move]);
        const text = sections
            .flatMap((s) => [...s.left, ...s.right])
            .map((l) => l.text);
        expect(text.join()).not.toContain("same");
        expect(text).toContain(file === before ? "old();" : "new();");
    }
    const unrelated = { ...before, path: "unrelated.ts" };
    expect(
        hideEqualChanges(unrelated, sectionsForFile(unrelated), [move]),
    ).toEqual(sectionsForFile(unrelated));
    const incomplete = { ...move, new: { ...move.new, contextWindows: [] } };
    expect(
        hideEqualChanges(before, sectionsForFile(before), [incomplete]),
    ).toEqual(sectionsForFile(before));
});
