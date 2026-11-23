import type { DiffCommentsController } from "./FlowDiffComments";
import { For, Show, createEffect, createMemo } from "solid-js";
import { FileLine, type Side } from "./FlowDiffCode";
import type {
    FlowDiffFile,
    FlowDiffLine,
    FlowDiffMove,
    Reconstruction,
} from "./FlowDiffModel";
import { moveLocation, equalMoveLines } from "./FlowDiffModel";

export function pairedMoveLine(
    move: FlowDiffMove,
    line: FlowDiffLine,
    side: Side,
): FlowDiffLine | undefined {
    const endpoint = move[side];
    const number = side === "old" ? line.oldLine : line.newLine;
    if (number === undefined) return undefined;
    const offset = number - endpoint.startLine;
    if (offset < 0 || offset >= endpoint.lineCount) return undefined;
    const otherSide = side === "old" ? "new" : "old";
    if (offset >= move[otherSide].lineCount) return undefined;
    const otherNumber = move[otherSide].startLine + offset;
    const window = move[otherSide].contextWindows.find(
        (item) =>
            otherNumber >= item.startLine &&
            otherNumber < item.startLine + item.lines.length,
    );
    const counterpart = window?.lines[otherNumber - window.startLine];
    return (otherSide === "old"
        ? counterpart?.oldLine
        : counterpart?.newLine) === otherNumber
        ? counterpart
        : undefined;
}

export function MoveOverlay(props: {
    comments?: DiffCommentsController;
    move: FlowDiffMove;
    reconstruction: Reconstruction;
    hideEqual?: boolean;
    file: FlowDiffFile;
    syntax?: Record<string, string>;
    onOpenSource?: (path: string, line: number, side: Side) => void;
}) {
    const hidden = createMemo(() =>
        props.hideEqual ? equalMoveLines(props.move, props.file) : undefined,
    );
    const visibleLines = (lines: FlowDiffLine[]) =>
        lines.filter((line) =>
            props.reconstruction === "old"
                ? !hidden()?.oldLines.has(line.oldLine!)
                : !hidden()?.newLines.has(line.newLine!),
        );
    const endpoint = () =>
        props.reconstruction === "old" ? props.move.old : props.move.new;
    let scroller: HTMLDivElement | undefined;
    createEffect(() => {
        endpoint();
        queueMicrotask(() => {
            const pairedLine =
                scroller?.querySelector<HTMLElement>(".flow-move-paired");
            if (scroller && pairedLine) {
                const position =
                    pairedLine.getBoundingClientRect().top -
                    scroller.getBoundingClientRect().top;
                scroller.scrollTop = Math.max(
                    0,
                    scroller.scrollTop + position - 22,
                );
            }
        });
    });

    return (
        <aside
            class="flow-move-overlay"
            aria-label={`Possible moved-code match: ${props.move.label || props.move.id}`}
            style={{
                "--flow-move-height": `${Math.min(12, Math.max(5, endpoint().lineCount + 3)) * 22}px`,
            }}
        >
            <div class="flow-move-heading">
                <strong>Possible match</strong>
                <Show when={props.move.label}>
                    <span class="flow-move-label" title={props.move.label}>
                        {props.move.label}
                    </span>
                </Show>
            </div>
            <div class="flow-move-route">
                <span>
                    Before{" "}
                    <code title={moveLocation(props.move.old)}>
                        {moveLocation(props.move.old)}
                    </code>
                </span>
                <span aria-hidden="true">→</span>
                <span>
                    After{" "}
                    <code title={moveLocation(props.move.new)}>
                        {moveLocation(props.move.new)}
                    </code>
                </span>
            </div>
            <div class="flow-move-context-label">
                {props.reconstruction === "old" ? "Before" : "After"} context ·
                matched lines highlighted
            </div>
            <div
                class="flow-move-scroll"
                ref={scroller}
                role="region"
                aria-label={`${endpoint().path} context`}
                tabindex={0}
            >
                <For each={endpoint().contextWindows}>
                    {(window, index) => (
                        <>
                            <Show when={index() > 0}>
                                <div class="flow-move-gap">
                                    ··· context omitted ···
                                </div>
                            </Show>
                            <For each={visibleLines(window.lines)}>
                                {(line) => {
                                    const number =
                                        props.reconstruction === "old"
                                            ? line.oldLine
                                            : line.newLine;
                                    const paired =
                                        number !== undefined &&
                                        number >= endpoint().startLine &&
                                        number <
                                            endpoint().startLine +
                                                endpoint().lineCount;
                                    return (
                                        <div
                                            classList={{
                                                "flow-move-paired": paired,
                                            }}
                                        >
                                            <FileLine
                                                comments={props.comments}
                                                file={props.file}
                                                pathOverride={endpoint().path}
                                                line={line}
                                                counterpart={pairedMoveLine(
                                                    props.move,
                                                    line,
                                                    props.reconstruction,
                                                )}
                                                side={props.reconstruction}
                                                syntax={props.syntax}
                                                onOpenSource={
                                                    props.onOpenSource
                                                }
                                            />
                                        </div>
                                    );
                                }}
                            </For>
                        </>
                    )}
                </For>
            </div>
            <Show when={!endpoint().contextComplete}>
                <div class="flow-move-truncated">Showing nearby context</div>
            </Show>
        </aside>
    );
}
