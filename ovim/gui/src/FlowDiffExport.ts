import { PngWriter } from "./pngWriter";
import {
    replacementParts,
    hideEqualChanges,
    equalMoveLines,
    unifiedSections,
    unifiedLeftLines,
    isEqualOnly,
    sectionsForFile,
    sectionsWithMoves,
    moveLocation,
    type FlowDiffFile,
    type FlowDiffLine,
    type FlowDiffMove,
    type FlowDiffReview,
    type Reconstruction,
} from "./FlowDiffModel";
import type { Side } from "./FlowDiffCode";
import { sectionPairing } from "./FlowDiffPairing";

export type DiffExportView = "files" | "guided";

export type DiffExportOptions = {
    view: DiffExportView;
    layout?: "split" | "unified";
    reconstruction: Reconstruction;
    traceMoves?: boolean;
    hideEqual?: boolean;
    showNotes?: boolean;
};

export type DiffExportImage = {
    filename: string;
    svg: string;
    width: number;
    height: number;
};

export type RasterizedDiffExportImage = {
    filename: string;
    blob: Blob;
};

const IMAGE_WIDTH = 1600;
const STRIP_HEIGHT = 1024;
const MARGIN = 54;
const COLUMN_GAP = 24;
const COLUMN_WIDTH = (IMAGE_WIDTH - MARGIN * 2 - COLUMN_GAP) / 2;
const FONT_SIZE = 16;
const GLYPH_WIDTH = 10;
const CODE_CHARS = Math.floor((COLUMN_WIDTH - 82) / GLYPH_WIDTH);
const LINE_HEIGHT = 25;
const MONO = "'DejaVu Sans Mono', Menlo, Consolas, monospace";

type CodeCell = {
    number?: number;
    marker: string;
    kind: FlowDiffLine["kind"];
    text: string;
    fullText: string;
    counterpart?: string;
    offset: number;
    paired?: boolean;
};

type ExportRow =
    | {
          kind: "file";
          height: number;
          lines: string[];
          status: string;
          stats: string;
          continued?: boolean;
      }
    | {
          kind: "note";
          height: number;
          text: string;
          side?: Side;
          overlayTitle?: string;
      }
    | {
          kind: "gap";
          height: number;
          text: string;
          side?: Side;
          overlayTitle?: string;
      }
    | {
          kind: "code";
          height: number;
          left?: CodeCell;
          right?: CodeCell;
          overlaySide?: Side;
          overlayTitle?: string;
      }
    | {
          kind: "overlayHeading";
          height: number;
          text: string;
          side: Side;
          overlayTitle: string;
      };

type FileBlock = {
    file: FlowDiffFile;
    header: ExportRow & { kind: "file" };
    rows: ExportRow[];
};
type PositionedRow = { row: ExportRow; y: number };
type ImageLayout = { rows: PositionedRow[]; bottom: number };

