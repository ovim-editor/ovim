import { describe, expect, it } from "vitest";
import { buildDiffExportImage } from "./FlowDiffExport";
import type { FlowDiffFile, FlowDiffReview } from "./FlowDiffModel";

function file(path: string): FlowDiffFile {
    return {
        id: path,
        path,
        status: "modified",
        additions: 0,
        deletions: 0,
        binary: false,
        metadata: [],
        hunks: [],
    };
}

function review(files: FlowDiffFile[]): FlowDiffReview {
    return {
        title: "Extract parser <safely>",
        layout: "split",
        managed: true,
        custom: true,
        files,
    };
}

function textY(svg: string, text: string): number {
    const textEnd = svg.indexOf(`>${text}</text>`);
    expect(textEnd).toBeGreaterThan(-1);
    const textStart = svg.lastIndexOf("<text", textEnd);
    const y = svg.slice(textStart, textEnd).match(/\sy="(\d+)"/);
    expect(y).not.toBeNull();
    return Number(y![1]);
}

describe("diff image export", () => {
    it("renders all canonical changes and nearby moved context without clipping", () => {
        const oldFile = file("src/main.ts");
        oldFile.deletions = 2;
        oldFile.hunks = [
            {
                header: "@@ -10,2 +10,0 @@",
                oldStart: 10,
                oldCount: 2,
                newStart: 10,
                newCount: 0,
                lines: [
                    { kind: "removed", text: "DELETE <one>", oldLine: 10 },
                    { kind: "removed", text: "DELETE &two", oldLine: 11 },
                ],
            },
        ];
        const newFile = file("src/parser.ts");
        newFile.additions = 2;
        newFile.hunks = [
            {
                header: "@@ -0,0 +3,2 @@",
                oldStart: 0,
                oldCount: 0,
                newStart: 3,
                newCount: 2,
                lines: [
                    { kind: "added", text: "ADD <one>", newLine: 3 },
                    { kind: "added", text: "ADD &two", newLine: 4 },
                ],
            },
        ];
        const document = review([oldFile, newFile]);
        document.provenance = {
            baseLabel: "main",
            comparisonBaseOid: "abc123",
            snapshotId: "snapshot-7",
        };
        document.moves = [
            {
                id: "parser-move",
                label: "Parser moved and simplified across files",
                old: {
                    path: oldFile.path,
                    startLine: 10,
                    lineCount: 2,
                    contextComplete: true,
                    contextWindows: [
                        {
                            startLine: 1,
                            lines: Array.from({ length: 20 }, (_, index) => ({
                                kind: "context" as const,
                                text: `SOURCE ${index + 1}`,
                                oldLine: index + 1,
                            })),
                        },
                    ],
                },
                new: {
                    path: newFile.path,
                    startLine: 3,
                    lineCount: 2,
                    contextComplete: true,
                    contextWindows: [
                        {
                            startLine: 1,
                            lines: Array.from({ length: 8 }, (_, index) => ({
                                kind: "context" as const,
                                text: `DEST ${index + 1}`,
                                newLine: index + 1,
                            })),
                        },
                    ],
                },
            },
        ];
        const plainSvg = buildDiffExportImage(document, {
            view: "files",
            reconstruction: "old",
        }).svg;
        expect(plainSvg).toContain("FILE CHANGES");
        expect(plainSvg).not.toContain("SOURCE 10");

        const image = buildDiffExportImage(document, {
            view: "files",
            reconstruction: "old",
            traceMoves: true,
        });

        expect(image.height).toBeLessThan(2000);
        expect(image.filename).toMatch(/extract-parser-safely-files\.png/);
        const svg = image.svg;
        expect(svg).toContain("DELETE &lt;one&gt;");
        expect(svg).toContain("DELETE &amp;two");
        expect(svg).toContain("ADD &lt;one&gt;");
        expect(svg).toContain("ADD &amp;two");
        expect(svg.match(/ADD &lt;one&gt;/g)).toHaveLength(1);
        expect(svg).toContain("Parser moved and simplified across files");
        expect(svg).toContain("SOURCE 7");
        expect(svg).toContain("SOURCE 14");
        expect(svg).not.toContain("SOURCE 6");
        expect(svg).not.toContain("SOURCE 15");
        expect(textY(svg, "SOURCE 10")).toBe(textY(svg, "ADD &lt;one&gt;"));
        expect(svg).toContain("Snapshot base: main");
        expect(svg).not.toContain("Page 1 of");
        expect(svg).not.toContain("foreignObject");

        const newSvg = buildDiffExportImage(document, {
            view: "files",
            reconstruction: "new",
            traceMoves: true,
        }).svg;
        expect(textY(newSvg, "DEST 3")).toBe(
            textY(newSvg, "DELETE &lt;one&gt;"),
        );
    });

    it("refuses guided images with missing and duplicate source lines", () => {
        const canonical = file("src/mod.ts");
        canonical.additions = 2;
        canonical.hunks = [
            {
                header: "@@ -0,0 +1,2 @@",
                oldStart: 0,
                oldCount: 0,
                newStart: 1,
                newCount: 2,
                lines: [
                    { kind: "added", text: "first", newLine: 1 },
                    { kind: "added", text: "second", newLine: 2 },
                ],
            },
        ];
        const guided = file(canonical.path);
        guided.id = "guided-1";
        guided.additions = 2;
        guided.hunks = [
            {
                ...canonical.hunks[0],
                lines: [
                    canonical.hunks[0].lines[0],
                    canonical.hunks[0].lines[0],
                ],
            },
        ];
        const document = review([canonical]);
        document.guidedFiles = [guided];
        expect(() =>
            buildDiffExportImage(document, {
                view: "guided",
                reconstruction: "old",
            }),
        ).toThrow(/do not cover every canonical changed line/);

        guided.label = "Parser moved after validation";
        guided.hunks[0].lines = canonical.hunks[0].lines;
        const image = buildDiffExportImage(document, {
            view: "guided",
            reconstruction: "old",
        });
        expect(image.svg).toContain("Parser moved after validation");
        expect(image.svg).toContain("GUIDED SECTIONS");
    });

    it("keeps long wrapped code in one dark image and includes metadata and binary files", () => {
        const source = file("src/long.ts");
        source.additions = 180;
        source.hunks = [
            {
                header: "@@ -0,0 +1,180 @@",
                oldStart: 0,
                oldCount: 0,
                newStart: 1,
                newCount: 180,
                lines: Array.from({ length: 180 }, (_, index) => ({
                    kind: "added" as const,
                    text: `LINE-${String(index + 1).padStart(3, "0")} ${"x".repeat(190)}`,
                    newLine: index + 1,
                })),
            },
        ];
        const binary = file("assets/icon.png");
        binary.binary = true;
        binary.metadata = ["new mode 100644"];
        const image = buildDiffExportImage(review([source, binary]), {
            view: "files",
            reconstruction: "new",
        });

        expect(image.height).toBeGreaterThan(18000);
        expect(image.svg).toContain('fill="#111827"');
        const text = image.svg;
        expect(text).toContain("LINE-001");
        expect(text).toContain("LINE-180");
        expect(text).toContain("new mode 100644");
        expect(text).toContain("Binary file; no text diff.");
        expect(image.svg).not.toContain("continued");
    });

    it("exports reviews beyond the former page and text limits", () => {
        const source = file("src/huge.ts");
        source.additions = 5100;
        source.hunks = [
            {
                header: "@@ -0,0 +1,5100 @@",
                oldStart: 0,
                oldCount: 0,
                newStart: 1,
                newCount: 5100,
                lines: Array.from({ length: 5100 }, (_, index) => ({
                    kind: "added" as const,
                    text: `LINE-${index + 1}`,
                    newLine: index + 1,
                })),
            },
        ];
        const image = buildDiffExportImage(review([source]), {
            view: "files",
            reconstruction: "old",
        });

        expect(image.height).toBeGreaterThan(127500);
        expect(image.svg).toContain("LINE-5100");
    });

    it("wraps Unicode code points without losing characters", () => {
        const source = file("src/unicode.ts");
        source.additions = 1;
        source.hunks = [
            {
                header: "@@ -0,0 +1 @@",
                oldStart: 0,
                oldCount: 0,
                newStart: 1,
                newCount: 1,
                lines: [{ kind: "added", text: "🙂".repeat(130), newLine: 1 }],
            },
        ];
        const svg = buildDiffExportImage(review([source]), {
            view: "files",
            reconstruction: "new",
        }).svg;
        expect(svg.match(/🙂/g)).toHaveLength(130);
        expect(svg).toContain('textLength="650"');
    });
});

