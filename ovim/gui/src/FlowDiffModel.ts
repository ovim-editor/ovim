import { diffArrays } from "diff";
import type { GuiDiffDocument, GuiDiffHunk, GuiDiffLine } from "./types";

export type FlowDiffLine = GuiDiffLine;
export type FlowDiffHunk = GuiDiffHunk;
export type FlowDiffReview = GuiDiffDocument;
export type FlowDiffFile = GuiDiffDocument["files"][number];
export type FlowDiffMove = NonNullable<GuiDiffDocument["moves"]>[number];
export type Reconstruction = "old" | "new";

export function moveLocation(endpoint: FlowDiffMove["old"]): string {
    const end = endpoint.startLine + endpoint.lineCount - 1;
    return `${endpoint.path}:${endpoint.startLine}${end > endpoint.startLine ? `–${end}` : ""}`;
}

/** A gap this small opens with a single click on its label. */
export const GAP_EXPAND_ALL_MAX = 40;

export type FlowSection = {
    id: string;
    kind: "context" | "change" | "gap";
    hunkIndex: number;
    label?: string;
    /** Hidden source lines of an expandable gap between two hunks. */
    gapLines?: number;
    left: FlowDiffLine[];
    right: FlowDiffLine[];
    move?: FlowDiffMove;
    equal?: boolean;
    expanded?: boolean;
};

/** Keep move cards anchored to their exact canonical change lines. */
export function sectionsWithMoves(
    file: FlowDiffFile,
    moves: FlowDiffMove[],
    reconstruction: Reconstruction,
): FlowSection[] {
    const anchorSide = reconstruction === "old" ? "right" : "left";
    const endpointSide = reconstruction === "old" ? "new" : "old";
    const relevant = moves.filter(
        (move) =>
            move[endpointSide].path ===
            (endpointSide === "old" ? file.oldPath || file.path : file.path),
    );
    if (!relevant.length) return sectionsForFile(file);

    return sectionsForFile(file).flatMap((section) => {
        if (section.kind !== "change") return [section];
        const count = Math.max(section.left.length, section.right.length);
        const moveAt = (index: number) => {
            const number =
                section[anchorSide][index]?.[
                    reconstruction === "old" ? "newLine" : "oldLine"
                ];
            return relevant.find(
                (move) =>
                    number !== undefined &&
                    number >= move[endpointSide].startLine &&
                    number <
                        move[endpointSide].startLine +
                            move[endpointSide].lineCount,
            );
        };
        const parts: FlowSection[] = [];
        for (let start = 0; start < count;) {
            const move = moveAt(start);
            let end = start + 1;
            while (end < count && moveAt(end)?.id === move?.id) end++;
            parts.push({
                ...section,
                id: `${section.id}-p${parts.length}`,
                left: section.left.slice(start, end),
                right: section.right.slice(start, end),
                move,
            });
            start = end;
        }
        return parts;
    });
}

/** These statuses describe owned review slices, not whole-file Git statuses. */
export function isCuratedSection(file: FlowDiffFile): boolean {
    return (
        file.status === "reassigned" ||
        file.status === "deletion" ||
        file.status === "addition"
    );
}

/** A section is the smallest unit whose two edges need to stay related. */
export function sectionsForFile(file: FlowDiffFile): FlowSection[] {
    const sections: FlowSection[] = [];
    let previousOldEnd = 1;
    let previousNewEnd = 1;

    file.hunks.forEach((hunk, hunkIndex) => {
        const skippedOld = Math.max(
            0,
            (hunk.context?.before.old[0]?.number ?? hunk.oldStart) -
                previousOldEnd,
        );
        const skippedNew = Math.max(
            0,
            (hunk.context?.before.new[0]?.number ?? hunk.newStart) -
                previousNewEnd,
        );
        if ((skippedOld || skippedNew) && !isCuratedSection(file)) {
            sections.push({
                id: `gap-${hunkIndex}`,
                kind: "gap",
                hunkIndex,
                gapLines: Math.max(skippedOld, skippedNew),
                label: `${Math.max(skippedOld, skippedNew)} unchanged ${Math.max(skippedOld, skippedNew) === 1 ? "line" : "lines"}`,
                left: [],
                right: [],
            });
        }

        const contextSection = (edge: "before" | "after") => {
            const context = hunk.context?.[edge];
            if (!context || (!context.old.length && !context.new.length))
                return;
            const line = (
                source: { number: number; text: string },
                side: "old" | "new",
            ): GuiDiffLine => ({
                kind: "context",
                text: source.text,
                [side === "old" ? "oldLine" : "newLine"]: source.number,
            });
            sections.push({
                id: `h${hunkIndex}-${edge}`,
                kind: "context",
                hunkIndex,
                expanded: true,
                left: context.old.map((source) => line(source, "old")),
                right: context.new.map((source) => line(source, "new")),
            });
        };
        contextSection("before");
        let runIndex = 0;
        let run: FlowSection | undefined;
        for (const line of hunk.lines) {
            const kind = line.kind === "context" ? "context" : "change";
            if (!run || run.kind !== kind) {
                run = {
                    id: `h${hunkIndex}-s${runIndex++}`,
                    kind,
                    hunkIndex,
                    left: [],
                    right: [],
                };
                sections.push(run);
            }
            if (line.kind !== "added") run.left.push(line);
            if (line.kind !== "removed") run.right.push(line);
        }
        contextSection("after");
        previousOldEnd =
            (hunk.context?.after.old.at(-1)?.number ??
                hunk.oldStart + hunk.oldCount - 1) + 1;
        previousNewEnd =
            (hunk.context?.after.new.at(-1)?.number ??
                hunk.newStart + hunk.newCount - 1) + 1;
    });

    return sections;
}