function xml(text: string): string {
    return printable(text)
        .replace(/&/g, "&amp;")
        .replace(/</g, "&lt;")
        .replace(/>/g, "&gt;")
        .replace(/"/g, "&quot;")
        .replace(/'/g, "&apos;");
}

function printable(text: string): string {
    return text.replace(/\t/g, "    ").replace(/[\u0000-\u001f\u007f]/g, "�");
}

function wrapCode(text: string, chars = CODE_CHARS): string[] {
    const characters = Array.from(printable(text));
    if (!characters.length) return [""];
    const chunks: string[] = [];
    for (let start = 0; start < characters.length; start += chars)
        chunks.push(characters.slice(start, start + chars).join(""));
    return chunks;
}

function wrapWords(text: string, limit: number): string[] {
    const words = printable(text).split(/\s+/).filter(Boolean);
    if (!words.length) return [""];
    const lines: string[] = [];
    let current = "";
    for (const word of words) {
        if (current && current.length + word.length + 1 > limit) {
            lines.push(current);
            current = "";
        }
        if (word.length > limit) {
            const chunks = Array.from(word);
            while (chunks.length > limit) {
                lines.push(chunks.splice(0, limit).join(""));
            }
            current = chunks.join("");
        } else {
            current = current ? `${current} ${word}` : word;
        }
    }
    if (current) lines.push(current);
    return lines;
}

function lineNumber(line: FlowDiffLine, side: Side): number | undefined {
    return side === "old" ? line.oldLine : line.newLine;
}

function codeCells(
    line: FlowDiffLine | undefined,
    side: Side,
    counterpart?: FlowDiffLine,
    paired = false,
    chars = CODE_CHARS,
): CodeCell[] {
    if (!line) return [];
    const fullText = printable(line.text);
    return wrapCode(line.text, chars).map((text, index) => ({
        number: index === 0 ? lineNumber(line, side) : undefined,
        marker:
            index === 0
                ? line.kind === "added"
                    ? "+"
                    : line.kind === "removed"
                      ? "−"
                      : ""
                : "↳",
        kind: line.kind,
        text,
        fullText,
        counterpart: counterpart ? printable(counterpart.text) : undefined,
        offset: index * chars,
        paired,
    }));
}

function pairedRows(
    left: FlowDiffLine | undefined,
    right: FlowDiffLine | undefined,
    // Word highlights follow similarity pairing, not row alignment.
    counterparts?: [FlowDiffLine | undefined, FlowDiffLine | undefined],
): ExportRow[] {
    const [leftCounterpart, rightCounterpart] = counterparts ?? [right, left];
    const leftCells = codeCells(left, "old", leftCounterpart);
    const rightCells = codeCells(right, "new", rightCounterpart);
    return Array.from(
        { length: Math.max(leftCells.length, rightCells.length) },
        (_, index) => ({
            kind: "code" as const,
            height: LINE_HEIGHT,
            left: leftCells[index],
            right: rightCells[index],
        }),
    );
}

function moveRows(
    move: FlowDiffMove,
    reconstruction: Reconstruction,
    anchorLines: FlowDiffLine[],
    hidden?: ReturnType<typeof equalMoveLines>,
): ExportRow[] {
    const endpoint = move[reconstruction];
    const side = reconstruction;
    const anchorSide = side === "old" ? "new" : "old";
    const anchor = move[anchorSide];
    const title = `Possible moved-code match · Before ${moveLocation(move.old)} → After ${moveLocation(move.new)}${move.label ? ` · ${move.label}` : ""}`;
    const rows: ExportRow[] = wrapWords(title, CODE_CHARS + 3).map((text) => ({
        kind: "overlayHeading",
        height: 30,
        text,
        side,
        overlayTitle: title,
    }));
    const firstWanted = Math.max(1, endpoint.startLine - 3);
    const lastWanted = endpoint.startLine + endpoint.lineCount + 2;
    const numbered = endpoint.contextWindows
        .flatMap((window) => window.lines)
        .filter((line) => {
            const number = lineNumber(line, side);
            return (
                number !== undefined &&
                number >= firstWanted &&
                number <= lastWanted
            );
        })
        .sort((a, b) => lineNumber(a, side)! - lineNumber(b, side)!);
    const sourceByNumber = new Map(
        numbered.map((line) => [lineNumber(line, side)!, line]),
    );
    const anchorByNumber = new Map(
        anchorLines.map((line) => [lineNumber(line, anchorSide)!, line]),
    );
    const contextLine = (line: FlowDiffLine) => {
        const cells = codeCells(line, side);
        for (const cell of cells)
            rows.push({
                kind: "code",
                height: LINE_HEIGHT,
                [side === "old" ? "left" : "right"]: cell,
                overlaySide: side,
                overlayTitle: title,
            });
    };
    const gap = (start: number, end: number) => {
        if (end < start) return;
        rows.push({
            kind: "gap",
            height: 26,
            text: `Lines ${start}–${end} outside saved context`,
            side,
            overlayTitle: title,
        });
    };
    let previous = firstWanted - 1;
    for (const line of numbered.filter(
        (item) => lineNumber(item, side)! < endpoint.startLine,
    )) {
        const number = lineNumber(line, side)!;
        if (number <= previous) continue;
        gap(previous + 1, number - 1);
        contextLine(line);
        previous = number;
    }
    for (
        let offset = 0;
        offset < Math.max(endpoint.lineCount, anchor.lineCount);
        offset++
    ) {
        const source =
            offset < endpoint.lineCount
                ? sourceByNumber.get(endpoint.startLine + offset)
                : undefined;
        if (offset < endpoint.lineCount && !source)
            throw new Error(
                `Moved source line ${endpoint.startLine + offset} is missing from saved context.`,
            );
        const canonical =
            offset < anchor.lineCount
                ? anchorByNumber.get(anchor.startLine + offset)
                : undefined;
        if (offset < anchor.lineCount && !canonical)
            throw new Error(
                `Canonical move line ${anchor.startLine + offset} is missing from this file.`,
            );
        const hiddenLine = (
            line: FlowDiffLine | undefined,
            lineSide: Reconstruction,
        ) =>
            line &&
            (lineSide === "old"
                ? hidden?.oldLines.has(line.oldLine!)
                : hidden?.newLines.has(line.newLine!));
        const sourceCells = hiddenLine(source, side)
            ? []
            : codeCells(source, side, canonical, Boolean(source));
        const anchorCells = hiddenLine(canonical, anchorSide)
            ? []
            : codeCells(canonical, anchorSide, source, Boolean(canonical));
        for (
            let index = 0;
            index < Math.max(sourceCells.length, anchorCells.length);
            index++
        )
            rows.push({
                kind: "code",
                height: LINE_HEIGHT,
                left: side === "old" ? sourceCells[index] : anchorCells[index],
                right: side === "old" ? anchorCells[index] : sourceCells[index],
                overlaySide: side,
                overlayTitle: title,
            });
    }
    previous = endpoint.startLine + endpoint.lineCount - 1;
    for (const line of numbered.filter(
        (item) => lineNumber(item, side)! > previous,
    )) {
        const number = lineNumber(line, side)!;
        if (number <= previous) continue;
        gap(previous + 1, number - 1);
        contextLine(line);
        previous = number;
    }
    if (!endpoint.contextComplete)
        rows.push({
            kind: "note",
            height: 28,
            text: "Showing nearby saved context",
            side,
            overlayTitle: title,
        });
    return rows;
}

function fileBlock(
    file: FlowDiffFile,
    moves: FlowDiffMove[],
    options: DiffExportOptions,
): FileBlock | undefined {
    const pathLines = wrapWords(file.path, 78);
    const header: ExportRow & { kind: "file" } = {
        kind: "file",
        height: 42 + (pathLines.length - 1) * 23,
        lines: pathLines,
        status: file.status,
        stats: `+${file.additions}  −${file.deletions}`,
    };
    const rows: ExportRow[] = [];
    if (file.label)
        for (const text of wrapWords(file.label, 130))
            rows.push({
                kind: "note",
                height: 28,
                text: `Description: ${text}`,
            });
    if (
        options.view === "guided" &&
        options.showNotes !== false &&
        file.message
    ) {
        rows.push({ kind: "note", height: 28, text: "Agent note" });
        for (const paragraph of file.message.split("\n"))
            for (const text of wrapWords(paragraph, 130))
                rows.push({
                    kind: "note",
                    height: 28,
                    text,
                });
    }
    if (file.oldPath && file.oldPath !== file.path)
        for (const text of wrapWords(`Before path: ${file.oldPath}`, 130))
            rows.push({ kind: "note", height: 28, text });
    for (const metadata of file.metadata)
        for (const text of wrapWords(metadata, 130))
            rows.push({ kind: "note", height: 28, text });
    if (file.binary) {
        rows.push({
            kind: "note",
            height: 28,
            text: "Binary file; no text diff.",
        });
        return { file, header, rows };
    }
    const rawSections =
        options.view === "files" && options.traceMoves && moves.length
            ? sectionsWithMoves(file, moves, options.reconstruction)
            : sectionsForFile(file);
    const sections = options.hideEqual
        ? hideEqualChanges(file, rawSections, moves)
        : rawSections;
    if (options.hideEqual && isEqualOnly(file, sections)) return undefined;
    if (!sections.length)
        rows.push({
            kind: "note",
            height: 28,
            text:
                options.view === "guided"
                    ? "No text changes in this section."
                    : "No text changes in this file.",
        });
    const renderedMoves = new Set<string>();
    for (const section of options.layout === "unified"
        ? unifiedSections(sections)
        : sections) {
        if (section.kind === "gap") {
            rows.push({
                kind: "gap",
                height: 28,
                text: section.label || "Context omitted",
            });
            continue;
        }
        if (options.layout === "unified") {
            for (const side of ["old", "new"] as const) {
                const lines =
                    side === "old"
                        ? unifiedLeftLines(file, section)
                        : section.right;
                const other = side === "old" ? section.right : section.left;
                lines.forEach((line, index) => {
                    for (const cell of codeCells(
                        line,
                        side,
                        other[index],
                        false,
                        Math.floor(
                            (IMAGE_WIDTH - 2 * MARGIN - 82) / GLYPH_WIDTH,
                        ),
                    ))
                        rows.push({
                            kind: "code",
                            height: LINE_HEIGHT,
                            [side === "old" ? "left" : "right"]: cell,
                        });
                });
            }
            if (section.move && !renderedMoves.has(section.move.id)) {
                renderedMoves.add(section.move.id);
                const side = options.reconstruction;
                const endpoint = section.move[side];
                rows.push({
                    kind: "overlayHeading",
                    height: 30,
                    side,
                    text: `Possible match · ${moveLocation(endpoint)}`,
                    overlayTitle: section.move.label || section.move.id,
                });
                const hidden = options.hideEqual
                    ? equalMoveLines(section.move, file)
                    : undefined;
                for (const window of endpoint.contextWindows) {
                    for (const line of window.lines) {
                        if (
                            side === "old"
                                ? hidden?.oldLines.has(line.oldLine!)
                                : hidden?.newLines.has(line.newLine!)
                        )
                            continue;
                        for (const cell of codeCells(
                            line,
                            side,
                            undefined,
                            false,
                            Math.floor(
                                (IMAGE_WIDTH - 2 * MARGIN - 82) / GLYPH_WIDTH,
                            ),
                        ))
                            rows.push({
                                kind: "code",
                                height: LINE_HEIGHT,
                                overlaySide: side,
                                [side === "old" ? "left" : "right"]: cell,
                            });
                    }
                }
            }
            continue;
        }
        if (section.move) {
            const sourceSide = options.reconstruction;
            const sourceLines =
                sourceSide === "old" ? section.left : section.right;
            for (const line of sourceLines)
                rows.push(
                    ...pairedRows(
                        sourceSide === "old" ? line : undefined,
                        sourceSide === "new" ? line : undefined,
                    ),
                );
            if (renderedMoves.has(section.move.id)) continue;
            renderedMoves.add(section.move.id);
            const anchorSide = sourceSide === "old" ? "new" : "old";
            const anchor = section.move[anchorSide];
            const anchorLines = file.hunks
                .flatMap((hunk) => hunk.lines)
                .filter((line) => {
                    const number = lineNumber(line, anchorSide);
                    return (
                        line.kind ===
                            (anchorSide === "old" ? "removed" : "added") &&
                        number !== undefined &&
                        number >= anchor.startLine &&
                        number < anchor.startLine + anchor.lineCount
                    );
                });
            rows.push(
                ...moveRows(
                    section.move,
                    sourceSide,
                    anchorLines,
                    options.hideEqual
                        ? equalMoveLines(section.move, file)
                        : undefined,
                ),
            );
            continue;
        }
        const count = Math.max(section.left.length, section.right.length);
        const pairing =
            section.kind === "change"
                ? sectionPairing(section.left, section.right)
                : undefined;
        for (let index = 0; index < count; index++)
            rows.push(
                ...pairedRows(
                    section.left[index],
                    section.right[index],
                    pairing && [pairing.left[index], pairing.right[index]],
                ),
            );
    }
    return { file, header, rows };
}

function rowSvg(row: ExportRow, y: number, unified = false): string {
    if (row.kind === "file") {
        const label = row.lines
            .map(
                (line, index) =>
                    `<text x="${MARGIN + 14}" y="${y + 27 + index * 23}" fill="#e5e7eb" font-family="${MONO}" font-size="17" font-weight="700">${xml(line)}</text>`,
            )
            .join("");
        return `<rect x="${MARGIN}" y="${y}" width="${IMAGE_WIDTH - 2 * MARGIN}" height="${row.height}" rx="5" fill="#263244"/><text x="${IMAGE_WIDTH - MARGIN - 14}" y="${y + 27}" text-anchor="end" fill="#b4bfce" font-family="${MONO}" font-size="14">${xml(`${row.status}   ${row.stats}${row.continued ? "   continued" : ""}`)}</text>${label}`;
    }
    if (row.kind === "overlayHeading") {
        const x =
            unified || row.side === "old"
                ? MARGIN
                : MARGIN + COLUMN_WIDTH + COLUMN_GAP;
        return `<rect x="${x}" y="${y}" width="${unified ? IMAGE_WIDTH - 2 * MARGIN : COLUMN_WIDTH}" height="${row.height}" fill="#243c56"/><rect x="${x}" y="${y}" width="4" height="${row.height}" fill="#7aa2f7"/><text x="${x + 14}" y="${y + 21}" fill="#b5d5f5" font-family="${MONO}" font-size="14" font-weight="700">${xml(row.text)}</text>`;
    }
    if (row.kind === "note" || row.kind === "gap") {
        const x =
            row.side === "new" ? MARGIN + COLUMN_WIDTH + COLUMN_GAP : MARGIN;
        const width = row.side ? COLUMN_WIDTH : IMAGE_WIDTH - 2 * MARGIN;
        const fill = row.side
            ? "#1d3045"
            : row.kind === "gap"
              ? "#192231"
              : "#161e2c";
        return `<rect x="${x}" y="${y}" width="${width}" height="${row.height}" fill="${fill}"/><text x="${x + 13}" y="${y + 19}" fill="#9ca3af" font-family="${MONO}" font-size="13">${xml(row.text)}</text>`;
    }
    if (unified)
        return cellSvg(
            row.left || row.right,
            MARGIN,
            y,
            Boolean(row.overlaySide),
            IMAGE_WIDTH - 2 * MARGIN,
        );
    const left = cellSvg(row.left, MARGIN, y, row.overlaySide === "old");
    const right = cellSvg(
        row.right,
        MARGIN + COLUMN_WIDTH + COLUMN_GAP,
        y,
        row.overlaySide === "new",
    );
    return left + right;
}

function cellSvg(
    cell: CodeCell | undefined,
    x: number,
    y: number,
    overlay: boolean,
    width = COLUMN_WIDTH,
): string {
    const fill = overlay
        ? "#1d3045"
        : cell?.kind === "added"
          ? "#19362b"
          : cell?.kind === "removed"
            ? "#3b242d"
            : "#161e2c";
    let svg = `<rect x="${x}" y="${y}" width="${width}" height="${LINE_HEIGHT}" fill="${fill}"/>`;
    if (!cell) return svg;
    if (cell.paired)
        svg += `<rect x="${x}" y="${y}" width="4" height="${LINE_HEIGHT}" fill="#7aa2f7"/>`;
    const codeX = x + 78;
    if (cell.counterpart !== undefined) {
        let offset = 0;
        for (const part of replacementParts(cell.fullText, cell.counterpart)) {
            const start = Math.max(cell.offset, offset);
            const end = Math.min(
                cell.offset + Array.from(cell.text).length,
                offset + Array.from(part.text).length,
            );
            if (part.changed && end > start)
                svg += `<rect x="${codeX + (start - cell.offset) * GLYPH_WIDTH}" y="${y + 4}" width="${(end - start) * GLYPH_WIDTH}" height="18" rx="2" fill="${cell.kind === "added" ? "#285a40" : cell.kind === "removed" ? "#713741" : "#355575"}"/>`;
            offset += Array.from(part.text).length;
        }
    }
    const number = cell.number === undefined ? "" : String(cell.number);
    svg += `<text x="${x + 43}" y="${y + 18}" text-anchor="end" fill="#94a3b8" font-family="${MONO}" font-size="13">${xml(number)}</text>`;
    svg += `<text x="${x + 58}" y="${y + 18}" fill="${cell.kind === "added" ? "#7bdca1" : cell.kind === "removed" ? "#f59ba5" : "#94a3b8"}" font-family="${MONO}" font-size="14">${xml(cell.marker)}</text>`;
    if (cell.text)
        svg += `<text x="${codeX}" y="${y + 18}" fill="#e5e7eb" font-family="${MONO}" font-size="${FONT_SIZE}" textLength="${Array.from(cell.text).length * GLYPH_WIDTH}" lengthAdjust="spacingAndGlyphs" xml:space="preserve">${xml(cell.text)}</text>`;
    return svg;
}

function safeStem(title: string): string {
    const stem = title
        .normalize("NFKD")
        .replace(/[^a-zA-Z0-9]+/g, "-")
        .replace(/^-+|-+$/g, "")
        .toLowerCase()
        .slice(0, 64);
    return stem || "diff-review";
}

function changedLineCounts(files: FlowDiffFile[]): Map<string, number> {
    const counts = new Map<string, number>();
    for (const file of files) {
        for (const hunk of file.hunks) {
            for (const line of hunk.lines) {
                if (line.kind === "context") continue;
                const path =
                    line.kind === "removed"
                        ? file.oldPath || file.path
                        : file.path;
                const number =
                    line.kind === "removed" ? line.oldLine : line.newLine;
                const key = JSON.stringify([
                    path,
                    line.kind,
                    number,
                    line.text,
                ]);
                counts.set(key, (counts.get(key) || 0) + 1);
            }
        }
    }
    return counts;
}

function assertGuidedCoverage(
    canonical: FlowDiffFile[],
    guided: FlowDiffFile[],
): void {
    const expected = changedLineCounts(canonical);
    const actual = changedLineCounts(guided);
    if (
        expected.size !== actual.size ||
        [...expected].some(([key, count]) => actual.get(key) !== count)
    )
        throw new Error(
            "Guided sections do not cover every canonical changed line; image export was stopped.",
        );
    for (const file of canonical) {
        if (
            file.binary &&
            !guided.some((item) => item.path === file.path && item.binary)
        )
            throw new Error(`Guided sections omit binary file ${file.path}.`);
        for (const metadata of file.metadata) {
            if (
                !guided.some(
                    (item) =>
                        item.path === file.path &&
                        item.metadata.includes(metadata),
                )
            )
                throw new Error(
                    `Guided sections omit metadata for ${file.path}.`,
                );
        }
    }
}

/** Lay out every selected canonical line before creating any raster image. */
export function buildDiffExportImage(
    review: FlowDiffReview,
    options: DiffExportOptions,
): DiffExportImage {
    options = { ...options, layout: options.layout ?? review.layout };
    const files = options.view === "guided" ? review.guidedFiles : review.files;
    if (!files)
        throw new Error("Guided review is unavailable for this comparison.");
    if (options.view === "guided") assertGuidedCoverage(review.files, files);
    const moves = options.view === "files" ? review.moves || [] : [];
    const titleLines = wrapWords(review.title, 116);
    const bodyTop = 151 + titleLines.length * 24;
    const provenance = review.provenance;
    const source = provenance
        ? `Change accounting · Snapshot base: ${provenance.baseLabel} · ${provenance.comparisonBaseOid}${provenance.snapshotId ? ` · ${provenance.snapshotId}` : ""}`
        : "Change accounting from the selected review";
    const sourceLines = wrapWords(source, 115);
    const footerSpace = Math.max(74, 38 + sourceLines.length * 18);
    const draft: ImageLayout = { rows: [], bottom: bodyTop };
    const append = (row: ExportRow) => {
        draft.rows.push({ row, y: draft.bottom });
        draft.bottom += row.height;
    };
    const hiddenReviewed = review.showChecked
        ? 0
        : files.filter((file) => file.checked).length;
    for (const file of files) {
        if (file.checked && !review.showChecked) continue;
        const block = fileBlock(file, moves, options);
        if (!block) continue;
        append(block.header);
        for (const row of block.rows) append(row);
    }
    const additions = review.files.reduce(
        (sum, file) => sum + file.additions,
        0,
    );
    const deletions = review.files.reduce(
        (sum, file) => sum + file.deletions,
        0,
    );
    const moveCount = review.moves?.length ?? 0;
    const plural = (count: number, one: string, many = `${one}s`) =>
        `${count} ${count === 1 ? one : many}`;
    const coverage = `${hiddenReviewed ? `${plural(hiddenReviewed, "reviewed item")} hidden · ` : ""}${options.hideEqual ? "Equal paired changes hidden · original patch: " : ""}${[
        plural(review.files.length, "file"),
        moveCount
            ? plural(
                  moveCount,
                  "possible moved-code match",
                  "possible moved-code matches",
              )
            : "",
        `+${additions} −${deletions}`,
    ]
        .filter(Boolean)
        .join(" · ")}`;
    const scope = options.view === "guided" ? "Guided sections" : "Files";
    const stem = safeStem(review.title);

    const height = Math.max(420, draft.bottom + footerSpace);
    const header = titleLines
        .map(
            (line, lineIndex) =>
                `<text x="${MARGIN}" y="${72 + lineIndex * 24}" fill="#e5e7eb" font-family="${MONO}" font-size="20" font-weight="700">${xml(line)}</text>`,
        )
        .join("");
    const rows = draft.rows.length
        ? draft.rows
              .map(({ row, y }) => rowSvg(row, y, options.layout === "unified"))
              .join("")
        : rowSvg(
              {
                  kind: "note",
                  height: 28,
                  text: hiddenReviewed
                      ? "All visible changes reviewed"
                      : options.hideEqual
                        ? "No unequal changes"
                        : "No changes",
              },
              bodyTop,
          );
    const footer = sourceLines
        .map(
            (line, lineIndex) =>
                `<text x="${MARGIN}" y="${height - footerSpace + 32 + lineIndex * 18}" fill="#9ca3af" font-family="${MONO}" font-size="12">${xml(line)}</text>`,
        )
        .join("");
    const viewLabel =
        options.view === "guided"
            ? "GUIDED SECTIONS"
            : options.traceMoves
              ? options.reconstruction === "old"
                  ? "MOVED CODE · BEFORE CONTEXT"
                  : "MOVED CODE · AFTER CONTEXT"
              : "FILE CHANGES";
    const svg = [
        `<svg xmlns="http://www.w3.org/2000/svg" width="${IMAGE_WIDTH}" height="${height}" viewBox="0 0 ${IMAGE_WIDTH} ${height}">`,
        `<rect width="${IMAGE_WIDTH}" height="${height}" fill="#111827"/>`,
        `<text x="${MARGIN}" y="38" fill="#7aa2f7" font-family="${MONO}" font-size="14" font-weight="700">DIFF REVIEW · ${viewLabel} · ${options.layout === "unified" ? "UNIFIED" : "SIDE BY SIDE"}</text>`,
        header,
        `<text x="${MARGIN}" y="${bodyTop - 55}" fill="#9ca3af" font-family="${MONO}" font-size="13">${xml(coverage)}</text>`,
        `<line x1="${MARGIN}" x2="${IMAGE_WIDTH - MARGIN}" y1="${bodyTop - 44}" y2="${bodyTop - 44}" stroke="#374151"/>`,
        `<text x="${MARGIN + 8}" y="${bodyTop - 19}" fill="#b4bfce" font-family="${MONO}" font-size="13" font-weight="700">${options.layout === "unified" ? "CHANGES" : "BEFORE"}</text>`,
        options.layout === "unified"
            ? ""
            : `<text x="${MARGIN + COLUMN_WIDTH + COLUMN_GAP + 8}" y="${bodyTop - 19}" fill="#b4bfce" font-family="${MONO}" font-size="13" font-weight="700">AFTER</text>`,
        rows,
        `<line x1="${MARGIN}" x2="${IMAGE_WIDTH - MARGIN}" y1="${height - footerSpace + 12}" y2="${height - footerSpace + 12}" stroke="#374151"/>`,
        footer,
        `<text x="${IMAGE_WIDTH - MARGIN}" y="${height - 34}" text-anchor="end" fill="#9ca3af" font-family="${MONO}" font-size="12">${xml(scope)}</text>`,
        "</svg>",
    ].join("");
    return {
        filename: `${stem}-${options.view}.png`,
        svg,
        width: IMAGE_WIDTH,
        height,
    };
}

/** Render bounded strips into one PNG; tall reviews never require a giant canvas. */
export async function rasterizeDiffExportImage(
    page: DiffExportImage,
): Promise<RasterizedDiffExportImage> {
    const png = new PngWriter(page.width, page.height);
    const canvas = document.createElement("canvas");
    canvas.width = page.width;
    try {
        for (let top = 0; top < page.height; top += STRIP_HEIGHT) {
            const height = Math.min(STRIP_HEIGHT, page.height - top);
            // A small SVG viewport also avoids image-decoder dimension limits.
            const svg = page.svg.replace(
                /<svg[^>]*>/,
                `<svg xmlns="http://www.w3.org/2000/svg" width="${page.width}" height="${height}" viewBox="0 ${top} ${page.width} ${height}">`,
            );
            const url = URL.createObjectURL(
                new Blob([svg], { type: "image/svg+xml;charset=utf-8" }),
            );
            try {
                const image = new Image();
                await new Promise<void>((resolve, reject) => {
                    image.onload = () => resolve();
                    image.onerror = () =>
                        reject(new Error(`Could not render ${page.filename}.`));
                    image.src = url;
                });
                canvas.height = height;
                const context = canvas.getContext("2d", {
                    willReadFrequently: true,
                });
                if (!context)
                    throw new Error(
                        "Image export is unavailable in this browser.",
                    );
                context.drawImage(image, 0, 0);
                png.write(context.getImageData(0, 0, page.width, height).data);
                // Let progress repaint and controls respond between strips.
                await new Promise<void>((resolve) => setTimeout(resolve, 0));
            } finally {
                URL.revokeObjectURL(url);
            }
        }
        return { filename: page.filename, blob: png.finish() };
    } finally {
        canvas.width = 0;
        canvas.height = 0;
    }
}
