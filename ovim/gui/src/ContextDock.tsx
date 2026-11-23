import {
    Index,
    onCleanup,
    Show,
    createEffect,
    createMemo,
    createSignal,
    type Component,
} from "solid-js";
import { Dynamic } from "solid-js/web";
import ResizeDivider from "./ResizeDivider";
import {
    CONTEXT_DEFAULT_WIDTH,
    CONTEXT_MIN_WIDTH,
    CONTEXT_MAX_WIDTH,
} from "./contextDockLayout";
import { Icon } from "./Icon";
import type { IconName } from "./icons.generated";

export type ContextPanelId = "ai" | "tests" | "debug" | "run" | "terminal";

export interface ContextPanelDefinition {
    id: ContextPanelId;
    label: string;
    state: string;
    icon: IconName;
    component: Component;
    keepMounted?: boolean;
}

export default function ContextDock(props: {
    panels: ContextPanelDefinition[];
    width?: number;
    onWidthChange?: (width: number) => void;
    activePanel?: ContextPanelId;
    onActivePanel?: (id: ContextPanelId) => void;
}) {
    const [dock, setDock] = createSignal<HTMLElement>();
    const [localWidth, setLocalWidth] = createSignal(CONTEXT_DEFAULT_WIDTH);
    const [maximumWidth, setMaximumWidth] = createSignal(CONTEXT_MAX_WIDTH);
    const width = () => Math.min(props.width ?? localWidth(), maximumWidth());
    createEffect(() => {
        const element = dock();
        if (!element) return;
        const measure = () => {
            const available =
                element.parentElement?.clientWidth ?? window.innerWidth;
            const editorBudget = window.innerWidth >= 1440 ? 360 : 0;
            setMaximumWidth(
                Math.max(
                    CONTEXT_MIN_WIDTH,
                    Math.min(CONTEXT_MAX_WIDTH, available - editorBudget),
                ),
            );
        };
        const observer = new ResizeObserver(measure);
        if (element.parentElement) observer.observe(element.parentElement);
        window.addEventListener("resize", measure);
        measure();
        onCleanup(() => {
            observer.disconnect();
            window.removeEventListener("resize", measure);
        });
    });
    const [activeId, setActiveId] = createSignal<ContextPanelId>(
        props.panels[0]?.id ?? "ai",
    );
    const activePanel = createMemo(
        () =>
            props.panels.find((panel) => panel.id === activeId()) ??
            props.panels[0],
    );

    createEffect(() => {
        const requested = props.activePanel;
        if (
            requested &&
            requested !== activeId() &&
            props.panels.some((panel) => panel.id === requested)
        ) {
            setActiveId(requested);
            return;
        }
        const panel = activePanel();
        if (!panel) return;
        if (panel.id !== activeId()) setActiveId(panel.id);
        props.onActivePanel?.(panel.id);
    });

    const selectPanel = (id: ContextPanelId) => {
        setActiveId(id);
        props.onActivePanel?.(id);
    };

    const moveFocus = (event: KeyboardEvent, panel: ContextPanelDefinition) => {
        const panels = props.panels;
        const current = panels.findIndex(
            (candidate) => candidate.id === panel.id,
        );
        let next = current;
        if (event.key === "ArrowRight") next = (current + 1) % panels.length;
        else if (event.key === "ArrowLeft")
            next = (current - 1 + panels.length) % panels.length;
        else if (event.key === "Home") next = 0;
        else if (event.key === "End") next = panels.length - 1;
        else return;

        event.preventDefault();
        const id = panels[next].id;
        selectPanel(id);
        queueMicrotask(() =>
            document.getElementById(`context-tab-${id}`)?.focus(),
        );
    };

    return (
        <Show when={activePanel()?.id}>
            <aside
                ref={setDock}
                class="side-dock"
                style={{ "--context-width": `${width()}px` }}
                classList={{
                    "has-context-tabs": props.panels.length > 1,
                }}
                aria-label="Context"
            >
                <ResizeDivider
                    label="Context panel width"
                    value={width()}
                    minimum={CONTEXT_MIN_WIDTH}
                    maximum={maximumWidth()}
                    defaultValue={CONTEXT_DEFAULT_WIDTH}
                    reverse
                    onChange={(value) => {
                        setLocalWidth(value);
                        props.onWidthChange?.(value);
                    }}
                />
                <Show when={props.panels.length > 1}>
                    <div
                        class="context-tabs"
                        role="tablist"
                        aria-label="Context panels"
                    >
                        <Index each={props.panels}>
                            {(panel) => (
                                <button
                                    id={`context-tab-${panel().id}`}
                                    type="button"
                                    role="tab"
                                    aria-label={panel().label}
                                    aria-selected={
                                        activePanel()!.id === panel().id
                                    }
                                    aria-controls={`context-panel-${panel().id}`}
                                    tabIndex={
                                        activePanel()!.id === panel().id
                                            ? 0
                                            : -1
                                    }
                                    onClick={() => selectPanel(panel().id)}
                                    onKeyDown={(event) =>
                                        moveFocus(event, panel())
                                    }
                                >
                                    <Icon name={panel().icon} size={16} />
                                    <span>{panel().label}</span>
                                    <small>{panel().state}</small>
                                </button>
                            )}
                        </Index>
                    </div>
                </Show>
                <Index each={props.panels}>
                    {(panel) => (
                        <Show
                            when={
                                panel().keepMounted ||
                                activePanel()!.id === panel().id
                            }
                        >
                            <div
                                id={`context-panel-${panel().id}`}
                                class="context-panel"
                                role="tabpanel"
                                hidden={activePanel()!.id !== panel().id}
                                aria-label={
                                    props.panels.length === 1
                                        ? panel().label
                                        : undefined
                                }
                                aria-labelledby={
                                    props.panels.length > 1
                                        ? `context-tab-${panel().id}`
                                        : undefined
                                }
                            >
                                <Dynamic component={panel().component} />
                            </div>
                        </Show>
                    )}
                </Index>
            </aside>
        </Show>
    );
}