function compareLines(before: FlowDiffLine[], after: FlowDiffLine[]) {
    const normalized = (line: FlowDiffLine) =>
        line.text.replace(/[ \t\r\f\v]/g, "");
    return diffArrays(before.map(normalized), after.map(normalized), {
        maxEditLength: 1000,
    });
}

/** Compare a complete agent pairing using its frozen endpoint text. */
export function equalMoveLines(move: FlowDiffMove, file?: FlowDiffFile) {
    const oldLines = new Set<number>();
    const newLines = new Set<number>();
    const endpointLines = (side: "old" | "new") => {
        const endpoint = move[side];
        const coordinate = side === "old" ? "oldLine" : "newLine";
        const canonical =
            file &&
            endpoint.path ===
                (side === "old" ? file.oldPath || file.path : file.path)
                ? file.hunks
                      .flatMap((h) => h.lines)
                      .filter(
                          (l) =>
                              l.kind === (side === "old" ? "removed" : "added"),
                      )
                : [];
        const byNumber = new Map(
            [...endpoint.contextWindows.flatMap((w) => w.lines), ...canonical]
                .filter((l) => l[coordinate] !== undefined)
                .map((l) => [l[coordinate]!, l]),
        );
        return Array.from({ length: endpoint.lineCount }, (_, i) =>
            byNumber.get(endpoint.startLine + i),
        );
    };
    const before = endpointLines("old"),
        after = endpointLines("new");
    if (before.some((l) => !l) || after.some((l) => !l))
        return { oldLines, newLines };
    let oldIndex = 0,
        newIndex = 0;
    for (const part of compareLines(
        before as FlowDiffLine[],
        after as FlowDiffLine[],
    ) ?? []) {
        if (!part.added && !part.removed) {
            for (let i = 0; i < part.count; i++) {
                oldLines.add(before[oldIndex + i]!.oldLine!);
                newLines.add(after[newIndex + i]!.newLine!);
            }
        }
        if (!part.added) oldIndex += part.count;
        if (!part.removed) newIndex += part.count;
    }
    return { oldLines, newLines };
}

/** Apply paired omissions only to their owning file and side. */
export function equalPairedLines(file: FlowDiffFile, moves: FlowDiffMove[]) {
    const oldLines = new Set<number>(),
        newLines = new Set<number>();
    for (const move of moves) {
        const old = move.old.path === (file.oldPath || file.path);
        const next = move.new.path === file.path;
        if (!old && !next) continue;
        const equal = equalMoveLines(move, file);
        if (old) for (const line of equal.oldLines) oldLines.add(line);
        if (next) for (const line of equal.newLines) newLines.add(line);
    }
    return { oldLines, newLines };
}

/** Collapse equal lines after ignoring horizontal whitespace, preserving source
 * coordinates and the original patch. Re-diff each replacement so cosmetic
 * edits do not obscure real changes inside the same block. Unpaired insertions and
 * deletions remain changes. Explicit pairings may cross file boundaries.
 */
