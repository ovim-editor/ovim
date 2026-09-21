import {
    For,
    Show,
    createEffect,
    createMemo,
    createSignal,
    onCleanup,
    onMount,
} from "solid-js";
import { Icon } from "./Icon";
import ResizeDivider from "./ResizeDivider";
import type { GuiSnapshot } from "./types";
import {
    EXPLORER_DEFAULT_WIDTH,
    EXPLORER_MIN_WIDTH,
    explorerWidthLimit,
    middleEllipsis,
} from "./explorerLayout";

type FileTree = NonNullable<GuiSnapshot["fileTree"]>;

export default function FileExplorer(props: {
    tree: FileTree;
    width: number;
    reservedWidth?: number;
    active: boolean;
    onWidthChange: (width: number) => void;
    onSelect: (index: number, activate: boolean) => void;
}) {
    let list!: HTMLDivElement;
    let frame = 0;
    let automaticScrollLeft = 0;
    const [windowWidth, setWindowWidth] = createSignal(window.innerWidth);
    const limit = createMemo(() =>
        explorerWidthLimit(windowWidth(), props.reservedWidth),
    );
    const [summary, setSummary] = createSignal<{
        path: string;
        text: string;
    }>();
    const width = () =>
        Math.max(EXPLORER_MIN_WIDTH, Math.min(props.width, limit()));
    // Only this viewport scrolls. Scrolling the row itself could also move the workbench.
    const ensureSelectedVisible = (center: boolean) => {
        if (!list.clientWidth || !list.clientHeight) return;
        const row = list.querySelector<HTMLElement>('[aria-selected="true"]');
        const label = row?.querySelector<HTMLElement>(".tree-name-full");
        if (!row || !label) {
            setSummary(undefined);
            return;
        }
        const viewport = list.getBoundingClientRect();
        const bounds = row.getBoundingClientRect();
        const bottom = viewport.top + list.clientHeight;
        if (bounds.top < viewport.top || bounds.bottom > bottom) {
            list.scrollTop += center
                ? bounds.top -
                  viewport.top -
                  (list.clientHeight - bounds.height) / 2
                : bounds.top < viewport.top
                  ? bounds.top - viewport.top
                  : bounds.bottom - bottom;
        }
        const name = label.getBoundingClientRect();
        const available = list.clientWidth - 16;
        const left = name.left - viewport.left + list.scrollLeft;
        const right = left + name.width;
        let nextLeft = list.scrollLeft;
        if (name.width > available || left < nextLeft + 8) nextLeft = left - 8;
        else if (right > nextLeft + list.clientWidth - 8)
            nextLeft = right - list.clientWidth + 8;
        list.scrollLeft = Math.max(0, nextLeft);
        automaticScrollLeft = list.scrollLeft;
        const selected = props.tree.items.find(
            (item) => item.index === props.tree.selected,
        );
        if (selected && name.width > available) {
            const context = document.createElement("canvas").getContext("2d");
            if (context) {
                const font = getComputedStyle(label);
                // WebKit can return an empty shorthand when shaping properties are set.
                context.font = `${font.fontStyle} ${font.fontWeight} ${font.fontSize} ${font.fontFamily}`;
                setSummary({
                    path: selected.path,
                    text: middleEllipsis(
                        selected.name,
                        available,
                        (text) => context.measureText(text).width,
                    ),
                });
                return;
            }
        }
        setSummary(undefined);
    };
    let pendingCenter = false;
    const scheduleVisibility = (center = false) => {
        pendingCenter ||= center;
        cancelAnimationFrame(frame);
        frame = requestAnimationFrame(() => {
            const center = pendingCenter;
            pendingCenter = false;
            ensureSelectedVisible(center);
        });
    };

    const revealGeneration = createMemo(() => props.tree.revealGeneration);
    const selectedIdentity = createMemo(() => {
        const item = props.tree.items.find(
            (item) => item.index === props.tree.selected,
        );
        return item ? `${item.index}:${item.depth}:${item.path}` : undefined;
    });
    createEffect(() => {
        void revealGeneration();
        scheduleVisibility(true);
    });
    createEffect(() => {
        // Path identity survives expansion and snapshot replacement; no dependency on
        // unrelated editor revisions, so manual scrolling remains undisturbed.
        void selectedIdentity();
        void props.active;
        scheduleVisibility();
    });
    onMount(() => {
        const observer = new ResizeObserver(() => scheduleVisibility());
        observer.observe(list);
        const windowResize = () => setWindowWidth(window.innerWidth);
        window.addEventListener("resize", windowResize);
        const fontsChanged = () => scheduleVisibility();
        document.fonts?.addEventListener("loadingdone", fontsChanged);
        onCleanup(() => {
            observer.disconnect();
            window.removeEventListener("resize", windowResize);
            document.fonts?.removeEventListener("loadingdone", fontsChanged);
        });
    });
    onCleanup(() => cancelAnimationFrame(frame));

    return (
        <aside
            class="explorer"
            style={{ "--explorer-width": `${width()}px` }}
            aria-label="Explorer"
        >
            <div class="panel-heading">
                <span>Explorer</span>
                <small title={props.tree.root}>{props.tree.root}</small>
            </div>
            <div
                ref={list}
                class="tree-list"
                role="tree"
                aria-label="Project files"
                onScroll={() => {
                    if (Math.abs(list.scrollLeft - automaticScrollLeft) > 1)
                        setSummary(undefined);
                }}
            >
                <div class="tree-content">
                    <For
                        each={props.tree.items}
                        fallback={
                            <p class="panel-empty compact">
                                This workspace is empty
                            </p>
                        }
                    >
                        {(item) => (
                            <button
                                type="button"
                                role="treeitem"
                                class="tree-item"
                                aria-label={item.name}
                                aria-selected={
                                    item.index === props.tree.selected
                                }
                                aria-expanded={
                                    item.directory ? item.expanded : undefined
                                }
                                aria-level={item.depth + 1}
                                classList={{
                                    selected:
                                        item.index === props.tree.selected,
                                }}
                                style={{
                                    "padding-left": `${10 + item.depth * 14}px`,
                                }}
                                title={item.path}
                                onClick={() =>
                                    props.onSelect(item.index, false)
                                }
                                onDblClick={() =>
                                    props.onSelect(item.index, true)
                                }
                                onKeyDown={(event) => {
                                    if (event.key !== "Enter") return;
                                    event.preventDefault();
                                    props.onSelect(item.index, true);
                                }}
                            >
                                <span
                                    class={`tree-chevron ${item.directory ? "directory" : "file"}`}
                                >
                                    <Show when={item.directory}>
                                        <Icon
                                            name={
                                                item.expanded
                                                    ? "chevron-down"
                                                    : "chevron-right"
                                            }
                                            size={16}
                                        />
                                    </Show>
                                </span>
                                <Icon
                                    name={item.directory ? "folder" : "file"}
                                    size={16}
                                    tone={item.directory ? "warning" : "muted"}
                                />
                                <span
                                    class="tree-name"
                                    classList={{
                                        summarized:
                                            summary()?.path === item.path,
                                    }}
                                >
                                    <span class="tree-name-full">
                                        {item.name}
                                    </span>
                                    <Show when={summary()?.path === item.path}>
                                        <span
                                            class="tree-name-summary"
                                            aria-hidden="true"
                                        >
                                            {summary()?.text}
                                        </span>
                                    </Show>
                                </span>
                            </button>
                        )}
                    </For>
                </div>
            </div>
            <ResizeDivider
                label="Explorer width"
                value={width()}
                minimum={EXPLORER_MIN_WIDTH}
                maximum={limit()}
                defaultValue={EXPLORER_DEFAULT_WIDTH}
                onChange={props.onWidthChange}
            />
        </aside>
    );
}
