/** @vitest-environment jsdom */

import { cleanup, fireEvent, render, screen } from "@solidjs/testing-library";
import { createSignal, onMount } from "solid-js";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import ContextDock, { type ContextPanelDefinition } from "./ContextDock";

beforeEach(() => {
    vi.stubGlobal(
        "ResizeObserver",
        class {
            observe() {}
            disconnect() {}
        },
    );
    vi.spyOn(HTMLElement.prototype, "clientWidth", "get").mockReturnValue(1200);
});
afterEach(() => {
    cleanup();
    vi.restoreAllMocks();
    vi.unstubAllGlobals();
});

const panel = (
    id: ContextPanelDefinition["id"],
    label: string,
): ContextPanelDefinition => ({
    id,
    label,
    state: "ready",
    icon: id === "ai" ? "ai-spark" : id === "tests" ? "test" : "debug",
    component: () => <p>{label} content</p>,
});

describe("ContextDock", () => {
    it("mounts one context surface and switches it through accessible tabs", () => {
        const result = render(() => (
            <ContextDock
                panels={[
                    panel("ai", "AI chat"),
                    panel("tests", "Tests"),
                    panel("debug", "Debug"),
                ]}
            />
        ));

        expect(
            result.container
                .querySelector(".side-dock")
                ?.classList.contains("has-context-tabs"),
        ).toBe(true);
        expect(screen.getByRole("tabpanel").textContent).toContain(
            "AI chat content",
        );
        expect(screen.queryByText("Tests content")).toBeNull();

        fireEvent.click(screen.getByRole("tab", { name: "Tests" }));
        expect(screen.getByRole("tabpanel").textContent).toContain(
            "Tests content",
        );
        expect(screen.queryByText("AI chat content")).toBeNull();
    });

    it("moves tab focus with arrow keys and recovers when a panel closes", async () => {
        const [panels, setPanels] = createSignal([
            panel("ai", "AI chat"),
            panel("tests", "Tests"),
        ]);
        const result = render(() => <ContextDock panels={panels()} />);

        const ai = screen.getByRole("tab", { name: "AI chat" });
        fireEvent.keyDown(ai, { key: "ArrowRight" });
        await Promise.resolve();
        expect(document.activeElement).toBe(
            screen.getByRole("tab", { name: "Tests" }),
        );
        expect(screen.getByRole("tabpanel").textContent).toContain(
            "Tests content",
        );

        setPanels([panel("ai", "AI chat")]);
        expect(
            result.container
                .querySelector(".side-dock")
                ?.classList.contains("has-context-tabs"),
        ).toBe(false);
        expect(
            screen.getByRole("tabpanel", { name: "AI chat" }).textContent,
        ).toContain("AI chat content");
    });

    it("honors a controlled active panel without undoing user selection", () => {
        const [active, setActive] =
            createSignal<ContextPanelDefinition["id"]>("ai");
        render(() => (
            <ContextDock
                panels={[panel("ai", "AI chat"), panel("tests", "Tests")]}
                activePanel={active()}
                onActivePanel={setActive}
            />
        ));

        fireEvent.click(screen.getByRole("tab", { name: "Tests" }));
        expect(active()).toBe("tests");
        expect(screen.getByRole("tabpanel").textContent).toContain(
            "Tests content",
        );
    });

    it("preserves the active surface when snapshot metadata changes", () => {
        let mounts = 0;
        const StablePanel = () => {
            onMount(() => mounts++);
            return <input aria-label="Persistent input" />;
        };
        const [panels, setPanels] = createSignal<ContextPanelDefinition[]>([
            {
                ...panel("ai", "AI chat"),
                state: "idle",
                component: StablePanel,
            },
            panel("tests", "Tests"),
        ]);
        render(() => <ContextDock panels={panels()} />);
        const input = screen.getByRole("textbox", {
            name: "Persistent input",
        });
        const tab = screen.getByRole("tab", { name: "AI chat" });

        setPanels([
            {
                ...panel("ai", "AI chat"),
                state: "streaming",
                component: StablePanel,
            },
            panel("tests", "Tests"),
        ]);

        expect(screen.getByRole("textbox", { name: "Persistent input" })).toBe(
            input,
        );
        expect(screen.getByRole("tab", { name: "AI chat" })).toBe(tab);
        expect(mounts).toBe(1);
    });

    it("keeps a terminal surface mounted while another context panel is active", () => {
        let mounts = 0;
        const Terminal = () => {
            onMount(() => mounts++);
            return <input aria-label="Shell input" />;
        };
        render(() => (
            <ContextDock
                panels={[
                    {
                        ...panel("terminal", "Terminal"),
                        component: Terminal,
                        keepMounted: true,
                    },
                    panel("ai", "AI chat"),
                ]}
            />
        ));
        const input = screen.getByRole("textbox", { name: "Shell input" });

        fireEvent.click(screen.getByRole("tab", { name: "AI chat" }));
        expect(input.isConnected).toBe(true);
        expect(input.closest("[hidden]")).toBeTruthy();
        fireEvent.click(screen.getByRole("tab", { name: "Terminal" }));
        expect(screen.getByRole("textbox", { name: "Shell input" })).toBe(
            input,
        );
        expect(mounts).toBe(1);
    });
});

it("resizes from the left edge with keyboard controls and resets", () => {
    const [width, setWidth] = createSignal(480);
    render(() => (
        <ContextDock
            panels={[panel("tests", "Tests")]}
            width={width()}
            onWidthChange={setWidth}
        />
    ));
    const divider = screen.getByRole("separator", {
        name: "Context panel width",
    });
    fireEvent.keyDown(divider, { key: "ArrowLeft" });
    expect(width()).toBe(490);
    fireEvent.keyDown(divider, { key: "ArrowRight", shiftKey: true });
    expect(width()).toBe(450);
    fireEvent.keyDown(divider, { key: "Home" });
    expect(width()).toBe(280);
    fireEvent.keyDown(divider, { key: "End" });
    expect(width()).toBe(800);
    fireEvent.dblClick(divider);
    expect(width()).toBe(480);
});