it("exports the selected unified guided order with full-width code", () => {
    const source = file("new.ts");
    source.oldPath = "old.ts";
    source.hunks = [
        {
            header: "Pair",
            oldStart: 1,
            oldCount: 2,
            newStart: 10,
            newCount: 2,
            lines: [
                { kind: "removed", text: "old first", oldLine: 1 },
                { kind: "removed", text: "old second", oldLine: 2 },
                { kind: "added", text: "new first", newLine: 10 },
                { kind: "added", text: "new second", newLine: 11 },
            ],
        },
    ];
    const document = {
        ...review([source]),
        guidedFiles: [{ ...source, label: "Curated pairing" }],
    };
    const svg = buildDiffExportImage(document, {
        view: "guided",
        layout: "unified",
        reconstruction: "old",
    }).svg;
    expect(svg).toContain("GUIDED SECTIONS · UNIFIED");
    expect(svg).toContain("Curated pairing");
    expect(textY(svg, "old second")).toBeLessThan(textY(svg, "new first"));
    expect(svg).toContain('width="1492" height="25"');
    expect(svg).not.toContain(">AFTER</text>");
});

it("exports only visible reviewed items and labels hidden accounting", () => {
    const checked = { ...file("checked.rs"), checked: true, binary: true };
    const document = review([checked]);
    const options = {
        view: "files" as const,
        layout: "unified" as const,
        reconstruction: "old" as const,
        traceMoves: false,
        hideEqual: false,
    };
    const hidden = buildDiffExportImage(document, options).svg;
    expect(hidden).not.toContain("checked.rs");
    expect(hidden).toContain("1 reviewed item hidden");
    expect(hidden).toContain("All visible changes reviewed");
    expect(
        buildDiffExportImage({ ...document, showChecked: true }, options).svg,
    ).toContain("checked.rs");
});

it("pluralizes the coverage footer and omits an empty moved-code count", () => {
    const options = {
        view: "files" as const,
        layout: "unified" as const,
        reconstruction: "old" as const,
    };
    const one = buildDiffExportImage(review([file("a.rs")]), options).svg;
    expect(one).toContain("1 file · +0 −0");
    expect(one).not.toContain("moved-code");
    const many = buildDiffExportImage(
        review([file("a.rs"), file("b.rs")]),
        options,
    ).svg;
    expect(many).toContain("2 files · +0 −0");
});