export function hideEqualChanges(
    file: FlowDiffFile,
    sections: FlowSection[],
    moves: FlowDiffMove[] = [],
): FlowSection[] {
    const paired = equalPairedLines(file, moves);
    return sections.flatMap((original) => {
        const left = original.left.filter(
            (line) => !paired.oldLines.has(line.oldLine!),
        );
        const right = original.right.filter(
            (line) => !paired.newLines.has(line.newLine!),
        );
        const hidden =
            original.left.length +
            original.right.length -
            left.length -
            right.length;
        const section = hidden ? { ...original, left, right } : original;
        if (hidden && !left.length && !right.length)
            return [
                {
                    id: original.id,
                    kind: "gap" as const,
                    equal: true,
                    hunkIndex: original.hunkIndex,
                    label: "Equal pairing hidden",
                    left: [],
                    right: [],
                },
            ];
        if (
            section.kind !== "change" ||
            section.move ||
            !section.left.length ||
            !section.right.length
        )
            return [section];
        const diff = compareLines(section.left, section.right);
        // Very dissimilar blocks stay fully visible if the bounded comparison
        // cannot finish. Never discard a change without proving equality.
        if (!diff) return [section];
        let oldIndex = 0;
        let newIndex = 0;
        const result: FlowSection[] = [];
        let change: FlowSection | undefined;
        for (const part of diff) {
            const count = part.count;
            if (!part.added && !part.removed) {
                change = undefined;
                result.push({
                    id: `${section.id}-ws-${oldIndex}-${newIndex}`,
                    kind: "gap",
                    equal: true,
                    hunkIndex: section.hunkIndex,
                    label: `${count} ${count === 1 ? "line" : "lines"} unchanged ignoring whitespace`,
                    left: [],
                    right: [],
                });
                oldIndex += count;
                newIndex += count;
                continue;
            }
            if (!change) {
                change = {
                    id: `${section.id}-edit-${oldIndex}-${newIndex}`,
                    kind: "change",
                    hunkIndex: section.hunkIndex,
                    left: [],
                    right: [],
                };
                result.push(change);
            }
            if (part.removed) {
                change.left.push(
                    ...section.left.slice(oldIndex, oldIndex + count),
                );
                oldIndex += count;
            } else {
                change.right.push(
                    ...section.right.slice(newIndex, newIndex + count),
                );
                newIndex += count;
            }
        }
        return result;
    });
}

/** A wholly equal paired section should disappear, not leave an empty card.
 * Metadata and binary changes always remain available for review.
 */
export function isEqualOnly(
    file: FlowDiffFile,
    sections: FlowSection[],
): boolean {
    return (
        !file.binary &&
        !file.metadata.length &&
        sections.some((section) => section.equal) &&
        !sections.some((section) => section.kind === "change")
    );
}

export type InlinePart = { text: string; changed: boolean };

/** Highlight only the changed middle of a replacement pair. */
export function replacementParts(text: string, other?: string): InlinePart[] {
    if (other === undefined) return [{ text, changed: false }];
    if (text === other) return [{ text, changed: false }];
    const chars = Array.from(text);
    const otherChars = Array.from(other);
    let prefix = 0;
    while (
        prefix < chars.length &&
        prefix < otherChars.length &&
        chars[prefix] === otherChars[prefix]
    ) {
        prefix++;
    }
    let suffix = 0;
    while (
        suffix < chars.length - prefix &&
        suffix < otherChars.length - prefix &&
        chars[chars.length - suffix - 1] ===
            otherChars[otherChars.length - suffix - 1]
    ) {
        suffix++;
    }
    return [
        { text: chars.slice(0, prefix).join(""), changed: false },
        {
            text: chars.slice(prefix, chars.length - suffix).join(""),
            changed: true,
        },
        { text: chars.slice(chars.length - suffix).join(""), changed: false },
    ].filter((part) => part.text.length > 0);
}

/** Map a scroll position by section, retaining progress through unequal blocks. */
export function mappedScrollTop(
    sourceTop: number,
    source: Array<{ top: number; height: number }>,
    target: Array<{ top: number; height: number }>,
): number {
    if (!source.length || source.length !== target.length) return sourceTop;
    const index = source.findIndex(
        (section, i) =>
            sourceTop < section.top + section.height || i === source.length - 1,
    );
    const from = source[index];
    const to = target[index];
    const progress = Math.max(
        0,
        Math.min(1, (sourceTop - from.top) / Math.max(1, from.height)),
    );
    return to.top + progress * to.height;
}

/** A paired excerpt has two independent source streams. In unified layout,
 * keep each stream's context with its changes instead of interleaving files. */
/** Ordinary context appears once; independent expanded excerpts retain both sides. */
export function unifiedLeftLines(
    file: FlowDiffFile,
    section: FlowSection,
): FlowDiffLine[] {
    const shared =
        section.kind === "context" &&
        (!section.expanded ||
            ((file.oldPath || file.path) === file.path &&
                section.left.length === section.right.length &&
                section.left.every(
                    (line, index) => line.text === section.right[index]?.text,
                )));
    return shared ? [] : section.left;
}

export function unifiedSections(sections: FlowSection[]): FlowSection[] {
    const combined = new Set<number>();
    return sections.flatMap((section) => {
        if (section.kind === "gap") return [section];
        const hunk = sections.filter(
            (s) => s.hunkIndex === section.hunkIndex && s.kind !== "gap",
        );
        const original = hunk.filter((s) => !s.expanded);
        if (
            !hunk.some((s) => s.expanded) ||
            !original.length ||
            original.some((s) => s.kind === "context")
        )
            return [section];
        if (combined.has(section.hunkIndex)) return [];
        combined.add(section.hunkIndex);
        return [
            {
                ...original[0],
                kind: "change" as const,
                expanded: true,
                left: hunk.flatMap((s) => s.left),
                right: hunk.flatMap((s) => s.right),
            },
        ];
    });
}
