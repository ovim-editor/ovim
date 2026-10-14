import { CommentedLine, type DiffCommentsController } from "./FlowDiffComments";
import {
    replacementParts,
    type FlowDiffFile,
    type FlowDiffLine,
} from "./FlowDiffModel";

export type Side = "old" | "new";

function lineParts(line: FlowDiffLine, counterpart: FlowDiffLine | undefined) {
    return replacementParts(line.text, counterpart?.text);
}

export function FileLine(props: {
    comments?: DiffCommentsController;
    file: FlowDiffFile;
    pathOverride?: string;
    line: FlowDiffLine;
    counterpart?: FlowDiffLine;
    side: Side;
    syntax?: Record<string, string>;
    onNavigateReviewLine?: (line: number) => void;
    onOpenSource?: (path: string, line: number, side: Side) => void;
}) {
    const number = () =>
        props.side === "old" ? props.line.oldLine : props.line.newLine;
    const canOpen = () => number() !== undefined && Boolean(props.onOpenSource);
    const open = () => {
        if (canOpen())
            props.onOpenSource?.(
                props.pathOverride ||
                    (props.side === "old"
                        ? props.file.oldPath || props.file.path
                        : props.file.path),
                number()!,
                props.side,
            );
        else if (props.line.reviewLine !== undefined)
            props.onNavigateReviewLine?.(props.line.reviewLine);
    };
    const coloredText = () => {
        const parts = lineParts(props.line, props.counterpart);
        if (!props.syntax || !props.line.highlights?.length) {
            return parts.map((part) => (
                <span classList={{ "flow-word-change": part.changed }}>
                    {part.text}
                </span>
            ));
        }
        const boundaries = new Set([0, props.line.text.length]);
        for (const highlight of props.line.highlights) {
            boundaries.add(
                Math.max(0, Math.min(props.line.text.length, highlight.start)),
            );
            boundaries.add(
                Math.max(0, Math.min(props.line.text.length, highlight.end)),
            );
        }
        let offset = 0;
        for (const part of parts) {
            boundaries.add(offset);
            offset += part.text.length;
            boundaries.add(offset);
        }
        const edges = [...boundaries].sort((a, b) => a - b);
        return edges.slice(0, -1).map((start, index) => {
            const end = edges[index + 1];
            const highlight = props.line.highlights?.find(
                (item) => item.start <= start && item.end >= end,
            );
            const changed = parts.some((part, partIndex) => {
                const before = parts
                    .slice(0, partIndex)
                    .reduce((sum, item) => sum + item.text.length, 0);
                return (
                    part.changed &&
                    start >= before &&
                    end <= before + part.text.length
                );
            });
            return (
                <span
                    classList={{ "flow-word-change": changed }}
                    style={{
                        color: highlight
                            ? props.syntax?.[highlight.token]
                            : undefined,
                    }}
                >
                    {props.line.text.slice(start, end)}
                </span>
            );
        });
    };
    return (
        <CommentedLine
            comments={props.comments}
            sourceText={props.line.text}
            target={
                number() === undefined
                    ? undefined
                    : {
                          path:
                              props.pathOverride ||
                              (props.side === "old"
                                  ? props.file.oldPath || props.file.path
                                  : props.file.path),
                          line: number()!,
                          side: props.side,
                      }
            }
        >
            <div
                class={`flow-code-line ${props.line.kind}`}
                data-source-path={
                    props.pathOverride ||
                    (props.side === "old"
                        ? props.file.oldPath || props.file.path
                        : props.file.path)
                }
                data-source-line={number()}
                data-source-side={props.side}
            >
                <button
                    type="button"
                    class="flow-line-number"
                    aria-label={`${props.side === "old" ? "Before" : "After"} line ${number() ?? "unavailable"}${canOpen() ? ", open source" : ""}`}
                    disabled={
                        !canOpen() &&
                        (props.line.reviewLine === undefined ||
                            !props.onNavigateReviewLine)
                    }
                    onClick={open}
                >
                    {number() ?? ""}
                </button>
                <span class="flow-line-mark" aria-hidden="true">
                    {props.line.kind === "added"
                        ? "+"
                        : props.line.kind === "removed"
                          ? "−"
                          : ""}
                </span>
                <code class="flow-line-text">{coloredText()}</code>
            </div>
        </CommentedLine>
    );
}
