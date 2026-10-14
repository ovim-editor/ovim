import { createDiffComments, type CommentInput } from "./FlowDiffComments";
import {
    For,
    Show,
    createEffect,
    createMemo,
    createSignal,
    onCleanup,
    onMount,
} from "solid-js";
import type { JSX } from "solid-js";
import {
    mappedScrollTop,
    hideEqualChanges,
    isEqualOnly,
    sectionsForFile,
    unifiedSections as combineUnifiedSections,
    unifiedLeftLines,
    sectionsWithMoves,
    type FlowDiffLine,
    type FlowDiffFile,
    type FlowDiffReview,
    type FlowSection,
    type Reconstruction,
} from "./FlowDiffModel";
import { FileLine, type Side } from "./FlowDiffCode";
import { MoveOverlay, pairedMoveLine } from "./FlowDiffMove";
import {
    buildDiffExportImage,
    rasterizeDiffExportImage,
} from "./FlowDiffExport";
import { downloadDiffImages, type DiffImageFile } from "./diffImageDownload";
import "./FlowDiff.css";

export type FlowDiffProps = {
    review: FlowDiffReview;
    syntax?: Record<string, string>;
    onNavigateReviewLine?: (line: number) => void;
    onOpenSource?: (path: string, line: number, side: Side) => void;
    onDefinition?: (
        path: string,
        line: number,
        side: Side,
        column: number,
    ) => void;
    onComment?: (comment: CommentInput) => Promise<void>;
    onLayoutChange?: (layout: "split" | "unified") => void;
    onExport?: (file: DiffImageFile) => Promise<boolean>;
    onAction?: (
        key:
            | "show_checked"
            | `check:${string}`
            | "q"
            | "r"
            | "clear_comments"
            | `remove_comment:${number}`
            | "toggle_overlay"
            | "open_saved_overlay"
            | "return_to_live_diff",
    ) => void;
    onExpandContext?: (id: string, up: boolean) => void;
    onCoreKey?: (key: ":" | " ") => void;
};

function defaultReviewView(review: FlowDiffReview): "files" | "guided" {
    return review.custom &&
        review.overlay?.mode === "saved" &&
        review.guidedFiles?.length
        ? "guided"
        : "files";
}

export default function FlowDiff(props: FlowDiffProps) {
    let panel: HTMLElement | undefined;
    let searchInput: HTMLInputElement | undefined;
    const [searchOpen, setSearchOpen] = createSignal(false);
    const [searchText, setSearchText] = createSignal("");
    const [searchCount, setSearchCount] = createSignal(0);
    const [searchIndex, setSearchIndex] = createSignal(-1);
    let searchMatches: HTMLElement[] = [];
    const openSearch = () => {
        setSearchOpen(true);
        queueMicrotask(() => {
            searchInput?.focus();
            searchInput?.select();
        });
    };
    const selectSearchMatch = (index: number) => {
        searchMatches.forEach((line) =>
            line.classList.remove("flow-search-current"),
        );
        if (!searchMatches.length) {
            setSearchIndex(-1);
            return;
        }
        const next = (index + searchMatches.length) % searchMatches.length;
        setSearchIndex(next);
        const line = searchMatches[next];
        line.classList.add("flow-search-current");
        line.scrollIntoView?.({ block: "center", inline: "nearest" });
    };
    const stepSearch = (direction: number) =>
        selectSearchMatch(searchIndex() + direction);
    onMount(() => {
        panel?.addEventListener("ovim-diff-find", openSearch);
        onCleanup(() =>
            panel?.removeEventListener("ovim-diff-find", openSearch),
        );
    });
    const comments = createDiffComments({
        comments: () => props.review.comments ?? [],
        save: props.onComment,
        remove: (id) => props.onAction?.(`remove_comment:${id}`),
    });
    const [hideEqual, setHideEqual] = createSignal(false);
    const [showNotes, setShowNotes] = createSignal(true);
    const [exporting, setExporting] = createSignal(false);
    const [exportMessage, setExportMessage] = createSignal("");
    const [exportFailed, setExportFailed] = createSignal(false);
    const [selectedId, setSelectedId] = createSignal("");
    const [view, setView] = createSignal<"files" | "guided">(
        defaultReviewView(props.review),
    );
    const [layout, setLayout] = createSignal<"split" | "unified">(
        props.review.layout,
    );
    const [ribbons, setRibbons] = createSignal<
        Array<{ id: string; path: string; kind: string }>
    >([]);
    const [activeHunk, setActiveHunk] = createSignal(0);
    const [activeSectionId, setActiveSectionId] = createSignal("");
    const [reconstruction, setReconstruction] =
        createSignal<Reconstruction>("old");
    const [traceMoves, setTraceMoves] = createSignal(false);
    const [activeMoveId, setActiveMoveId] = createSignal("");
    const effectiveView = createMemo<"files" | "guided">(() =>
        view() === "guided" &&
        props.review.custom &&
        props.review.guidedFiles?.length
            ? "guided"
            : "files",
    );
    const hasNotes = createMemo(
        () =>
            effectiveView() === "guided" &&
            props.review.guidedFiles?.some((item) => item.message) === true,
    );

    async function exportImages() {
        if (exporting()) return;
        const review = props.review;
        const options = {
            view: effectiveView(),
            layout: layout(),
            reconstruction: reconstruction(),
            traceMoves: traceMoves(),
            hideEqual: hideEqual(),
            showNotes: showNotes(),
        };
        setExporting(true);
        setExportMessage("");
        setExportFailed(false);
        try {
            await document.fonts?.ready;
            const image = buildDiffExportImage(review, options);
            const file = await rasterizeDiffExportImage(image);
            if (props.onExport) {
                if (await props.onExport(file))
                    setExportMessage(`Saved ${file.filename}`);
            } else {
                downloadDiffImages(file);
                setExportMessage(`Download started · ${file.filename}`);
            }
        } catch (error) {
            setExportFailed(true);
            setExportMessage(
                error instanceof Error ? error.message : String(error),
            );
        } finally {
            setExporting(false);
        }
    }
    let reviewIdentity = "";
    let leftScroller: HTMLDivElement | undefined;
    let rightScroller: HTMLDivElement | undefined;
    let bridge: HTMLDivElement | undefined;
    let unifiedScroller: HTMLDivElement | undefined;
    const expectedScrolls = new WeakMap<HTMLElement, number>();
    let pendingBracket = "";
    let pendingGo = false;
    let resizeObserver: ResizeObserver | undefined;
    const leftSections = new Map<string, HTMLElement>();
    const rightSections = new Map<string, HTMLElement>();
    const unifiedSections = new Map<string, HTMLElement>();

    const sourceFiles = createMemo(() =>
        effectiveView() === "guided" && props.review.guidedFiles?.length
            ? props.review.guidedFiles
            : props.review.files,
    );
    const sectionKey = (fileId: string, sectionId: string) =>
        `${fileId}\0${sectionId}`;
    const sectionsByFile = createMemo(
        () =>
            new Map(
                sourceFiles().map((item) => {
                    const sections =
                        props.review.custom &&
                        effectiveView() === "files" &&
                        traceMoves() &&
                        props.review.moves?.length
                            ? sectionsWithMoves(
                                  item,
                                  props.review.moves,
                                  reconstruction(),
                              )
                            : sectionsForFile(item);
                    return [
                        item.id,
                        hideEqual()
                            ? hideEqualChanges(
                                  item,
                                  sections,
                                  effectiveView() === "files"
                                      ? props.review.moves
                                      : [],
                              )
                            : sections,
                    ];
                }),
            ),
    );
    const fileSections = (item: FlowDiffFile) =>
        sectionsByFile().get(item.id) ?? [];
    const checkItem = (item: FlowDiffFile) => {
        props.onAction?.(
            `check:${effectiveView() === "guided" ? "section" : "file"}:${item.id}`,
        );
        panel?.focus();
    };
    const checkedCount = createMemo(
        () => sourceFiles().filter((item) => item.checked).length,
    );
    const visibleFiles = createMemo(() =>
        sourceFiles().filter(
            (item) =>
                (!item.checked || props.review.showChecked) &&
                (!hideEqual() || !isEqualOnly(item, fileSections(item))),
        ),
    );
    const allSections = createMemo(() =>
        visibleFiles().flatMap((item) =>
            fileSections(item).map((section) => ({ file: item, section })),
        ),
    );
    const unplacedComments = createMemo(() =>
        (props.review.comments ?? []).filter(
            (comment) =>
                !visibleFiles().some((item) =>
                    fileSections(item).some((section) =>
                        (comment.side === "old"
                            ? section.left
                            : section.right
                        ).some(
                            (line) =>
                                (comment.side === "old"
                                    ? item.oldPath || item.path
                                    : item.path) === comment.path &&
                                (comment.side === "old"
                                    ? line.oldLine
                                    : line.newLine) === comment.line &&
                                line.text === comment.excerpt,
                        ),
                    ),
                ),
        ),
    );
    const file = createMemo(
        () =>
            visibleFiles().find((item) => item.id === selectedId()) ||
            visibleFiles()[0],
    );
    const sections = createMemo(() => (file() ? fileSections(file()!) : []));
    const changes = createMemo(() =>
        visibleFiles().flatMap((item) =>
            fileSections(item)
                .filter((section) => section.kind === "change")
                .map((section) => ({
                    fileId: item.id,
                    sectionId: section.id,
                    hunkIndex: section.hunkIndex,
                })),
        ),
    );
    const activeChange = createMemo(() =>
        changes().findIndex(
            (change) =>
                change.fileId === file()?.id &&
                change.sectionId === activeSectionId(),
        ),
    );

    createEffect(() => setLayout(props.review.layout));
    createEffect(() => {
        const review = props.review;
        const identity = [
            review.title,
            review.custom,
            review.overlay?.mode ?? "",
            review.overlay?.mode === "saved"
                ? (review.provenance?.snapshotId ?? "")
                : "",
        ].join("\0");
        if (reviewIdentity && identity !== reviewIdentity) {
            setSelectedId("");
            setActiveSectionId("");
            setActiveHunk(0);
            setView(defaultReviewView(review));
        }
        reviewIdentity = identity;
    });
    createEffect(() => {
        const first = changes()[0];
        if (!visibleFiles().some((item) => item.id === selectedId())) {
            setSelectedId(first?.fileId ?? visibleFiles()[0]?.id ?? "");
        }
        if (
            !changes().some(
                (change) =>
                    change.fileId === selectedId() &&
                    change.sectionId === activeSectionId(),
            )
        ) {
            const next =
                changes().find((change) => change.fileId === selectedId()) ??
                first;
            setActiveSectionId(next?.sectionId ?? "");
            setActiveHunk(next?.hunkIndex ?? 0);
        }
    });

    function boxes(side: Side) {
        const scroller = side === "old" ? leftScroller : rightScroller;
        if (!scroller) return [];
        return Array.from(
            scroller.querySelectorAll<HTMLElement>(
                ".flow-file-heading, .flow-file-metadata, .flow-section, .flow-empty",
            ),
        ).map((element) => ({
            top: elementTop(element, scroller),
            height: element.offsetHeight || 1,
            viewportTop: elementTop(element, scroller) - scroller.scrollTop,
        }));
    }

    function sectionBoxes(side: Side) {
        const scroller = side === "old" ? leftScroller : rightScroller;
        const elements = side === "old" ? leftSections : rightSections;
        return allSections().map(({ file: item, section }) => {
            const element = elements.get(sectionKey(item.id, section.id));
            return {
                top: element && scroller ? elementTop(element, scroller) : 0,
                height: element?.offsetHeight ?? 1,
                viewportTop:
                    (element && scroller ? elementTop(element, scroller) : 0) -
                    (scroller?.scrollTop ?? 0),
            };
        });
    }

    function elementTop(element: HTMLElement, scroller: HTMLElement) {
        const elementRect = element.getBoundingClientRect();
        const scrollerRect = scroller.getBoundingClientRect();
        if (!elementRect.top && !scrollerRect.top && element.offsetTop)
            return element.offsetTop;
        return elementRect.top - scrollerRect.top + scroller.scrollTop;
    }

    function setScrollTop(scroller: HTMLElement, top: number) {
        scroller.scrollTop = top;
        expectedScrolls.set(scroller, scroller.scrollTop);
    }

    function consumeExpectedScroll(scroller: HTMLElement) {
        const expected = expectedScrolls.get(scroller);
        if (expected === undefined) return false;
        expectedScrolls.delete(scroller);
        return Math.abs(scroller.scrollTop - expected) < 1;
    }

    function measure() {
        if (!bridge || layout() !== "split") return;
        const left = sectionBoxes("old");
        const right = sectionBoxes("new");
        const height = bridge.clientHeight;
        setRibbons(
            allSections().flatMap(({ file: item, section }, index) => {
                if (section.kind !== "change") return [];
                const a = left[index];
                const b = right[index];
                if (
                    !a ||
                    !b ||
                    (a.viewportTop > height && b.viewportTop > height) ||
                    (a.viewportTop + a.height < 0 &&
                        b.viewportTop + b.height < 0)
                )
                    return [];
                const topA = a.viewportTop;
                const topB = b.viewportTop;
                const bottomA = topA + Math.max(a.height, 3);
                const bottomB = topB + Math.max(b.height, 3);
                const width = bridge.clientWidth;
                const curve = width * 0.48;
                return [
                    {
                        id: sectionKey(item.id, section.id),
                        kind:
                            section.left.length && section.right.length
                                ? "replace"
                                : section.left.length
                                  ? "remove"
                                  : "add",
                        path: `M 0 ${topA} C ${curve} ${topA}, ${width - curve} ${topB}, ${width} ${topB} L ${width} ${bottomB} C ${width - curve} ${bottomB}, ${curve} ${bottomA}, 0 ${bottomA} Z`,
                    },
                ];
            }),
        );
    }

    function syncScroll(source: Side) {
        if (!leftScroller || !rightScroller) return;
        const from = source === "old" ? leftScroller : rightScroller;
        const to = source === "old" ? rightScroller : leftScroller;
        if (consumeExpectedScroll(from)) return;
        const elements = source === "old" ? leftSections : rightSections;
        const entries = allSections().filter(
            ({ section }) => section.kind === "change",
        );
        const visible =
            [...entries].reverse().find(({ file: item, section }) => {
                const element = elements.get(sectionKey(item.id, section.id));
                return (
                    element && elementTop(element, from) <= from.scrollTop + 8
                );
            }) ?? entries[0];
        if (visible) {
            setSelectedId(visible.file.id);
            setActiveHunk(visible.section.hunkIndex);
            setActiveSectionId(visible.section.id);
            if (visible.section.move) setActiveMoveId(visible.section.move.id);
        }
        setScrollTop(
            to,
            mappedScrollTop(
                from.scrollTop,
                boxes(source),
                boxes(source === "old" ? "new" : "old"),
            ),
        );
        measure();
    }

    function goToSection(id: string, navigate = true, fileId = file()?.id) {
        const entry = allSections().find(
            ({ file: item, section }) =>
                item.id === fileId && section.id === id,
        );
        if (!entry) return;
        const section = entry.section;
        setSelectedId(entry.file.id);
        setActiveHunk(section.hunkIndex);
        if (section.kind === "change") setActiveSectionId(section.id);
        if (section.move) setActiveMoveId(section.move.id);
        const key = sectionKey(entry.file.id, id);
        const left = leftSections.get(key);
        const right = rightSections.get(key);
        const unified = unifiedSections.get(key);
        if (leftScroller && left)
            setScrollTop(
                leftScroller,
                Math.max(0, elementTop(left, leftScroller) - 8),
            );
        if (rightScroller && right)
            setScrollTop(
                rightScroller,
                Math.max(0, elementTop(right, rightScroller) - 8),
            );
        if (unifiedScroller && unified)
            setScrollTop(
                unifiedScroller,
                Math.max(0, elementTop(unified, unifiedScroller) - 8),
            );
        measure();
        const reviewLine = [...section.left, ...section.right].find(
            (line) => line.reviewLine !== undefined,
        )?.reviewLine;
        if (navigate && reviewLine !== undefined)
            props.onNavigateReviewLine?.(reviewLine);
    }

    function goToFile(id: string) {
        if (!visibleFiles().some((item) => item.id === id)) return;
        setSelectedId(id);
        const scrollToHeading = (scroller?: HTMLElement) => {
            const group = Array.from(
                scroller?.querySelectorAll<HTMLElement>(".flow-file-group") ??
                    [],
            ).find((element) => element.dataset.fileId === id);
            if (scroller && group)
                setScrollTop(scroller, elementTop(group, scroller));
        };
        scrollToHeading(leftScroller);
        scrollToHeading(rightScroller);
        scrollToHeading(unifiedScroller);
        measure();
    }

    function stepChange(direction: number) {
        const entries = changes();
        if (!entries.length) return;
        const current = activeChange();
        const next =
            entries[
                ((current < 0 ? (direction > 0 ? -1 : 0) : current) +
                    direction +
                    entries.length) %
                    entries.length
            ];
        if (!next) return;
        setSelectedId(next.fileId);
        setActiveHunk(next.hunkIndex);
        setActiveSectionId(next.sectionId);
        queueMicrotask(() => goToSection(next.sectionId, true, next.fileId));
    }

    function changeReconstruction(next: Reconstruction) {
        if (next === reconstruction()) return;
        const anchorSide = reconstruction() === "old" ? "new" : "old";
        const move =
            props.review.moves?.find((item) => item.id === activeMoveId()) ??
            props.review.moves?.find(
                (item) =>
                    item[anchorSide].path ===
                    (anchorSide === "old"
                        ? file()?.oldPath || file()?.path
                        : file()?.path),
            );
        setReconstruction(next);
        if (!move) return;
        const anchor = next === "old" ? move.new : move.old;
        const target = visibleFiles().find(
            (item) =>
                (next === "old" ? item.path : item.oldPath || item.path) ===
                anchor.path,
        );
        if (target) setSelectedId(target.id);
        queueMicrotask(() => {
            const section = sections().find(
                (item) => item.move?.id === move.id,
            );
            if (section) goToSection(section.id, false);
        });
    }

    function updateUnifiedActive() {
        if (!unifiedScroller || consumeExpectedScroll(unifiedScroller)) return;
        const threshold = unifiedScroller.scrollTop + 8;
        const visible = [...allSections()]
            .reverse()
            .filter(({ section }) => section.kind === "change")
            .find(({ file: item, section }) => {
                const element = unifiedSections.get(
                    sectionKey(item.id, section.id),
                );
                return (
                    element &&
                    elementTop(element, unifiedScroller!) <= threshold
                );
            });
        if (visible) {
            setSelectedId(visible.file.id);
            setActiveSectionId(visible.section.id);
            setActiveHunk(visible.section.hunkIndex);
            if (visible.section.move) setActiveMoveId(visible.section.move.id);
        }
    }

    let expansionAnchor: Array<{
        scroller: HTMLElement;
        file: string;
        section: string;
        sourceSelector?: string;
        top: number;
    }> = [];
    function expandContext(
        item: FlowDiffFile,
        section: FlowSection,
        up: boolean,
    ) {
        const context = item.hunks[section.hunkIndex]?.context;
        if (
            !context ||
            !(up ? context.canExpandUp : context.canExpandDown) ||
            !props.onExpandContext
        )
            return;
        expansionAnchor = [];
        for (const [scroller, elements] of [
            [leftScroller, leftSections],
            [rightScroller, rightSections],
            [unifiedScroller, unifiedSections],
        ] as const) {
            if (!scroller) continue;
            // Anchor the original code, not newly revealed lines or a disappearing gap.
            const original = fileSections(item).find(
                (s) =>
                    s.hunkIndex === section.hunkIndex &&
                    s.kind !== "gap" &&
                    !s.expanded,
            );
            const element =
                original && elements.get(sectionKey(item.id, original.id));
            const source =
                element?.querySelector<HTMLElement>(
                    ".flow-code-line.removed, .flow-code-line.added",
                ) ?? element?.querySelector<HTMLElement>(".flow-code-line");
            if (element && original)
                expansionAnchor.push({
                    scroller,
                    file: item.id,
                    section: original.id,
                    sourceSelector: source?.dataset.sourceLine
                        ? `[data-source-line="${source.dataset.sourceLine}"][data-source-side="${source.dataset.sourceSide}"]`
                        : undefined,
                    top: (source ?? element).getBoundingClientRect().top,
                });
        }
        props.onExpandContext(context.id, up);
    }
    createEffect(() => {
        props.review;
        if (!expansionAnchor.length) return;
        requestAnimationFrame(() => {
            for (const anchor of expansionAnchor) {
                const elements =
                    anchor.scroller === leftScroller
                        ? leftSections
                        : anchor.scroller === rightScroller
                          ? rightSections
                          : unifiedSections;
                const element = elements.get(
                    sectionKey(anchor.file, anchor.section),
                );
                const source = anchor.sourceSelector
                    ? element?.querySelector<HTMLElement>(anchor.sourceSelector)
                    : undefined;
                if (element)
                    setScrollTop(
                        anchor.scroller,
                        anchor.scroller.scrollTop +
                            (source ?? element).getBoundingClientRect().top -
                            anchor.top,
                    );
            }
            expansionAnchor = [];
        });
    });
    const contextControls = (item: FlowDiffFile, section: FlowSection) => {
        const boundary = (up: boolean) => {
            const sameHunk = (
                layout() === "unified"
                    ? combineUnifiedSections(fileSections(item))
                    : fileSections(item)
            ).filter(
                (s) => s.hunkIndex === section.hunkIndex && s.kind !== "gap",
            );
            return (up ? sameHunk[0] : sameHunk.at(-1))?.id === section.id;
        };
        return (
            <For each={[true, false]}>
                {(up) => (
                    <Show
                        when={
                            props.onExpandContext &&
                            boundary(up) &&
                            (up
                                ? item.hunks[section.hunkIndex]?.context
                                      ?.canExpandUp
                                : item.hunks[section.hunkIndex]?.context
                                      ?.canExpandDown)
                        }
                    >
                        <button
                            type="button"
                            class={`flow-expand-context ${up ? "up" : "down"}`}
                            aria-label={`Show more context ${up ? "above" : "below"}`}
                            title={`Show 10 more lines ${up ? "above (K)" : "below (J)"}`}
                            onClick={(event) => {
                                event.stopPropagation();
                                expandContext(item, section, up);
                            }}
                        >
                            {up ? "↑" : "↓"}
                        </button>
                    </Show>
                )}
            </For>
        );
    };

    const sectionView = (
        section: FlowSection,
        side: Side,
        item: FlowDiffFile,
    ) => {
        const lines = side === "old" ? section.left : section.right;
        const other = side === "old" ? section.right : section.left;
        return (
            <div
                class={`flow-section ${section.kind}${section.move && side !== reconstruction() ? " flow-move-anchor" : ""}`}
                data-section={section.id}
                data-file-id={item.id}
                data-active={
                    item.id === file()?.id && section.id === activeSectionId()
                }
                ref={(element) =>
                    (side === "old" ? leftSections : rightSections).set(
                        sectionKey(item.id, section.id),
                        element,
                    )
                }
            >
                {contextControls(item, section)}
                <Show when={section.kind === "gap"}>
                    <div class="flow-gap">
                        <span>···</span>
                        {section.label}
                    </div>
                </Show>
                <For each={lines}>
                    {(line, index) => (
                        <FileLine
                            comments={comments}
                            file={item}
                            line={line}
                            counterpart={
                                section.move && side !== reconstruction()
                                    ? pairedMoveLine(section.move, line, side)
                                    : section.kind === "change"
                                      ? other[index()]
                                      : undefined
                            }
                            side={side}
                            syntax={props.syntax}
                            onNavigateReviewLine={props.onNavigateReviewLine}
                            onOpenSource={props.onOpenSource}
                        />
                    )}
                </For>
                <Show
                    when={
                        section.move &&
                        side === reconstruction() &&
                        props.review.custom
                    }
                >
                    <MoveOverlay
                        comments={comments}
                        hideEqual={hideEqual()}
                        move={section.move!}
                        reconstruction={reconstruction()}
                        file={item}
                        syntax={props.syntax}
                        onOpenSource={props.onOpenSource}
                    />
                </Show>
                <Show
                    when={
                        section.kind === "change" &&
                        lines.length === 0 &&
                        !(section.move && side === reconstruction())
                    }
                >
                    <div
                        class="flow-absence"
                        aria-label={`No ${side === "old" ? "removed" : "added"} lines in this section`}
                    >
                        <span>∅</span>
                    </div>
                </Show>
            </div>
        );
    };

    const unifiedLine = (
        line: FlowDiffLine,
        side: Side,
        item: FlowDiffFile,
        counterpart?: FlowDiffLine,
    ) => (
        <FileLine
            comments={comments}
            file={item}
            line={line}
            side={side}
            counterpart={line.kind === "context" ? undefined : counterpart}
            syntax={props.syntax}
            onNavigateReviewLine={props.onNavigateReviewLine}
            onOpenSource={props.onOpenSource}
        />
    );

    const fileHeading = (item: FlowDiffFile) => (
        <div class="flow-file-heading" data-file-id={item.id}>
            <div>
                <span class={`flow-status ${item.status}`}>{item.status}</span>
                <strong>
                    {item.label ? `${item.label} · ${item.path}` : item.path}
                </strong>
                <Show when={item.oldPath && item.oldPath !== item.path}>
                    <span class="flow-renamed">from {item.oldPath}</span>
                </Show>
            </div>
            <Show when={props.review.managed && props.onAction}>
                <label
                    class="flow-review-check"
                    title="Mark reviewed and hide (x)"
                >
                    <input
                        type="checkbox"
                        checked={item.checked ?? false}
                        aria-label={`Reviewed ${item.label || item.path}`}
                        onChange={() => checkItem(item)}
                    />
                    Reviewed
                </label>
            </Show>
            <span
                class="flow-file-stats"
                title="Original patch additions and deletions"
            >
                <b>+{item.additions}</b>
                <i>−{item.deletions}</i>
            </span>
        </div>
    );

    const fileMetadata = (item: FlowDiffFile) => (
        <Show when={item.metadata?.length}>
            <div class="flow-file-metadata" aria-label="File metadata">
                <For each={item.metadata}>{(line) => <code>{line}</code>}</For>
            </div>
        </Show>
    );

    const fileNote = (item: FlowDiffFile) => (
        <Show
            when={effectiveView() === "guided" && showNotes() && item.message}
        >
            <div class="flow-agent-note" aria-label="Agent note">
                <strong>Agent note</strong>
                <span>{item.message}</span>
            </div>
        </Show>
    );

    onMount(() => {
        if (typeof ResizeObserver !== "undefined")
            resizeObserver = new ResizeObserver(() => measure());
        queueMicrotask(measure);
    });
    onCleanup(() => resizeObserver?.disconnect());
    createEffect(() => {
        layout();
        allSections();
        queueMicrotask(() => {
            resizeObserver?.disconnect();
            if (bridge) resizeObserver?.observe(bridge);
            if (leftScroller) resizeObserver?.observe(leftScroller);
            if (rightScroller) resizeObserver?.observe(rightScroller);
            measure();
        });
    });

    createEffect(() => {
        const query = searchText().toLocaleLowerCase();
        allSections();
        layout();
        reconstruction();
        queueMicrotask(() => {
            const lines = [
                ...(panel?.querySelectorAll<HTMLElement>(".flow-code-line") ??
                    []),
            ];
            for (const line of lines)
                line.classList.remove(
                    "flow-search-match",
                    "flow-search-current",
                );
            searchMatches = query
                ? lines.filter((line) =>
                      line
                          .querySelector(".flow-line-text")
                          ?.textContent?.toLocaleLowerCase()
                          .includes(query),
                  )
                : [];
            for (const line of searchMatches)
                line.classList.add("flow-search-match");
            setSearchCount(searchMatches.length);
            selectSearchMatch(0);
        });
    });

    const openCurrentChange = () => {
        const currentFile = file();
        const activeSection = sections().find(
            (section) => section.id === activeSectionId(),
        );
        const lines = [
            ...(activeSection?.right ?? []),
            ...(activeSection?.left ?? []),
        ];
        const line =
            lines?.find((item) => item.kind === "added") ??
            lines?.find((item) => item.kind === "removed") ??
            lines?.[0];
        if (!currentFile || !line) return;
        const side = line.kind === "removed" ? "old" : "new";
        const number = side === "old" ? line.oldLine : line.newLine;
        if (number !== undefined)
            props.onOpenSource?.(
                side === "old"
                    ? currentFile.oldPath || currentFile.path
                    : currentFile.path,
                number,
                side,
            );
    };

    const [navigationMessage, setNavigationMessage] = createSignal("");
    const gotoDefinition = () => {
        const selection = window.getSelection();
        const node = selection?.focusNode;
        const element = node instanceof Element ? node : node?.parentElement;
        const code = element?.closest<HTMLElement>(".flow-line-text");
        const row = code?.closest<HTMLElement>(".flow-code-line");
        if (
            !node ||
            !code ||
            !row ||
            !panel?.contains(row) ||
            !props.onDefinition
        ) {
            setNavigationMessage(
                "Click a symbol in the diff, then press gd to go to its definition.",
            );
            return;
        }
        const range = document.createRange();
        range.selectNodeContents(code);
        range.setEnd(node, selection!.focusOffset);
        const column = range.toString().length;
        const path = row.dataset.sourcePath;
        const line = Number(row.dataset.sourceLine);
        const side = row.dataset.sourceSide as Side;
        if (path && line > 0 && (side === "old" || side === "new")) {
            setNavigationMessage("");
            props.onDefinition(path, line, side, column);
        }
    };

    const keydown: JSX.EventHandlerUnion<HTMLElement, KeyboardEvent> = (
        event,
    ) => {
        if (
            event.target instanceof HTMLInputElement ||
            event.target instanceof HTMLTextAreaElement ||
            event.isComposing
        )
            return;
        if (
            event.key === "/" ||
            ((event.metaKey || event.ctrlKey) &&
                event.key.toLowerCase() === "f")
        ) {
            event.preventDefault();
            openSearch();
            return;
        }
        if (event.key === "Escape" && searchOpen()) {
            event.preventDefault();
            setSearchOpen(false);
            panel?.focus();
            return;
        }
        if ((event.key === "n" || event.key === "N") && searchText()) {
            event.preventDefault();
            stepSearch(event.key === "N" ? -1 : 1);
            return;
        }
        if (event.target instanceof HTMLSelectElement) return;
        if (
            event.target instanceof HTMLButtonElement &&
            (event.key === "Enter" || event.key === " ")
        )
            return;
        if (event.metaKey || event.ctrlKey || event.altKey) return;
        if (event.key === "ArrowRight" || event.key === "ArrowLeft") {
            event.preventDefault();
            stepChange(event.key === "ArrowRight" ? 1 : -1);
            return;
        }
        if (event.key === "ArrowDown" || event.key === "ArrowUp") {
            event.preventDefault();
            const scroller =
                event.target instanceof Element
                    ? event.target.closest<HTMLElement>(
                          ".flow-scroll, .flow-unified-scroll",
                      )
                    : null;
            const target =
                scroller ??
                (layout() === "unified" ? unifiedScroller : rightScroller);
            target?.scrollBy?.({ top: event.key === "ArrowDown" ? 48 : -48 });
            return;
        }
        if ((event.key === "o" || event.key === "O") && props.onComment) {
            event.preventDefault();
            const row =
                panel?.querySelector<HTMLElement>(
                    "[data-active=true] [data-source-line]",
                ) || panel?.querySelector<HTMLElement>("[data-source-line]");
            const target =
                comments.selected() ||
                (row
                    ? {
                          path: row.dataset.sourcePath!,
                          line: Number(row.dataset.sourceLine),
                          side: row.dataset.sourceSide as Side,
                      }
                    : undefined);
            if (target) comments.begin(target, event.key === "O");
            return;
        }
        if (event.key === "g") {
            pendingGo = true;
            return;
        }
        if (pendingGo && event.key === "d") {
            pendingGo = false;
            pendingBracket = "";
            event.preventDefault();
            gotoDefinition();
            return;
        }
        const openSource =
            event.key === "Enter" || (pendingGo && event.key === "f");
        pendingGo = false;
        if (openSource) {
            event.preventDefault();
            pendingBracket = "";
            openCurrentChange();
            return;
        }
        if (event.key === "[" || event.key === "]") {
            pendingBracket = event.key;
            return;
        }
        if (pendingBracket && (event.key === "c" || event.key === "f")) {
            event.preventDefault();
            if (event.key === "c") stepChange(pendingBracket === "]" ? 1 : -1);
            else {
                const current = visibleFiles().findIndex(
                    (item) => item.id === file()?.id,
                );
                const next =
                    (current +
                        (pendingBracket === "]" ? 1 : -1) +
                        visibleFiles().length) %
                    visibleFiles().length;
                goToFile(visibleFiles()[next]?.id ?? "");
            }
            pendingBracket = "";
            return;
        }
        pendingBracket = "";
        if (event.key === "F7" || event.key === "n") {
            event.preventDefault();
            stepChange(1);
            return;
        }
        if (event.key === "N") {
            event.preventDefault();
            stepChange(-1);
            return;
        }
        if (event.key === "K" || event.key === "J") {
            event.preventDefault();
            const item = file();
            const section =
                sections().find((s) => s.id === activeSectionId()) ||
                sections().find((s) => s.kind === "change");
            if (item && section)
                expandContext(item, section, event.key === "K");
            return;
        }
        if (event.key === "x" && file() && props.onAction) {
            event.preventDefault();
            checkItem(file()!);
            return;
        }
        if (event.key === "X" && props.onAction) {
            event.preventDefault();
            props.onAction("show_checked");
            return;
        }
        if (event.key === "w") {
            event.preventDefault();
            setHideEqual((value) => !value);
            return;
        }
        if (event.key === "a" && hasNotes()) {
            event.preventDefault();
            setShowNotes((visible) => !visible);
            return;
        }
        if (event.key === "s") {
            event.preventDefault();
            const next = layout() === "split" ? "unified" : "split";
            setLayout(next);
            props.onLayoutChange?.(next);
            return;
        }
        if ((event.key === "q" || event.key === "r") && props.onAction) {
            event.preventDefault();
            props.onAction(event.key);
            return;
        }
        if (event.key === "j" || event.key === "k") {
            event.preventDefault();
            const step = event.key === "j" ? 22 : -22;
            const localScroller =
                event.target instanceof Element
                    ? event.target.closest<HTMLElement>(".flow-move-scroll")
                    : null;
            if (localScroller) localScroller.scrollTop += step;
            else if (layout() === "unified" && unifiedScroller)
                unifiedScroller.scrollTop += step;
            else if (leftScroller) {
                leftScroller.scrollTop += step;
                syncScroll("old");
            }
            return;
        }
        if ((event.key === ":" || event.key === " ") && props.onCoreKey) {
            event.preventDefault();
            props.onCoreKey(event.key);
        }
    };

    return (
        <section
            ref={panel}
            class="flow-diff"
            aria-label="Diff review"
            tabindex={0}
            data-gui-native-control
            onKeyDown={keydown}
        >
            <Show when={navigationMessage()}>
                <div role="status" class="flow-navigation-message">
                    {navigationMessage()}
                </div>
            </Show>
            <Show when={unplacedComments().length}>
                <details class="flow-unplaced-comments">
                    <summary>
                        {unplacedComments().length} comments on source that
                        changed or is hidden
                    </summary>
                    <For each={unplacedComments()}>
                        {(comment) => (
                            <article>
                                <strong>
                                    {comment.path}:{comment.line} ·{" "}
                                    {comment.side} side
                                </strong>
                                <code>{comment.excerpt}</code>
                                <p>{comment.text}</p>
                                <button
                                    type="button"
                                    onClick={() =>
                                        props.onAction?.(
                                            `remove_comment:${comment.id}`,
                                        )
                                    }
                                >
                                    Delete comment
                                </button>
                            </article>
                        )}
                    </For>
                </details>
            </Show>
            <header class="flow-toolbar">
                <div class="flow-toolbar-title" title={props.review.title}>
                    <strong>Review</strong>
                    <span>{props.review.title}</span>
                </div>
                <div class="flow-toolbar-actions">
                    <Show when={props.onComment}>
                        <span class="flow-comment-hint">o / O comment</span>
                    </Show>
                    <Show when={props.review.comments?.length}>
                        <button
                            type="button"
                            class="flow-clear-comments"
                            disabled={comments.saving()}
                            onClick={() => {
                                comments.cancel();
                                props.onAction?.("clear_comments");
                            }}
                        >
                            Clear comments ({props.review.comments!.length})
                        </button>
                    </Show>
                    <Show
                        when={
                            props.review.custom &&
                            props.review.guidedFiles?.length
                        }
                    >
                        <div
                            class="flow-view-switch"
                            role="group"
                            aria-label="Review view"
                        >
                            <button
                                type="button"
                                aria-pressed={effectiveView() === "files"}
                                onClick={() => setView("files")}
                            >
                                Files
                            </button>
                            <button
                                type="button"
                                aria-pressed={effectiveView() === "guided"}
                                title="Read agent-arranged sections and notes"
                                onClick={() => setView("guided")}
                            >
                                Guided
                            </button>
                        </div>
                    </Show>
                    <Show when={hasNotes()}>
                        <button
                            type="button"
                            class="flow-equal-toggle"
                            aria-pressed={showNotes()}
                            title="Show agent notes (a)"
                            onClick={() => setShowNotes((visible) => !visible)}
                        >
                            Notes
                        </button>
                    </Show>
                    <span class="flow-file-count">
                        {visibleFiles().length}{" "}
                        {effectiveView() === "guided"
                            ? visibleFiles().length === 1
                                ? "section"
                                : "sections"
                            : visibleFiles().length === 1
                              ? "file"
                              : "files"}
                    </span>
                    <span class="flow-navigation-hint">
                        ↑↓ scroll · ←→ changes
                    </span>
                    <div class="flow-hunk-nav" aria-label="Change navigation">
                        <button
                            type="button"
                            aria-label="Previous change"
                            title="Previous change (← or Shift+N)"
                            disabled={!changes().length}
                            onClick={() => stepChange(-1)}
                        >
                            ←
                        </button>
                        <span>
                            {changes().length ? activeChange() + 1 : 0} /{" "}
                            {changes().length}
                        </span>
                        <button
                            type="button"
                            aria-label="Next change"
                            title="Next change (→ or F7)"
                            disabled={!changes().length}
                            onClick={() => stepChange(1)}
                        >
                            →
                        </button>
                    </div>
                    <button
                        type="button"
                        class="flow-equal-toggle"
                        aria-pressed={hideEqual()}
                        title="Hide equal paired changes, ignoring spaces and tabs (w)"
                        onClick={() => setHideEqual((value) => !value)}
                    >
                        Hide equal changes
                    </button>
                    <div
                        class="flow-layout-switch"
                        role="group"
                        aria-label="Diff layout"
                    >
                        <button
                            type="button"
                            aria-pressed={layout() === "split"}
                            onClick={() => {
                                setLayout("split");
                                props.onLayoutChange?.("split");
                                queueMicrotask(() =>
                                    goToSection(
                                        activeSectionId(),
                                        false,
                                        file()?.id,
                                    ),
                                );
                            }}
                        >
                            Side by side
                        </button>
                        <button
                            type="button"
                            aria-pressed={layout() === "unified"}
                            onClick={() => {
                                setLayout("unified");
                                props.onLayoutChange?.("unified");
                                queueMicrotask(() =>
                                    goToSection(
                                        activeSectionId(),
                                        false,
                                        file()?.id,
                                    ),
                                );
                            }}
                        >
                            Unified
                        </button>
                    </div>
                    <button
                        type="button"
                        class="flow-equal-toggle"
                        disabled={exporting() || !props.review.files.length}
                        title="Download this view and layout as one dark PNG image."
                        onClick={() => void exportImages()}
                    >
                        {exporting() ? "Exporting…" : "Export image"}
                    </button>
                    <Show when={props.review.managed && props.onAction}>
                        <button
                            type="button"
                            class="flow-equal-toggle"
                            aria-pressed={props.review.showChecked ?? false}
                            onClick={() => props.onAction?.("show_checked")}
                            title="Show or hide reviewed items (X)"
                        >
                            {props.review.showChecked
                                ? "Hide reviewed"
                                : "Show reviewed"}{" "}
                            ({checkedCount()})
                        </button>
                    </Show>
                    <Show when={props.onAction}>
                        <div class="flow-action-buttons">
                            <button
                                type="button"
                                title={
                                    props.review.custom &&
                                    props.review.overlay?.mode !== "active"
                                        ? "Redraw saved review (R)"
                                        : "Refresh review (R)"
                                }
                                onClick={() => props.onAction?.("r")}
                            >
                                {props.review.custom &&
                                props.review.overlay?.mode !== "active"
                                    ? "Redraw"
                                    : "Refresh"}
                            </button>
                            <button
                                type="button"
                                title="Close review (Q)"
                                onClick={() => props.onAction?.("q")}
                            >
                                Close
                            </button>
                        </div>
                    </Show>
                </div>
            </header>
            <Show when={searchOpen()}>
                <div class="flow-search" role="search" aria-label="Search diff">
                    <input
                        ref={searchInput}
                        type="search"
                        aria-label="Find in diff"
                        placeholder="Find in diff"
                        value={searchText()}
                        onInput={(event) =>
                            setSearchText(event.currentTarget.value)
                        }
                        onKeyDown={(event) => {
                            event.stopPropagation();
                            if (event.key === "Enter") {
                                event.preventDefault();
                                stepSearch(event.shiftKey ? -1 : 1);
                            }
                            if (event.key === "Escape") {
                                event.preventDefault();
                                setSearchOpen(false);
                                panel?.focus();
                            }
                        }}
                    />
                    <span role="status">
                        {searchText()
                            ? `${searchIndex() + 1} / ${searchCount()} matching lines`
                            : "Type to search"}
                    </span>
                    <button
                        type="button"
                        aria-label="Previous search match"
                        disabled={!searchCount()}
                        onClick={() => stepSearch(-1)}
                    >
                        ↑
                    </button>
                    <button
                        type="button"
                        aria-label="Next search match"
                        disabled={!searchCount()}
                        onClick={() => stepSearch(1)}
                    >
                        ↓
                    </button>
                    <button
                        type="button"
                        aria-label="Close search"
                        onClick={() => {
                            setSearchOpen(false);
                            panel?.focus();
                        }}
                    >
                        ×
                    </button>
                </div>
            </Show>
            <Show when={exportMessage()}>
                <div
                    class="flow-export-status"
                    role={exportFailed() ? "alert" : "status"}
                >
                    {exportMessage()}
                </div>
            </Show>
            <Show when={props.review.overlay && props.onAction}>
                <div class="flow-overlay-status" role="status">
                    <span>
                        {props.review.overlay?.mode === "stale"
                            ? "Saved restructuring cannot be verified against this comparison."
                            : props.review.overlay?.mode === "saved"
                              ? "Viewing saved restructuring."
                              : props.review.overlay?.mode === "active"
                                ? "Restructuring overlay applied."
                                : "Saved restructuring available."}
                    </span>
                    <Show when={props.review.overlay?.title}>
                        <code title={props.review.overlay?.title}>
                            {props.review.overlay?.title}
                        </code>
                    </Show>
                    <button
                        type="button"
                        onClick={() =>
                            props.onAction?.(
                                props.review.overlay?.mode === "stale"
                                    ? "open_saved_overlay"
                                    : props.review.overlay?.mode === "saved"
                                      ? "return_to_live_diff"
                                      : "toggle_overlay",
                            )
                        }
                    >
                        {props.review.overlay?.mode === "stale"
                            ? "View saved review"
                            : props.review.overlay?.mode === "saved"
                              ? "Return to live diff"
                              : props.review.overlay?.mode === "active"
                                ? "Remove overlay"
                                : "Apply overlay"}
                    </button>
                </div>
            </Show>
            <Show
                when={
                    props.review.custom &&
                    effectiveView() === "files" &&
                    props.review.moves?.length
                }
            >
                <div class="flow-move-explanation">
                    <strong>Possible moved code</strong>
                    <span>
                        Similar lines were matched between before and after
                        locations. They may also have been edited.{" "}
                        {traceMoves()
                            ? reconstruction() === "old"
                                ? "Before context appears beside the added lines."
                                : "After context appears beside the removed lines."
                            : "Show moved-code matches to compare their locations."}
                    </span>
                    <div class="flow-move-controls">
                        <div
                            class="flow-view-switch"
                            role="group"
                            aria-label="Moved code view"
                        >
                            <button
                                type="button"
                                aria-pressed={!traceMoves()}
                                onClick={() => setTraceMoves(false)}
                            >
                                File changes
                            </button>
                            <button
                                type="button"
                                aria-pressed={traceMoves()}
                                onClick={() => setTraceMoves(true)}
                            >
                                Show moved-code matches
                            </button>
                        </div>
                        <Show when={traceMoves()}>
                            <div
                                class="flow-reconstruction-switch"
                                role="group"
                                aria-label="Moved code context"
                            >
                                <button
                                    type="button"
                                    aria-pressed={reconstruction() === "old"}
                                    onClick={() => changeReconstruction("old")}
                                >
                                    Before context
                                </button>
                                <button
                                    type="button"
                                    aria-pressed={reconstruction() === "new"}
                                    onClick={() => changeReconstruction("new")}
                                >
                                    After context
                                </button>
                            </div>
                        </Show>
                    </div>
                </div>
            </Show>
            <Show
                when={visibleFiles().length}
                fallback={
                    <div class="flow-empty">
                        {checkedCount() && !props.review.showChecked
                            ? "All visible changes reviewed. Show reviewed to bring them back."
                            : hideEqual() && sourceFiles().length
                              ? "No unequal changes"
                              : "No changed files in this comparison."}
                    </div>
                }
            >
                <Show
                    when={layout() === "split"}
                    fallback={
                        <div
                            class="flow-unified-scroll"
                            ref={unifiedScroller}
                            onScroll={updateUnifiedActive}
                            role="region"
                            aria-label="Unified changes"
                            tabindex={0}
                        >
                            <For each={visibleFiles()}>
                                {(item) => (
                                    <div
                                        class="flow-file-group"
                                        data-file-id={item.id}
                                    >
                                        {fileHeading(item)}
                                        {fileNote(item)}
                                        {fileMetadata(item)}
                                        <Show
                                            when={!item.binary}
                                            fallback={
                                                <div class="flow-empty compact">
                                                    Binary file — no text diff
                                                    to display.
                                                </div>
                                            }
                                        >
                                            <Show
                                                when={item.hunks.length}
                                                fallback={
                                                    <div class="flow-empty compact">
                                                        No text changes in this
                                                        {effectiveView() ===
                                                        "guided"
                                                            ? " section."
                                                            : " file."}
                                                    </div>
                                                }
                                            >
                                                <For
                                                    each={combineUnifiedSections(
                                                        fileSections(item),
                                                    )}
                                                >
                                                    {(section) => (
                                                        <div
                                                            class={`flow-unified-section ${section.kind}`}
                                                            data-file-id={
                                                                item.id
                                                            }
                                                            data-section={
                                                                section.id
                                                            }
                                                            data-active={
                                                                item.id ===
                                                                    file()
                                                                        ?.id &&
                                                                section.id ===
                                                                    activeSectionId()
                                                            }
                                                            ref={(element) =>
                                                                unifiedSections.set(
                                                                    sectionKey(
                                                                        item.id,
                                                                        section.id,
                                                                    ),
                                                                    element,
                                                                )
                                                            }
                                                        >
                                                            {contextControls(
                                                                item,
                                                                section,
                                                            )}
                                                            <Show
                                                                when={
                                                                    section.kind ===
                                                                    "gap"
                                                                }
                                                            >
                                                                <div class="flow-gap">
                                                                    <span>
                                                                        ···
                                                                    </span>
                                                                    {
                                                                        section.label
                                                                    }
                                                                </div>
                                                            </Show>
                                                            <Show
                                                                when={
                                                                    section.kind ===
                                                                    "change"
                                                                }
                                                            >
                                                                <div class="flow-unified-heading">
                                                                    Change{" "}
                                                                    {changes().findIndex(
                                                                        (
                                                                            change,
                                                                        ) =>
                                                                            change.fileId ===
                                                                                item.id &&
                                                                            change.sectionId ===
                                                                                section.id,
                                                                    ) + 1}
                                                                </div>
                                                            </Show>
                                                            <For
                                                                each={unifiedLeftLines(
                                                                    item,
                                                                    section,
                                                                )}
                                                            >
                                                                {(
                                                                    line,
                                                                    index,
                                                                ) =>
                                                                    unifiedLine(
                                                                        line,
                                                                        "old",
                                                                        item,
                                                                        section.move &&
                                                                            reconstruction() ===
                                                                                "new"
                                                                            ? pairedMoveLine(
                                                                                  section.move,
                                                                                  line,
                                                                                  "old",
                                                                              )
                                                                            : section
                                                                                  .right[
                                                                                  index()
                                                                              ],
                                                                    )
                                                                }
                                                            </For>
                                                            <For
                                                                each={
                                                                    section.right
                                                                }
                                                            >
                                                                {(
                                                                    line,
                                                                    index,
                                                                ) =>
                                                                    unifiedLine(
                                                                        line,
                                                                        "new",
                                                                        item,
                                                                        section.move &&
                                                                            reconstruction() ===
                                                                                "old"
                                                                            ? pairedMoveLine(
                                                                                  section.move,
                                                                                  line,
                                                                                  "new",
                                                                              )
                                                                            : section
                                                                                  .left[
                                                                                  index()
                                                                              ],
                                                                    )
                                                                }
                                                            </For>
                                                            <Show
                                                                when={
                                                                    section.move &&
                                                                    effectiveView() ===
                                                                        "files"
                                                                }
                                                            >
                                                                <MoveOverlay
                                                                    comments={
                                                                        comments
                                                                    }
                                                                    hideEqual={hideEqual()}
                                                                    move={
                                                                        section.move!
                                                                    }
                                                                    reconstruction={reconstruction()}
                                                                    file={item}
                                                                    syntax={
                                                                        props.syntax
                                                                    }
                                                                    onOpenSource={
                                                                        props.onOpenSource
                                                                    }
                                                                />
                                                            </Show>
                                                        </div>
                                                    )}
                                                </For>
                                            </Show>
                                        </Show>
                                    </div>
                                )}
                            </For>
                        </div>
                    }
                >
                    <div class="flow-columns">
                        <div class="flow-side-heading">
                            <span>Before</span>
                        </div>
                        <div class="flow-bridge-heading" aria-hidden="true">
                            ↝
                        </div>
                        <div class="flow-side-heading">
                            <span>After</span>
                        </div>
                        <div
                            class="flow-scroll old"
                            ref={leftScroller}
                            onScroll={() => syncScroll("old")}
                            role="region"
                            aria-label="Before changes"
                            tabindex={0}
                        >
                            <For each={visibleFiles()}>
                                {(item) => (
                                    <div
                                        class="flow-file-group"
                                        data-file-id={item.id}
                                    >
                                        {fileHeading(item)}
                                        {fileNote(item)}
                                        {fileMetadata(item)}
                                        <Show
                                            when={!item.binary}
                                            fallback={
                                                <div class="flow-empty compact">
                                                    Binary file — no text diff
                                                    to display.
                                                </div>
                                            }
                                        >
                                            <Show
                                                when={item.hunks.length}
                                                fallback={
                                                    <div class="flow-empty compact">
                                                        No text changes in this
                                                        {effectiveView() ===
                                                        "guided"
                                                            ? " section."
                                                            : " file."}
                                                    </div>
                                                }
                                            >
                                                <For each={fileSections(item)}>
                                                    {(section) =>
                                                        sectionView(
                                                            section,
                                                            "old",
                                                            item,
                                                        )
                                                    }
                                                </For>
                                            </Show>
                                        </Show>
                                    </div>
                                )}
                            </For>
                        </div>
                        <div
                            class="flow-bridge"
                            ref={bridge}
                            aria-hidden="true"
                        >
                            <svg
                                width="42"
                                height="100%"
                                preserveAspectRatio="none"
                            >
                                <For each={ribbons()}>
                                    {(ribbon) => (
                                        <path
                                            class={`flow-ribbon ${ribbon.kind}`}
                                            d={ribbon.path}
                                            onClick={() => {
                                                const [fileId, sectionId] =
                                                    ribbon.id.split("\0");
                                                goToSection(
                                                    sectionId,
                                                    true,
                                                    fileId,
                                                );
                                            }}
                                        />
                                    )}
                                </For>
                            </svg>
                        </div>
                        <div
                            class="flow-scroll new"
                            ref={rightScroller}
                            onScroll={() => syncScroll("new")}
                            role="region"
                            aria-label="After changes"
                            tabindex={0}
                        >
                            <For each={visibleFiles()}>
                                {(item) => (
                                    <div
                                        class="flow-file-group"
                                        data-file-id={item.id}
                                    >
                                        {fileHeading(item)}
                                        {fileNote(item)}
                                        {fileMetadata(item)}
                                        <Show
                                            when={!item.binary}
                                            fallback={
                                                <div class="flow-empty compact">
                                                    Binary file — no text diff
                                                    to display.
                                                </div>
                                            }
                                        >
                                            <Show
                                                when={item.hunks.length}
                                                fallback={
                                                    <div class="flow-empty compact">
                                                        No text changes in this
                                                        {effectiveView() ===
                                                        "guided"
                                                            ? " section."
                                                            : " file."}
                                                    </div>
                                                }
                                            >
                                                <For each={fileSections(item)}>
                                                    {(section) =>
                                                        sectionView(
                                                            section,
                                                            "new",
                                                            item,
                                                        )
                                                    }
                                                </For>
                                            </Show>
                                        </Show>
                                    </div>
                                )}
                            </For>
                        </div>
                    </div>
                </Show>
            </Show>
        </section>
    );
}
