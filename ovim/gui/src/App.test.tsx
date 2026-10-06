/** @vitest-environment jsdom */

import { fireEvent, render, screen, waitFor } from "@solidjs/testing-library";
import { createSignal } from "solid-js";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import App, {
    ChatActivityGroup,
    ChatComposer,
    ChatMessageView,
    ChatPanel,
    ChatSetupCard,
    CodeWalkthrough,
    Markdown,
    activitySummary,
    chatSelectionText,
    chatTranscriptItems,
    guiKeyInput,
    imageExtension,
    isNearChatBottom,
    retainTranscriptItems,
    reconcileWorkbenchTabs,
    requestedBrowserPresentation,
    toolResultSummary,
} from "./App";
import { mockSnapshot } from "./mock";
import type { GuiAiChat, GuiCodeExplanation } from "./types";

it("Claude chat presents permission choices and hides Ovim agent policies", () => {
    const onApproval = vi.fn();
    const chat: GuiAiChat = {
        ...mockSnapshot.aiChat!,
        profile: "claude_code",
        externalAgent: true,
        profiles: [
            { id: "claude_code", provider: "claude_code", model: "default" },
        ],
        approval: "Claude Code: Bash\nRun the project tests?",
    };
    render(() => (
        <ChatPanel chat={chat} focusInput={() => {}} onApproval={onApproval} />
    ));
    expect(screen.queryByRole("button", { name: /YOLO/ })).toBeNull();
    expect(screen.queryByRole("button", { name: /COMPREHENSION/ })).toBeNull();
    expect(screen.getByText(/Run the project tests/)).toBeTruthy();
    fireEvent.click(screen.getByRole("button", { name: "Allow once" }));
    fireEvent.click(screen.getByRole("button", { name: "Deny" }));
    expect(onApproval.mock.calls).toEqual([[true], [false]]);
});

it("Claude questions keep an answer action in the composer while the agent waits", () => {
    const chat: GuiAiChat = {
        ...mockSnapshot.aiChat!,
        externalAgent: true,
        externalQuestion: true,
        waiting: true,
        activity: "waiting_tool_approval",
        input: "Blue",
        inputCursor: 4,
    };
    render(() => <ChatComposer chat={chat} />);
    expect(
        screen.getByPlaceholderText("Answer Claude’s question…"),
    ).toBeTruthy();
    expect(screen.getByRole("button", { name: "Send message" })).toBeTruthy();
    expect(
        screen.queryByRole("button", { name: "Stop generation" }),
    ).toBeNull();
});

class ResizeObserverMock {
    observe() {}
    unobserve() {}
    disconnect() {}
}

beforeEach(() => {
    const layoutStorage = new Map<string, string>();
    vi.stubGlobal("localStorage", {
        getItem: (key: string) => layoutStorage.get(key) ?? null,
        setItem: (key: string, value: string) => layoutStorage.set(key, value),
        removeItem: (key: string) => layoutStorage.delete(key),
        clear: () => layoutStorage.clear(),
        key: (index: number) => [...layoutStorage.keys()][index] ?? null,
        get length() {
            return layoutStorage.size;
        },
    });
    vi.stubGlobal("ResizeObserver", ResizeObserverMock);
    vi.stubGlobal("requestAnimationFrame", (callback: FrameRequestCallback) => {
        callback(0);
        return 1;
    });
    vi.spyOn(HTMLCanvasElement.prototype, "getContext").mockReturnValue({
        font: "",
        measureText: () => ({ width: 8 }),
    } as unknown as CanvasRenderingContext2D);
    vi.spyOn(HTMLElement.prototype, "focus").mockImplementation(() => {});
});

afterEach(() => {
    vi.restoreAllMocks();
    vi.unstubAllGlobals();
    document.body.replaceChildren();
});

describe("Ovim Solid workbench", () => {
    it("composes browser sessions into one navigable workbench tab model", () => {
        const browserSessions = [
            {
                sessionId: "browser-1",
                url: "https://nrk.no/",
                title: "NRK",
                visible: false,
                loading: false,
                documentId: 1,
                vimKeysEnabled: true,
                keyMode: "normal" as const,
            },
            {
                sessionId: "browser-2",
                url: "https://example.com/",
                title: "Example",
                visible: false,
                loading: false,
                documentId: 1,
                vimKeysEnabled: true,
                keyMode: "normal" as const,
            },
        ];

        expect(
            reconcileWorkbenchTabs(
                [],
                mockSnapshot.tabs,
                true,
                browserSessions,
                { kind: "source", tabId: 1 },
            ),
        ).toEqual([
            { id: "source:1", kind: "source", index: 0, tabId: 1 },
            {
                id: "browser:browser-1",
                kind: "browser",
                sessionId: "browser-1",
            },
            {
                id: "browser:browser-2",
                kind: "browser",
                sessionId: "browser-2",
            },
            { id: "vector:1", kind: "vector", sourceTabId: 1 },
            { id: "source:2", kind: "source", index: 1, tabId: 2 },
        ]);
    });

    it("replays a durable agent browser presentation request exactly once", () => {
        const state = {
            revision: 1,
            sessions: [
                {
                    sessionId: "browser-4",
                    url: "https://nrk.no/",
                    title: "NRK",
                    visible: false,
                    loading: false,
                    documentId: 2,
                    vimKeysEnabled: true,
                    keyMode: "normal" as const,
                },
            ],
            activeSessionId: "browser-4",
            maxSessions: 8,
            presentationRequest: {
                revision: 7,
                sessionId: "browser-4",
            },
        };

        expect(requestedBrowserPresentation(0, state)).toEqual({
            revision: 7,
            sessionId: "browser-4",
        });
        expect(requestedBrowserPresentation(7, state)).toBeUndefined();
        expect(
            requestedBrowserPresentation(0, {
                ...state,
                sessions: [],
            }),
        ).toEqual({ revision: 7, sessionId: undefined });
    });

    it("renders an interactive concept walkthrough from projected core state", () => {
        const onKey = vi.fn();
        const walkthrough: GuiCodeExplanation = {
            answerInProgress: false,
            current: 1,
            total: 2,
            page: {
                kind: "concept",
                title: "Two layers of history",
                body: "Input recall and conversation navigation are **separate** concerns.",
            },
            discussion: {
                state: "navigating",
                questionCount: 0,
                latestFailed: false,
            },
        };

        render(() => (
            <CodeWalkthrough walkthrough={walkthrough} onKey={onKey} />
        ));

        expect(
            screen.getByRole("dialog", { name: "Two layers of history" }),
        ).toBeTruthy();
        expect(screen.getByText("separate").tagName).toBe("STRONG");
        fireEvent.click(screen.getByRole("button", { name: "Next" }));
        fireEvent.click(screen.getByRole("button", { name: "Ask a question" }));
        expect(onKey).toHaveBeenNthCalledWith(1, "ArrowRight");
        expect(onKey).toHaveBeenNthCalledWith(2, " ");
    });

    it("reopens a hidden thread while keeping streaming controls disabled", () => {
        const onKey = vi.fn();
        const walkthrough: GuiCodeExplanation = {
            answerInProgress: true,
            current: 1,
            total: 2,
            page: {
                kind: "concept",
                title: "Original step",
                body: "The original explanation.",
            },
            discussion: {
                state: "navigating",
                questionCount: 2,
                latestFailed: false,
            },
        };
        render(() => (
            <CodeWalkthrough walkthrough={walkthrough} onKey={onKey} />
        ));
        fireEvent.click(screen.getByRole("button", { name: "Show thread" }));
        expect(onKey).toHaveBeenCalledWith("t");
        expect(
            screen
                .getByRole("button", { name: "Continue" })
                .hasAttribute("disabled"),
        ).toBe(true);
        expect(
            screen
                .getByRole("button", { name: "Ask a question" })
                .hasAttribute("disabled"),
        ).toBe(true);
        expect(screen.getByText("The original explanation.")).toBeTruthy();
    });

    it("renders code location and live answer state", () => {
        const walkthrough: GuiCodeExplanation = {
            answerInProgress: true,
            current: 2,
            total: 2,
            page: {
                kind: "code",
                path: "src/main.rs",
                startLine: 12,
                endLine: 14,
                comment: "This branch owns the handoff.",
            },
            discussion: {
                state: "answering",
                question: "Why here?",
                answer: "Because it owns the boundary.",
                questionCount: 1,
            },
        };

        render(() => (
            <CodeWalkthrough walkthrough={walkthrough} onKey={() => {}} />
        ));

        expect(
            screen.getByRole("dialog", { name: "src/main.rs:12–14" }),
        ).toBeTruthy();
        expect(screen.getByLabelText("Answering")).toBeTruthy();
        expect(
            screen.getByRole("button", { name: "Back to step" }),
        ).toBeTruthy();
        expect(
            screen.getByRole("button", { name: "Earlier question" }),
        ).toBeTruthy();
        expect(screen.getByText("Because it owns the boundary.")).toBeTruthy();
        expect(
            screen
                .getByRole("button", { name: "Finish" })
                .hasAttribute("disabled"),
        ).toBe(true);
    });

    it("renders a keyboard-accessible editor projection from the snapshot", async () => {
        const result = render(() => <App />);

        expect(
            screen.getByRole("navigation", { name: "Primary navigation" }),
        ).toBeTruthy();
        expect(screen.getByRole("button", { name: "Close" })).toBeTruthy();
        expect(screen.getByLabelText("Ovim editor input")).toBeTruthy();
        expect(screen.getByRole("tablist", { name: "Open tabs" })).toBeTruthy();
        expect(
            screen.getByRole("tree", { name: "Project files" }),
        ).toBeTruthy();
        expect(
            result.container.querySelectorAll(".code-line").length,
        ).toBeGreaterThan(10);
        expect(
            result.container.querySelector(".code-segment.cursor"),
        ).toBeTruthy();

        const focus = vi.mocked(HTMLElement.prototype.focus);
        focus.mockClear();
        fireEvent.mouseDown(result.container.querySelector(".line-content")!, {
            clientX: 80,
        });
        expect(focus.mock.instances).toContain(
            screen.getByLabelText("Ovim editor input"),
        );

        focus.mockClear();
        fireEvent.click(screen.getByRole("tab", { name: "FRONTEND_API.md" }));
        await Promise.resolve();
        expect(focus.mock.instances).toContain(
            screen.getByLabelText("Ovim editor input"),
        );

        focus.mockClear();
        const tabs = screen.getAllByRole("tab");
        fireEvent.keyDown(tabs[0], { key: "ArrowRight" });
        await Promise.resolve();
        expect(focus.mock.instances).toContain(tabs[1]);
    });

    it("switches the editor caret shape with insert mode without replacing its text", () => {
        const previousMode = mockSnapshot.mode;
        mockSnapshot.mode = "INSERT";

        try {
            const result = render(() => <App />);
            const cursor = result.container.querySelector(
                ".editor-pane.insert-mode .code-segment.cursor",
            );

            expect(cursor).toBeTruthy();
            expect(cursor?.textContent).toBe("            title");
        } finally {
            mockSnapshot.mode = previousMode;
        }
    });

    it("anchors the existing chat composer below an attached visual selection", () => {
        const previousChat = mockSnapshot.aiChat;
        mockSnapshot.aiChat = {
            ...previousChat!,
            pendingCodeAttachment: {
                bufferId: mockSnapshot.panes[0].bufferId,
                label: "src/main.rs:4–6",
                startLine: 3,
                startColumn: 0,
                endLine: 5,
                endColumn: 4,
                linewise: true,
            },
        };

        const result = render(() => <App />);
        try {
            expect(
                screen.getByRole("complementary", {
                    name: "Ask Ovim about src/main.rs:4–6",
                }),
            ).toBeTruthy();
            expect(screen.getAllByLabelText("AI chat input")).toHaveLength(1);
            expect(
                result.container.querySelector(".ai-panel > .chat-composer"),
            ).toBeNull();
        } finally {
            result.unmount();
            mockSnapshot.aiChat = previousChat;
        }
    });

    it("opens the terminal from the activity bar", async () => {
        render(() => <App />);

        const terminal = screen.getByRole("button", {
            name: "Terminal",
        });
        expect(terminal.hasAttribute("disabled")).toBe(false);
        fireEvent.click(terminal);

        expect(await screen.findByRole("tabpanel")).toBeTruthy();
        expect(
            screen.getByText("Terminal is available in the desktop app."),
        ).toBeTruthy();
    });

    it.each(["MacIntel", "Linux x86_64"])(
        "toggles the terminal with the platform shortcut on %s while keeping its dock mounted",
        (platform) => {
            vi.spyOn(navigator, "platform", "get").mockReturnValue(platform);
            const result = render(() => <App />);
            const shortcut = {
                key: "T",
                code: "KeyT",
                shiftKey: true,
                ctrlKey: platform !== "MacIntel",
                metaKey: platform === "MacIntel",
            };

            fireEvent.keyDown(document.body, shortcut);
            const panel = screen.getByRole("tabpanel", { name: "Terminal" });
            expect(
                screen.getByText("Terminal is available in the desktop app."),
            ).toBeTruthy();

            fireEvent.keyDown(document.body, shortcut);
            expect(
                result.container.querySelector(".workbench")?.classList,
            ).toContain("terminal-collapsed");
            expect(result.container.querySelector(".context-panel")).toBe(
                panel,
            );

            fireEvent.keyDown(document.body, shortcut);
            expect(
                result.container.querySelector(".workbench")?.classList,
            ).not.toContain("terminal-collapsed");
            expect(screen.getByRole("tabpanel", { name: "Terminal" })).toBe(
                panel,
            );
        },
    );

    it("opens the terminal from the dashboard", () => {
        const previousDashboard = mockSnapshot.dashboard;
        const previousTree = mockSnapshot.fileTree;
        mockSnapshot.dashboard = true;
        mockSnapshot.fileTree = undefined;
        try {
            const result = render(() => <App />);
            const button = screen.getByRole("button", { name: "Terminal" });
            fireEvent.click(button);
            expect(
                screen.getByText("Terminal is available in the desktop app."),
            ).toBeTruthy();
            fireEvent.click(button);
            expect(
                result.container.querySelector(".workbench")?.classList,
            ).toContain("terminal-collapsed");
            fireEvent.keyDown(document.body, {
                key: "T",
                code: "KeyT",
                shiftKey: true,
                ctrlKey: true,
            });
            expect(
                result.container.querySelector(".workbench")?.classList,
            ).not.toContain("terminal-collapsed");
        } finally {
            mockSnapshot.dashboard = previousDashboard;
            mockSnapshot.fileTree = previousTree;
        }
    });

    it("keeps Vector in tab navigation without a perpetual browser tab", async () => {
        const previousPath = mockSnapshot.filePath;
        const previousName = mockSnapshot.fileName;
        mockSnapshot.filePath = "/workspace/ovim/icons/sample.strok";
        mockSnapshot.fileName = "sample.strok";

        const result = render(() => <App />);
        try {
            const source = screen.getByRole("tab", { name: /mod.rs/ });
            const vector = screen.getByRole("tab", { name: "Vector" });
            expect(source.getAttribute("aria-selected")).toBe("true");
            expect(vector.getAttribute("aria-selected")).toBe("false");
            expect(screen.queryByRole("tab", { name: /Browser:/ })).toBeNull();
            expect(
                screen
                    .getByRole("button", { name: "New browser tab" })
                    .hasAttribute("disabled"),
            ).toBe(true);
            expect(
                screen.getAllByRole("tab").filter((tab) => tab.tabIndex === 0),
            ).toHaveLength(1);

            fireEvent.click(vector);
            await Promise.resolve();
            expect(source.getAttribute("aria-selected")).toBe("false");
            expect(vector.getAttribute("aria-selected")).toBe("true");
            expect(
                screen.getAllByRole("tab").filter((tab) => tab.tabIndex === 0),
            ).toEqual([vector]);
            expect(
                screen.getByRole("region", { name: "Strøk vector preview" }),
            ).toBeTruthy();

            fireEvent.keyDown(vector, { key: "Home" });
            await Promise.resolve();
            expect(source.getAttribute("aria-selected")).toBe("true");
            expect(vector.getAttribute("aria-selected")).toBe("false");

            fireEvent.click(vector);
            const feedback = screen.getByLabelText("Review with the agent");
            fireEvent.input(feedback, {
                target: { value: "Reduce the visual weight." },
            });
            fireEvent.click(
                screen.getByRole("button", { name: "Add to agent chat" }),
            );
            await Promise.resolve();
            expect(
                screen.getByText(
                    "Added to the AI chat draft — review and send when ready.",
                ),
            ).toBeTruthy();
            expect((feedback as HTMLTextAreaElement).value).toBe("");
        } finally {
            result.unmount();
            mockSnapshot.filePath = previousPath;
            mockSnapshot.fileName = previousName;
        }
    });

    it("restores the persisted terminal panel", async () => {
        window.localStorage.setItem(
            "ovim.gui.layout.v1.%2Fworkspace%2Fovim",
            JSON.stringify({
                activeDock: "context",
                activeContextPanel: "terminal",
            }),
        );

        render(() => <App />);

        expect(
            await screen.findByText(
                "Terminal is available in the desktop app.",
            ),
        ).toBeTruthy();
    });

    it("switches existing compact docks without toggling their core state", () => {
        const previousChat = mockSnapshot.aiChat;
        mockSnapshot.aiChat = {
            profile: "codex",
            profiles: [],
            reasoningEffort: "medium",
            reasoningEffortSelection: "default",
            reasoningEfforts: ["default"],
            yoloMode: false,
            comprehensionPolicy: "off",
            activity: "idle",
            waiting: false,
            input: "",
            inputCursor: 0,
            pendingImages: [],
            queuedInputs: [],
            messages: [],
            thinkingLive: false,
            focus: "textInput",
            agents: [],
            agentCursor: 0,
        };
        vi.stubGlobal(
            "matchMedia",
            vi.fn(() => ({
                matches: true,
                addEventListener: vi.fn(),
                removeEventListener: vi.fn(),
            })),
        );

        const result = render(() => <App />);
        try {
            const workbench = result.container.querySelector(".workbench")!;
            expect(workbench.classList).toContain("active-context-dock");

            fireEvent.click(screen.getByRole("button", { name: "Explorer" }));
            expect(workbench.classList).toContain("active-explorer-dock");
            expect(mockSnapshot.fileTree).toBeTruthy();

            fireEvent.click(screen.getByRole("button", { name: "AI chat" }));
            expect(workbench.classList).toContain("active-context-dock");
            expect(mockSnapshot.aiChat).toBeTruthy();

            const terminal = screen.getByRole("button", {
                name: "Terminal",
            });
            fireEvent.click(terminal);
            expect(workbench.classList).toContain("active-context-dock");
            expect(
                screen.getByText("Terminal is available in the desktop app."),
            ).toBeTruthy();

            fireEvent.click(terminal);
            expect(workbench.classList).toContain("active-explorer-dock");
        } finally {
            result.unmount();
            mockSnapshot.aiChat = previousChat;
        }
    });

    it("renders attached selections as compact chat context", () => {
        render(() => (
            <ChatMessageView
                message={{
                    id: "selection",
                    index: 0,
                    selected: false,
                    role: "user",
                    content: "Check this change",
                    attachment: "src/main.rs:5–6",
                    tools: [],
                }}
            />
        ));
        expect(screen.getByText("src/main.rs:5–6")).toBeTruthy();
        expect(screen.getByText("Check this change")).toBeTruthy();
    });

    it("sanitizes rendered AI markdown", () => {
        const result = render(() => (
            <Markdown
                text={'**safe**<img src="x" onerror="window.__unsafe = true">'}
            />
        ));

        expect(screen.getByText("safe").tagName).toBe("STRONG");
        expect(
            result.container.querySelector("img")?.hasAttribute("onerror"),
        ).toBe(false);
        expect(result.container.querySelector("script")).toBeNull();
    });

    it("shows sent image attachments in the chat transcript", () => {
        render(() => (
            <ChatMessageView
                message={{
                    id: "1:1",
                    index: 0,
                    selected: false,
                    role: "user",
                    content: "Please inspect this screenshot",
                    tools: [],
                    images: ["layout.png", "Pasted image 2"],
                }}
            />
        ));

        expect(screen.getByLabelText("Attached images")).toBeTruthy();
        expect(screen.getByText("layout.png")).toBeTruthy();
        expect(screen.getByText("Pasted image 2")).toBeTruthy();
    });

    it("hands safe markdown links to the desktop opener without navigating the editor", () => {
        const onOpenLink = vi.fn();
        const result = render(() => (
            <Markdown
                text="Read the [Ovim guide](https://example.com/guide)."
                onOpenLink={onOpenLink}
            />
        ));
        const link = screen.getByRole("link", { name: "Ovim guide" });
        const click = new MouseEvent("click", {
            bubbles: true,
            cancelable: true,
        });

        link.dispatchEvent(click);

        expect(click.defaultPrevented).toBe(true);
        expect(onOpenLink).toHaveBeenCalledWith("https://example.com/guide");
        expect(result.container.ownerDocument.location.href).toBe(
            "http://localhost:3000/",
        );
    });

    it("uses a native textarea while preserving UTF-8 core cursor offsets", async () => {
        const onUpdate = vi.fn().mockResolvedValue(undefined);
        render(() => (
            <ChatComposer
                onUpdate={onUpdate}
                chat={{
                    profile: "codex",
                    profiles: [
                        { id: "codex", provider: "codex", model: "gpt-test" },
                    ],
                    reasoningEffort: "high",
                    reasoningEffortSelection: "default",
                    reasoningEfforts: ["default", "high"],
                    yoloMode: false,
                    comprehensionPolicy: "off",
                    activity: "idle",
                    waiting: false,
                    input: "a界b",
                    inputCursor: 4,
                    queuedInputs: [],
                    pendingImages: ["diagram.png"],
                    messages: [],
                    thinkingLive: false,
                    focus: "textInput",
                    agents: [],
                    agentCursor: 0,
                }}
            />
        ));

        expect(screen.getByText("diagram.png")).toBeTruthy();
        const input = screen.getByLabelText(
            "AI chat input",
        ) as HTMLTextAreaElement;
        await Promise.resolve();
        expect(input.value).toBe("a界b");
        expect(input.selectionStart).toBe(2);

        input.value = "a›界b";
        input.setSelectionRange(2, 2);
        fireEvent.input(input);
        await Promise.resolve();
        expect(onUpdate).toHaveBeenCalledWith({
            expectedInput: "a界b",
            expectedCursor: 4,
            input: "a›界b",
            cursor: 4,
            action: undefined,
        });
    });

    it("keeps a native word selection while chat snapshots arrive", async () => {
        let finishUpdate!: () => void;
        const onUpdate = vi.fn(
            () => new Promise<void>((resolve) => (finishUpdate = resolve)),
        );
        const [chat, setChat] = createSignal<GuiAiChat>({
            ...mockSnapshot.aiChat!,
            input: "one two three",
            inputCursor: 13,
        });
        render(() => <ChatComposer chat={chat()} onUpdate={onUpdate} />);
        const input = screen.getByLabelText(
            "AI chat input",
        ) as HTMLTextAreaElement;
        await Promise.resolve();
        vi.mocked(HTMLElement.prototype.focus).mockRestore();
        input.focus();
        input.setSelectionRange(8, 13, "backward");
        fireEvent.select(input);
        await waitFor(() => expect(onUpdate).toHaveBeenCalledTimes(1));
        expect(onUpdate).toHaveBeenCalledWith(
            expect.objectContaining({ expectedCursor: 13, cursor: 8 }),
        );

        // A stale snapshot can arrive before the queued native edit completes.
        setChat({ ...chat(), activity: "working" });
        await Promise.resolve();
        expect(input.selectionStart).toBe(8);
        expect(input.selectionEnd).toBe(13);

        finishUpdate();
        await Promise.resolve();
        setChat({ ...chat(), inputCursor: 8 });
        await Promise.resolve();
        expect(input.selectionStart).toBe(8);
        expect(input.selectionEnd).toBe(13);

        setChat({ ...chat(), activity: "idle" });
        await Promise.resolve();

        expect(input.selectionStart).toBe(8);
        expect(input.selectionEnd).toBe(13);
        expect(input.selectionDirection).toBe("backward");

        const menuButton = document.createElement("button");
        document.body.append(menuButton);
        menuButton.focus();
        expect(document.activeElement).toBe(menuButton);
        setChat({ ...chat(), activity: "working" });
        await Promise.resolve();

        expect(input.selectionStart).toBe(8);
        expect(input.selectionEnd).toBe(13);
        expect(input.selectionDirection).toBe("backward");
    });

    it("uses the active end of a forward selection for core cursor updates", async () => {
        const onUpdate = vi.fn().mockResolvedValue(undefined);
        render(() => (
            <ChatComposer
                chat={{
                    ...mockSnapshot.aiChat!,
                    input: "one two three",
                    inputCursor: 0,
                }}
                onUpdate={onUpdate}
            />
        ));
        const input = screen.getByLabelText(
            "AI chat input",
        ) as HTMLTextAreaElement;
        await Promise.resolve();
        vi.mocked(HTMLElement.prototype.focus).mockRestore();
        input.focus();
        input.setSelectionRange(0, 3, "forward");
        fireEvent.select(input);

        await waitFor(() => expect(onUpdate).toHaveBeenCalledTimes(1));
        expect(onUpdate).toHaveBeenCalledWith(
            expect.objectContaining({ expectedCursor: 0, cursor: 3 }),
        );
    });

    it("applies an external core cursor change to the focused composer", async () => {
        const [chat, setChat] = createSignal<GuiAiChat>({
            ...mockSnapshot.aiChat!,
            input: "one two three",
            inputCursor: 13,
        });
        render(() => <ChatComposer chat={chat()} />);
        const input = screen.getByLabelText(
            "AI chat input",
        ) as HTMLTextAreaElement;
        await Promise.resolve();
        vi.mocked(HTMLElement.prototype.focus).mockRestore();
        input.focus();

        setChat({ ...chat(), inputCursor: 4 });
        await Promise.resolve();

        expect(input.selectionStart).toBe(4);
        expect(input.selectionEnd).toBe(4);
    });

    it("publishes submit atomically with the latest native draft", async () => {
        const onUpdate = vi.fn().mockResolvedValue(undefined);
        render(() => (
            <ChatComposer
                onUpdate={onUpdate}
                chat={{
                    profile: "codex",
                    profiles: [],
                    reasoningEffort: "medium",
                    reasoningEffortSelection: "default",
                    reasoningEfforts: ["default"],
                    yoloMode: false,
                    comprehensionPolicy: "off",
                    activity: "idle",
                    waiting: false,
                    input: "",
                    inputCursor: 0,
                    queuedInputs: [],
                    pendingImages: [],
                    messages: [],
                    thinkingLive: false,
                    focus: "textInput",
                    agents: [],
                    agentCursor: 0,
                }}
            />
        ));
        const input = screen.getByLabelText(
            "AI chat input",
        ) as HTMLTextAreaElement;
        input.value = "ship this";
        input.setSelectionRange(9, 9);
        fireEvent.input(input);
        await Promise.resolve();
        input.setSelectionRange(9, 9);
        fireEvent.keyDown(input, { key: "Enter" });
        await waitFor(() => expect(onUpdate).toHaveBeenCalledTimes(2));

        expect(onUpdate).toHaveBeenLastCalledWith(
            expect.objectContaining({
                expectedInput: "ship this",
                input: "ship this",
                cursor: 9,
                action: expect.objectContaining({ key: "Enter" }),
            }),
        );
        expect(input.value).toBe("");
    });

    it("returns DOM focus to the editor input when AI chat is activated", () => {
        const previousChat = mockSnapshot.aiChat;
        mockSnapshot.aiChat = undefined;
        try {
            render(() => <App />);
            const input = screen.getByLabelText("Ovim editor input");
            const focus = vi.mocked(HTMLElement.prototype.focus);
            focus.mockClear();

            fireEvent.click(
                document.querySelector<HTMLButtonElement>(
                    '[title^="AI chat"]',
                )!,
            );

            expect(focus).toHaveBeenCalledWith({ preventScroll: true });
            expect(focus.mock.instances).toContain(input);
        } finally {
            mockSnapshot.aiChat = previousChat;
        }
    });

    it("follows chat updates until the reader scrolls away from the bottom", async () => {
        const initial: GuiAiChat = {
            profile: "codex",
            profiles: [{ id: "codex", provider: "codex", model: "gpt-test" }],
            reasoningEffort: "high",
            reasoningEffortSelection: "default",
            reasoningEfforts: ["default", "high"],
            yoloMode: false,
            comprehensionPolicy: "off",
            activity: "idle",
            waiting: false,
            input: "",
            inputCursor: 0,
            queuedInputs: [],
            pendingImages: [],
            messages: [
                {
                    id: "1:1",
                    index: 0,
                    selected: false,
                    role: "assistant",
                    content: "First response",
                    model: "codex",
                    tools: [],
                },
            ],
            thinkingLive: false,
            focus: "textInput",
            agents: [],
            agentCursor: 0,
        };
        const [chat, setChat] = createSignal(initial);
        const result = render(() => (
            <ChatPanel chat={chat()} focusInput={() => {}} />
        ));
        const transcript =
            result.container.querySelector<HTMLElement>(".chat-messages")!;
        Object.defineProperties(transcript, {
            scrollHeight: { configurable: true, value: 600, writable: true },
            clientHeight: { configurable: true, value: 200 },
            scrollTop: { configurable: true, value: 0, writable: true },
        });

        await Promise.resolve();
        expect(transcript.scrollTop).toBe(600);

        transcript.scrollTop = 360;
        fireEvent.scroll(transcript);
        Object.defineProperty(transcript, "scrollHeight", {
            configurable: true,
            value: 700,
        });
        setChat({ ...initial, streaming: "Streaming while pinned" });
        await Promise.resolve();
        expect(transcript.scrollTop).toBe(700);

        transcript.scrollTop = 100;
        fireEvent.scroll(transcript);
        expect(
            screen.getByRole("button", { name: "New messages" }),
        ).toBeTruthy();
        setChat({ ...initial, streaming: "More streaming content" });
        await Promise.resolve();
        expect(transcript.scrollTop).toBe(100);

        fireEvent.click(screen.getByRole("button", { name: "New messages" }));
        expect(transcript.scrollTop).toBe(700);
    });

    it("does not repeatedly pull the reader back to an unchanged history selection", async () => {
        const selectedMessage: GuiAiChat["messages"][number] = {
            id: "1:1",
            index: 0,
            selected: true,
            role: "assistant",
            content: "Pinned response",
            model: "codex",
            tools: [],
        };
        const initial: GuiAiChat = {
            profile: "codex",
            profiles: [],
            reasoningEffort: "medium",
            reasoningEffortSelection: "default",
            reasoningEfforts: ["default"],
            yoloMode: false,
            comprehensionPolicy: "off",
            activity: "idle",
            waiting: false,
            input: "",
            inputCursor: 0,
            queuedInputs: [],
            pendingImages: [],
            messages: [selectedMessage],
            thinkingLive: false,
            focus: "textInput",
            agents: [],
            agentCursor: 0,
        };
        const [chat, setChat] = createSignal(initial);
        const result = render(() => (
            <ChatPanel chat={chat()} focusInput={() => {}} />
        ));
        const transcript =
            result.container.querySelector<HTMLElement>(".chat-messages")!;
        Object.defineProperties(transcript, {
            scrollHeight: { configurable: true, value: 600, writable: true },
            clientHeight: { configurable: true, value: 200 },
            scrollTop: { configurable: true, value: 100, writable: true },
        });

        await Promise.resolve();
        fireEvent.click(screen.getByRole("button", { name: "New messages" }));
        expect(transcript.scrollTop).toBe(600);

        setChat({
            ...initial,
            activity: "streaming",
            streaming: "New content on the same selected turn",
        });
        await Promise.resolve();

        expect(transcript.scrollTop).toBe(600);
        expect(
            screen.queryByRole("button", { name: "New activity" }),
        ).toBeNull();
    });

    it("offers configured model selections and releases pointer focus back to chat input", async () => {
        const onProfile = vi.fn();
        const onReasoningEffort = vi.fn();
        const onQueuedAction = vi.fn();
        const onYolo = vi.fn();
        const onComprehension = vi.fn();
        const focusInput = vi.fn();
        const chat: GuiAiChat = {
            profile: "codex",
            profiles: [
                { id: "codex", provider: "codex", model: "gpt-test" },
                { id: "local", provider: "ollama", model: "qwen-test" },
            ],
            reasoningEffort: "medium",
            reasoningEffortSelection: "default",
            reasoningEfforts: ["default", "low", "medium", "high"],
            yoloMode: false,
            comprehensionPolicy: "off",
            activity: "idle",
            waiting: false,
            input: "preserved",
            inputCursor: 9,
            queuedInputs: [
                {
                    id: 7,
                    kind: "followUp",
                    content: "Check the remaining tests",
                    imageCount: 1,
                    hasCodeAttachment: true,
                    selected: false,
                },
            ],
            pendingImages: [],
            messages: [],
            thinkingLive: false,
            focus: "textInput",
            agents: [],
            agentCursor: 0,
        };
        render(() => (
            <ChatPanel
                chat={chat}
                focusInput={focusInput}
                onProfile={onProfile}
                onReasoningEffort={onReasoningEffort}
                onYolo={onYolo}
                onComprehension={onComprehension}
                onQueuedAction={onQueuedAction}
            />
        ));

        const trigger = screen.getByRole("button", {
            name: /codex.*default.*medium/i,
        });
        fireEvent.click(trigger);
        expect(
            (screen.getByLabelText("AI provider") as HTMLSelectElement).value,
        ).toBe("codex");
        expect(
            (screen.getByLabelText("AI model") as HTMLSelectElement).value,
        ).toBe(JSON.stringify(["codex", "gpt-test"]));
        fireEvent.change(screen.getByLabelText("AI provider"), {
            target: { value: "ollama" },
        });
        expect(onProfile).toHaveBeenCalledWith("local", "qwen-test");
        expect(
            (screen.getByLabelText("AI model") as HTMLSelectElement).value,
        ).toBe(JSON.stringify(["local", "qwen-test"]));
        fireEvent.change(screen.getByLabelText("Reasoning effort"), {
            target: { value: "high" },
        });
        expect(onReasoningEffort).toHaveBeenCalledWith("high");
        fireEvent.click(screen.getByRole("button", { name: "Done" }));
        await Promise.resolve();
        expect(focusInput).toHaveBeenCalledOnce();
        fireEvent.click(screen.getByRole("button", { name: "YOLO OFF" }));
        fireEvent.click(
            screen.getByRole("button", { name: "COMPREHENSION OFF" }),
        );
        expect(onYolo).toHaveBeenCalledOnce();
        expect(onComprehension).toHaveBeenCalledOnce();
        expect(screen.getByText("Queued message")).toBeTruthy();
        expect(screen.getByText("Check the remaining tests")).toBeTruthy();
        expect(screen.getByText("1 image")).toBeTruthy();
        expect(screen.getByText("code attached")).toBeTruthy();
        fireEvent.click(screen.getByText("Check the remaining tests"));
        fireEvent.click(screen.getByRole("button", { name: "Edit" }));
        fireEvent.click(screen.getByRole("button", { name: "Remove" }));
        expect(onQueuedAction.mock.calls).toEqual([
            [7, "select"],
            [7, "recall"],
            [7, "remove"],
        ]);
    });

    it("exposes send, stop, and attachment removal as working controls", async () => {
        const onUpdate = vi.fn().mockResolvedValue(undefined);
        const onRemoveImage = vi.fn();
        const chat: GuiAiChat = {
            profile: "codex",
            profiles: [],
            reasoningEffort: "medium",
            reasoningEffortSelection: "default",
            reasoningEfforts: ["default"],
            yoloMode: false,
            comprehensionPolicy: "off",
            activity: "idle",
            waiting: false,
            input: "ship this",
            inputCursor: 9,
            queuedInputs: [],
            pendingImages: ["diagram.png"],
            messages: [],
            thinkingLive: false,
            focus: "textInput",
            agents: [],
            agentCursor: 0,
        };
        const result = render(() => (
            <ChatComposer
                chat={chat}
                onUpdate={onUpdate}
                onRemoveImage={onRemoveImage}
            />
        ));

        fireEvent.click(
            screen.getByRole("button", { name: "Remove diagram.png" }),
        );
        expect(onRemoveImage).toHaveBeenCalledWith(0);
        fireEvent.click(screen.getByRole("button", { name: "Send message" }));
        await waitFor(() => expect(onUpdate).toHaveBeenCalled());
        expect(onUpdate).toHaveBeenLastCalledWith(
            expect.objectContaining({
                input: "ship this",
                action: expect.objectContaining({ key: "Enter" }),
            }),
        );
        result.unmount();

        const onStop = vi.fn().mockResolvedValue(undefined);
        render(() => (
            <ChatComposer
                chat={{ ...chat, activity: "streaming", waiting: true }}
                onUpdate={onStop}
            />
        ));
        fireEvent.click(
            screen.getByRole("button", { name: "Stop generation" }),
        );
        await waitFor(() => expect(onStop).toHaveBeenCalled());
        expect(onStop).toHaveBeenLastCalledWith(
            expect.objectContaining({
                action: expect.objectContaining({ key: "Escape" }),
            }),
        );
    });

    it("shows and activates core history and agent navigation state", () => {
        const onMessage = vi.fn();
        const onAgent = vi.fn();
        const chat: GuiAiChat = {
            profile: "codex",
            profiles: [{ id: "codex", provider: "codex", model: "gpt-test" }],
            reasoningEffort: "high",
            reasoningEffortSelection: "default",
            reasoningEfforts: ["default", "high"],
            yoloMode: false,
            comprehensionPolicy: "off",
            activity: "idle",
            waiting: false,
            input: "",
            inputCursor: 0,
            queuedInputs: [],
            pendingImages: [],
            thinkingLive: false,
            focus: "treePanel",
            agentCursor: 1,
            selectedAgentId: "agt_1",
            agents: [
                {
                    id: "agt_1",
                    taskName: "Review changes",
                    lifecycle: "running",
                    model: "gpt-test",
                    depth: 0,
                },
            ],
            messages: [
                {
                    id: "1:1",
                    index: 0,
                    selected: true,
                    role: "assistant",
                    content: "Selected response",
                    tools: [],
                },
            ],
        };
        const result = render(() => (
            <ChatPanel
                chat={chat}
                focusInput={() => {}}
                onMessage={onMessage}
                onAgent={onAgent}
            />
        ));

        expect(
            result.container.querySelector(".chat-message.selected"),
        ).toBeTruthy();
        expect(
            screen
                .getByRole("button", { name: /Review changes/ })
                .classList.contains("cursor"),
        ).toBe(true);
        fireEvent.click(screen.getByText("Selected response"));
        fireEvent.click(screen.getByRole("button", { name: /Review changes/ }));
        expect(onMessage).toHaveBeenCalledWith(0);
        expect(onAgent).toHaveBeenCalledWith("agt_1");
    });

    it("uses a small threshold when deciding whether chat should follow", () => {
        expect(
            isNearChatBottom({
                scrollHeight: 500,
                scrollTop: 260,
                clientHeight: 200,
            }),
        ).toBe(true);
        expect(
            isNearChatBottom({
                scrollHeight: 500,
                scrollTop: 200,
                clientHeight: 200,
            }),
        ).toBe(false);
    });

    it("does not let stale editor overlays cover an active AI chat", () => {
        mockSnapshot.aiChat = {
            profile: "codex",
            profiles: [{ id: "codex", provider: "codex", model: "gpt-test" }],
            reasoningEffort: "high",
            reasoningEffortSelection: "default",
            reasoningEfforts: ["default", "high"],
            yoloMode: false,
            comprehensionPolicy: "off",
            activity: "idle",
            waiting: false,
            input: "visible draft",
            inputCursor: 13,
            queuedInputs: [],
            pendingImages: [],
            messages: [],
            thinkingLive: false,
            focus: "textInput",
            agents: [],
            agentCursor: 0,
        };
        mockSnapshot.picker = {
            title: "Stale picker",
            query: "",
            selected: 0,
            total: 1,
            items: [
                {
                    index: 0,
                    display: "Result",
                    location: "src/main.rs",
                    matched: [],
                },
            ],
        };
        mockSnapshot.lspManager = {
            filter: "",
            selected: 0,
            showDetail: false,
            items: [],
        };
        mockSnapshot.hover = { content: "Stale hover" };
        mockSnapshot.completion = {
            selected: 0,
            items: [{ index: 0, label: "stale" }],
        };

        try {
            const result = render(() => <App />);
            expect(
                (screen.getByLabelText("AI chat input") as HTMLTextAreaElement)
                    .value,
            ).toContain("visible draft");
            expect(result.container.querySelector(".overlay-shade")).toBeNull();
            expect(result.container.querySelector(".hover-popover")).toBeNull();
            expect(
                result.container.querySelector(".completion-popover"),
            ).toBeNull();
        } finally {
            delete mockSnapshot.aiChat;
            delete mockSnapshot.picker;
            delete mockSnapshot.lspManager;
            delete mockSnapshot.hover;
            delete mockSnapshot.completion;
        }
    });

    it("renders signature help with the active parameter highlighted", () => {
        mockSnapshot.signatureHelp = {
            before: "join(String delimiter, ",
            active: "String... parts",
            after: ")",
            documentation: "the parts to join",
            signatureIndex: 1,
            signatureCount: 3,
            line: 0,
            displayColumn: 5,
        };
        try {
            const result = render(() => <App />);
            const popover =
                result.container.querySelector(".signature-popover");
            expect(popover).not.toBeNull();
            expect(
                popover?.querySelector(".signature-active")?.textContent,
            ).toBe("String... parts");
            expect(popover?.textContent).toContain("2/3");
            expect(popover?.textContent).toContain("the parts to join");
        } finally {
            delete mockSnapshot.signatureHelp;
        }
    });

    it("renders completion rows with kind, label details, matches, deprecation and documentation", () => {
        mockSnapshot.completion = {
            selected: 0,
            total: 40,
            items: [
                {
                    index: 0,
                    label: "getEmail",
                    detail: "()",
                    description: "String",
                    kind: "Method",
                    kindGlyph: "m",
                    kindClass: "function",
                    matched: [0, 3, 4],
                },
                {
                    index: 1,
                    label: "ArrayList",
                    description: "java.util",
                    kind: "Class",
                    kindGlyph: "C",
                    kindClass: "type",
                    deprecated: true,
                    matched: [],
                },
            ],
            documentation:
                "```\nString getEmail()\n```\n\nReturns the **email**.",
        };
        try {
            const result = render(() => <App />);
            const rows = result.container.querySelectorAll(".completion-item");
            expect(rows).toHaveLength(2);
            expect(rows[0].querySelector(".completion-kind")?.textContent).toBe(
                "m",
            );
            expect(
                rows[0].querySelector(".completion-kind")?.className,
            ).toContain("kind-function");
            expect(
                rows[0].querySelector(".completion-detail")?.textContent,
            ).toBe("()");
            expect(
                [...rows[0].querySelectorAll(".completion-match")].map(
                    (mark) => mark.textContent,
                ),
            ).toEqual(["g", "Em"]);
            expect(
                rows[0].querySelector(".completion-description")?.textContent,
            ).toBe("String");
            expect(rows[1].classList.contains("deprecated")).toBe(true);
            expect(
                rows[1].querySelector(".completion-description")?.textContent,
            ).toBe("java.util");
            expect(
                result.container.querySelector(".completion-count")
                    ?.textContent,
            ).toContain("1 / 40");
            expect(
                result.container.querySelector(".completion-docs")?.textContent,
            ).toContain("Returns the email.");
        } finally {
            delete mockSnapshot.completion;
        }
    });

    it("marks a closed fold header with the hidden line count", () => {
        const pane = mockSnapshot.panes[0];
        const original = pane.lines[0].folded;
        pane.lines[0].folded = 12;
        try {
            const result = render(() => <App />);
            expect(
                result.container.querySelector(".fold-marker")?.textContent,
            ).toContain("12 lines");
        } finally {
            pane.lines[0].folded = original;
        }
    });

    it("draws fold markers in the gutter: closed headers always, inside lines never", () => {
        const pane = mockSnapshot.panes[0];
        const original = pane.lines.slice(0, 3).map((line) => line.fold);
        pane.lines[0].fold = "closed";
        pane.lines[1].fold = "open";
        pane.lines[2].fold = "inside";
        try {
            const result = render(() => <App />);
            const marks = result.container.querySelectorAll(".fold-gutter");
            expect(marks).toHaveLength(3);
            expect(marks[0].textContent).toBe("▸");
            expect(marks[0].classList.contains("fold-closed")).toBe(true);
            expect(marks[1].textContent).toBe("▾");
            expect(marks[2].textContent).toBe("");
        } finally {
            pane.lines.slice(0, 3).forEach((line, index) => {
                line.fold = original[index];
            });
        }
    });

    it("lists debuggee threads as selectable rows and marks the shown one", () => {
        mockSnapshot.debug = {
            running: false,
            executionLine: 1,
            stack: [],
            rows: [
                {
                    index: 4,
                    kind: "header",
                    depth: 0,
                    label: "Threads",
                    expandable: false,
                    expanded: false,
                    conditional: false,
                },
                {
                    index: 5,
                    kind: "thread",
                    depth: 1,
                    label: "main (1)",
                    value: "stopped here",
                    expandable: false,
                    expanded: false,
                    conditional: false,
                    selected: true,
                },
                {
                    index: 6,
                    kind: "thread",
                    depth: 1,
                    label: "worker (7)",
                    expandable: false,
                    expanded: false,
                    conditional: false,
                    selected: false,
                },
            ],
            output: [],
        };
        try {
            const { container } = render(() => <App />);
            const shown = screen.getByRole("treeitem", { name: /main \(1\)/ });
            expect(shown.getAttribute("aria-selected")).toBe("true");
            expect(shown.classList.contains("selected")).toBe(true);
            const other = screen.getByRole("treeitem", {
                name: /worker \(7\)/,
            });
            expect(other.getAttribute("aria-selected")).toBe("false");
            expect(other.getAttribute("title")).toBe(
                "Show this thread's stack",
            );
            expect(
                container.querySelectorAll(".debug-row.thread"),
            ).toHaveLength(2);
        } finally {
            delete mockSnapshot.debug;
        }
    });

    it("renders the selected language server detail projected by the core", () => {
        mockSnapshot.lspManager = {
            filter: "",
            selected: 4,
            showDetail: true,
            items: [
                {
                    index: 4,
                    language: "Rust",
                    section: "RUNNING",
                    command: "rust-analyzer",
                    state: "ready",
                    extensions: ["rs"],
                    rootMarkers: ["Cargo.toml"],
                    capabilities: ["hover", "completion"],
                },
            ],
        };

        try {
            render(() => <App />);
            const detail = screen.getByRole("complementary", {
                name: "Rust details",
            });
            expect(detail.textContent).toContain("rust-analyzer");
            expect(detail.textContent).toContain("Cargo.toml");
            expect(detail.textContent).toContain("hover, completion");
            expect(
                screen
                    .getByRole("button", { name: "Details" })
                    .getAttribute("aria-pressed"),
            ).toBe("true");
        } finally {
            delete mockSnapshot.lspManager;
        }
    });

    it("appends the enclosing symbols of the cursor to the breadcrumbs", () => {
        mockSnapshot.symbolBreadcrumbs = [
            { name: "Circle", kind: "class" },
            { name: "area()", kind: "method" },
        ];

        try {
            const result = render(() => <App />);
            const crumbs = result.container.querySelector(".breadcrumbs")!;
            const symbols = [...crumbs.querySelectorAll(".symbol-crumb")].map(
                (element) => element.textContent,
            );
            expect(symbols).toEqual(["Circle", "area()"]);
            expect(
                crumbs
                    .querySelector(".symbol-crumb")!
                    .getAttribute("data-kind"),
            ).toBe("class");
        } finally {
            mockSnapshot.symbolBreadcrumbs = [];
        }
    });

    it("renders the replace-in-files review projected by the core", () => {
        mockSnapshot.searchReplace = {
            find: "Circle",
            replace: "Round",
            files: "*.java",
            focus: "results",
            regex: false,
            caseSensitive: true,
            wholeWord: false,
            searching: false,
            searched: true,
            truncated: false,
            totalMatches: 2,
            checkedMatches: 1,
            fileCount: 1,
            selected: 1,
            rows: [
                {
                    index: 0,
                    kind: "file",
                    state: "partial",
                    path: "src/A.java",
                    line: 0,
                    count: 2,
                    before: "",
                    matched: "",
                    replacement: "",
                    after: "",
                },
                {
                    index: 1,
                    kind: "match",
                    state: "checked",
                    path: "src/A.java",
                    line: 3,
                    count: 0,
                    before: "new ",
                    matched: "Circle",
                    replacement: "Round",
                    after: "();",
                },
                {
                    index: 2,
                    kind: "match",
                    state: "unchecked",
                    path: "src/A.java",
                    line: 9,
                    count: 0,
                    before: "",
                    matched: "Circle",
                    replacement: "Round",
                    after: " c;",
                },
            ],
        };

        try {
            const result = render(() => <App />);
            const dialog = screen.getByRole("dialog", {
                name: "Replace in files",
            });
            expect(dialog.textContent).toContain("1 of 2 checked in 1 files");
            expect(dialog.textContent).toContain("Replace 1");
            expect(dialog.querySelectorAll("del")[0].textContent).toBe(
                "Circle",
            );
            expect(dialog.querySelectorAll("ins")[0].textContent).toBe("Round");
            const boxes = dialog.querySelectorAll<HTMLInputElement>(
                "input[type=checkbox]",
            );
            expect(boxes[0].indeterminate).toBe(true);
            expect(boxes[1].checked).toBe(true);
            expect(boxes[2].checked).toBe(false);
            expect(
                screen
                    .getByRole("button", { name: "Aa" })
                    .getAttribute("aria-pressed"),
            ).toBe("true");
            expect(
                result.container.querySelector(".sr-item.off"),
            ).not.toBeNull();
        } finally {
            delete mockSnapshot.searchReplace;
        }
    });

    it("exposes debugger execution controls and projected stop location", () => {
        mockSnapshot.debug = {
            running: false,
            executionLine: 27,
            stack: [],
            rows: [],
            output: [],
        };

        try {
            render(() => <App />);
            expect(screen.getByText("stopped at line 27")).toBeTruthy();
            expect(
                screen.getByRole("toolbar", { name: "Debug controls" }),
            ).toBeTruthy();
            expect(
                screen
                    .getByRole("button", { name: "Continue" })
                    .hasAttribute("disabled"),
            ).toBe(false);
            expect(
                screen.getByRole("button", { name: "Step over" }),
            ).toBeTruthy();
            expect(
                screen.getByRole("button", { name: "Step in" }),
            ).toBeTruthy();
            expect(
                screen.getByRole("button", { name: "Step out" }),
            ).toBeTruthy();
            expect(screen.getByRole("button", { name: "Stop" })).toBeTruthy();
        } finally {
            delete mockSnapshot.debug;
        }
    });

    it("lists variables, watches and breakpoints and marks breakpoint and execution lines in the gutter", () => {
        const first = mockSnapshot.lines[0];
        const second = mockSnapshot.lines[1];
        const previous = {
            first: first.breakpoint,
            executing: first.executing,
            second: second.breakpoint,
        };
        first.breakpoint = "enabled";
        first.executing = true;
        second.breakpoint = "disabled";
        mockSnapshot.debug = {
            running: false,
            executionLine: 1,
            stack: [],
            rows: [
                {
                    index: 3,
                    kind: "header",
                    depth: 0,
                    label: "Variables",
                    expandable: false,
                    expanded: false,
                    conditional: false,
                },
                {
                    index: 5,
                    kind: "variable",
                    depth: 1,
                    label: "user",
                    value: "User@1",
                    typeName: "User",
                    expandable: true,
                    expanded: true,
                    conditional: false,
                },
                {
                    index: 6,
                    kind: "variable",
                    depth: 2,
                    label: "name",
                    value: '"Ann"',
                    expandable: false,
                    expanded: false,
                    conditional: false,
                },
                {
                    index: 9,
                    kind: "breakpoint",
                    depth: 1,
                    label: "Main.java:12",
                    expandable: false,
                    expanded: false,
                    enabled: false,
                    conditional: false,
                },
            ],
            output: [],
        };

        try {
            const { container } = render(() => <App />);
            const user = screen.getByRole("treeitem", { name: /user/ });
            expect(user.getAttribute("aria-expanded")).toBe("true");
            expect(screen.getByText('"Ann"')).toBeTruthy();
            expect(screen.getByText("Main.java:12")).toBeTruthy();
            expect(screen.getByRole("button", { name: "Enable" })).toBeTruthy();
            expect(
                container.querySelector(".line-number.breakpoint-enabled"),
            ).toBeTruthy();
            expect(
                container.querySelector(".line-number.breakpoint-disabled"),
            ).toBeTruthy();
            expect(
                container.querySelector(".code-line.executing"),
            ).toBeTruthy();
        } finally {
            delete mockSnapshot.debug;
            first.breakpoint = previous.first;
            first.executing = previous.executing;
            second.breakpoint = previous.second;
        }
    });

    it("exposes test rerun and full-output actions without duplicating a running job", () => {
        mockSnapshot.testPanel = {
            scope: "nearest",
            command: "cargo test focused_case",
            directory: "ovim",
            status: "running",
            elapsedMs: 1200,
            truncated: 0,
            lines: [],
        };

        try {
            render(() => <App />);
            expect(screen.getByText("No test output yet")).toBeTruthy();
            expect(
                screen
                    .getByRole("button", { name: "Rerun" })
                    .hasAttribute("disabled"),
            ).toBe(true);
            expect(
                screen.getByRole("button", { name: "Full output" }),
            ).toBeTruthy();
            const fullOutput = screen.getByRole("button", {
                name: "Full output",
            });
            fullOutput.focus();
            fireEvent.keyDown(fullOutput, { key: "Escape" });
            expect(
                vi.mocked(HTMLElement.prototype.focus).mock.contexts.at(-1),
            ).toBe(screen.getByLabelText("Ovim editor input"));
        } finally {
            delete mockSnapshot.testPanel;
        }
    });

    it("keeps run output visible after the process exits, with exit code and jumpable frames", () => {
        mockSnapshot.runConsole = {
            title: "Main (app)",
            mode: "run",
            status: "failed",
            statusText: "failed (exit 1) in 2.3s",
            active: false,
            command: "java -cp out com.example.Main",
            exitCode: 1,
            elapsedMs: 2300,
            runIndex: 2,
            runCount: 3,
            truncated: 0,
            firstIndex: 0,
            lines: [
                { kind: "stdout", text: "hello", jumpable: false },
                {
                    kind: "stderr",
                    text: "\tat com.example.Main.main(Main.java:5)",
                    jumpable: true,
                },
                { kind: "system", text: "Process finished", jumpable: false },
            ],
        };

        try {
            render(() => <App />);
            expect(screen.getByText("failed (exit 1) in 2.3s")).toBeTruthy();
            expect(screen.getByText("exit code 1")).toBeTruthy();
            expect(screen.getByText("run 2 of 3")).toBeTruthy();
            const frame = screen.getByText(/Main\.main\(Main\.java:5\)/);
            expect(frame.classList.contains("stderr")).toBe(true);
            expect(frame.classList.contains("jumpable")).toBe(true);
            expect(
                screen.getByText("hello").classList.contains("jumpable"),
            ).toBe(false);
            // A finished run cannot be stopped, but can be rerun and cleared.
            expect(
                screen
                    .getByRole("button", { name: "Stop" })
                    .hasAttribute("disabled"),
            ).toBe(true);
            expect(
                screen
                    .getByRole("button", { name: "Rerun" })
                    .hasAttribute("disabled"),
            ).toBe(false);
            expect(
                screen
                    .getByRole("button", { name: "Clear" })
                    .hasAttribute("disabled"),
            ).toBe(false);
        } finally {
            delete mockSnapshot.runConsole;
        }
    });

    it("shows blocking chat setup inline with masked input and working actions", () => {
        const onKey = vi.fn();
        render(() => (
            <ChatSetupCard
                setup={{
                    kind: "exaKey",
                    title: "Enable web search",
                    detail: "Paste an Exa API key or skip this optional setup.",
                    maskedInput: "••••",
                    inputCursor: 2,
                    actions: [
                        { label: "Save key", key: "Enter" },
                        { label: "Not now", key: "Escape" },
                    ],
                }}
                onKey={onKey}
            />
        ));

        const input = screen.getByLabelText("Exa API key input");
        expect(input.textContent).toBe("••••");
        expect(
            input.querySelector(".chat-caret")?.previousSibling?.textContent,
        ).toBe("••");
        fireEvent.click(screen.getByRole("button", { name: "Not now" }));
        expect(onKey).toHaveBeenCalledWith("Escape");
    });

    it("recognizes a browser selection made inside the chat transcript", () => {
        const transcript = document.createElement("div");
        transcript.className = "chat-messages";
        transcript.textContent = "copy this response";
        document.body.append(transcript);
        const range = document.createRange();
        range.selectNodeContents(transcript);
        const selection = window.getSelection()!;
        selection.removeAllRanges();
        selection.addRange(range);

        expect(chatSelectionText(selection)).toBe("copy this response");

        transcript.className = "editor-content";
        expect(chatSelectionText(selection)).toBe("");
    });

    it("accepts only image formats supported by the core attachment path", () => {
        expect(imageExtension("image/png")).toBe("png");
        expect(imageExtension("image/jpeg")).toBe("jpg");
        expect(imageExtension("image/gif")).toBe("gif");
        expect(imageExtension("image/webp")).toBe("webp");
        expect(imageExtension("image/svg+xml")).toBeUndefined();
    });

    it("ignores standalone modifiers and treats option-produced Unicode as text", () => {
        expect(
            guiKeyInput({
                key: "Shift",
                shiftKey: true,
                ctrlKey: false,
                altKey: false,
                metaKey: false,
            }),
        ).toBeUndefined();
        expect(
            guiKeyInput({
                key: "›",
                shiftKey: true,
                ctrlKey: false,
                altKey: true,
                metaKey: false,
            }),
        ).toEqual({
            key: "›",
            shift: true,
            control: false,
            alt: false,
            meta: false,
        });
        expect(
            guiKeyInput({
                key: "b",
                shiftKey: false,
                ctrlKey: false,
                altKey: true,
                metaKey: false,
            })?.alt,
        ).toBe(true);
    });

    it("groups contiguous thinking and tool activity behind one live summary", async () => {
        const items = chatTranscriptItems(
            [
                {
                    id: "1:1",
                    index: 0,
                    selected: false,
                    role: "user",
                    content: "Please inspect this",
                    tools: [],
                },
                {
                    id: "1:2",
                    index: 1,
                    selected: false,
                    role: "thinking",
                    content: "**Planning the inspection**",
                    model: "codex",
                    tools: [],
                },
                {
                    id: "1:3",
                    index: 2,
                    selected: false,
                    role: "assistant",
                    content: "",
                    model: "codex",
                    tools: ["search_project"],
                },
                {
                    id: "1:4",
                    index: 3,
                    selected: false,
                    role: "tool",
                    content: "Found three matches",
                    toolName: "search_project",
                    tools: [],
                },
                {
                    id: "1:5",
                    index: 4,
                    selected: false,
                    role: "assistant",
                    content: "Here is the result",
                    model: "codex",
                    tools: [],
                },
            ],
            "Inspecting the matching files",
            true,
        );

        expect(items.map((item) => item.kind)).toEqual([
            "message",
            "activity",
            "message",
            "activity",
        ]);
        const live = items.at(-1)!;
        expect(live.kind).toBe("activity");
        if (live.kind !== "activity") throw new Error("expected live activity");
        expect(activitySummary(live.entries)).toBe(
            "Inspecting the matching files",
        );

        const result = render(() => <ChatActivityGroup item={live} />);
        expect(screen.getByText("Inspecting the matching files")).toBeTruthy();
        expect(screen.getByLabelText("Working")).toBeTruthy();
        expect(
            result.container.querySelector(".chat-activity-history"),
        ).toBeNull();
        const details =
            result.container.querySelector<HTMLDetailsElement>("details")!;
        details.open = true;
        fireEvent(details, new Event("toggle"));
        await Promise.resolve();
        expect(
            result.container.querySelector(".chat-activity-history"),
        ).toBeTruthy();
    });

    it("keeps the latest activity live between thinking chunks while work continues", () => {
        const items = chatTranscriptItems(
            [
                {
                    id: "1:1",
                    index: 0,
                    selected: false,
                    role: "thinking",
                    content: "**Planning**",
                    tools: [],
                },
                {
                    id: "1:2",
                    index: 1,
                    selected: false,
                    role: "assistant",
                    content: "",
                    tools: ["search_project"],
                },
                {
                    id: "1:3",
                    index: 2,
                    selected: false,
                    role: "tool",
                    content: "Found matches",
                    toolName: "search_project",
                    tools: [],
                },
            ],
            undefined,
            false,
            true,
        );

        const activity = items.at(-1);
        expect(activity?.kind).toBe("activity");
        if (activity?.kind !== "activity") throw new Error("expected activity");
        expect(activity.live).toBe(true);
        expect(activitySummary(activity.entries)).toBe("Planning");
    });

    it("keeps one activity group across tool-bearing assistant commentary", () => {
        const items = chatTranscriptItems(
            [
                {
                    id: "1:1",
                    index: 0,
                    selected: false,
                    role: "thinking",
                    content: "Planning",
                    tools: [],
                },
                {
                    id: "1:2",
                    index: 1,
                    selected: false,
                    role: "assistant",
                    content: "I’ll inspect the matching files.",
                    tools: ["search_project"],
                },
                {
                    id: "1:3",
                    index: 2,
                    selected: false,
                    role: "tool",
                    content: "Found matches",
                    toolName: "search_project",
                    tools: [],
                },
            ],
            "Comparing results",
            true,
            true,
        );
        expect(items.map((item) => item.kind)).toEqual(["activity", "message"]);
        const activity = items[0];
        if (activity.kind !== "activity") throw new Error("expected activity");
        expect(activity.entries.map((entry) => entry.role)).toEqual([
            "thinking",
            "tool",
            "thinking",
        ]);
        expect(activity.live).toBe(true);
    });

    it("retains unchanged transcript entries across streaming snapshots", async () => {
        const message: GuiAiChat["messages"][number] = {
            id: "1:1",
            index: 0,
            selected: false,
            role: "assistant",
            content: "A **stable** historical response",
            model: "codex",
            tools: [],
        };
        const previous = chatTranscriptItems([message]);
        const next = retainTranscriptItems(
            previous,
            chatTranscriptItems([{ ...message, tools: [] }]),
        );
        expect(next[0]).toBe(previous[0]);

        const initial: GuiAiChat = {
            profile: "codex",
            profiles: [],
            reasoningEffort: "medium",
            reasoningEffortSelection: "default",
            reasoningEfforts: ["default"],
            yoloMode: false,
            comprehensionPolicy: "off",
            activity: "idle",
            waiting: false,
            input: "",
            inputCursor: 0,
            queuedInputs: [],
            pendingImages: [],
            messages: [message],
            thinkingLive: false,
            focus: "textInput",
            agents: [],
            agentCursor: 0,
        };
        const [chat, setChat] = createSignal(initial);
        render(() => <ChatPanel chat={chat()} focusInput={() => {}} />);
        const historicalMarkup = screen.getByText("stable");

        setChat({
            ...initial,
            activity: "streaming",
            streaming: "A new live token",
        });
        await Promise.resolve();

        expect(screen.getByText("stable")).toBe(historicalMarkup);
    });

    it("keeps saved diff actions visible outside collapsed agent activity", () => {
        const onReplay = vi.fn();
        const onSelect = vi.fn();
        const message = {
            id: "1:3",
            index: 2,
            selected: false,
            role: "tool",
            content: "Saved refactor review",
            toolName: "show_custom_diff",
            replayToolCallId: "custom-diff-call",
            replayLabel: "Open diff",
            tools: [],
        };
        expect(chatTranscriptItems([message])[0].kind).toBe("message");
        render(() => (
            <ChatMessageView
                message={message}
                onReplay={onReplay}
                onSelect={onSelect}
            />
        ));
        fireEvent.click(screen.getByRole("button", { name: "Open diff" }));
        expect(onReplay).toHaveBeenCalledWith("custom-diff-call");
        expect(onSelect).not.toHaveBeenCalled();
        expect(screen.queryByText(message.content)).toBeNull();
    });

    it("keeps tool results collapsed until their details are requested", async () => {
        const payload = "large tool payload that should start hidden";
        const result = render(() => (
            <ChatMessageView
                message={{
                    id: "1:3",
                    index: 2,
                    selected: false,
                    role: "tool",
                    content: payload,
                    toolName: "search_project",
                    tools: [],
                }}
            />
        ));

        expect(screen.getByText("search_project")).toBeTruthy();
        expect(screen.getByText(toolResultSummary(payload))).toBeTruthy();
        expect(result.container.querySelector(".markdown")).toBeNull();

        const details =
            result.container.querySelector<HTMLDetailsElement>("details")!;
        details.open = true;
        fireEvent(details, new Event("toggle"));
        await Promise.resolve();
        expect(screen.getByText(payload)).toBeTruthy();
    });

    it("collapses assistant tool-call lists by default", () => {
        const result = render(() => (
            <ChatMessageView
                message={{
                    id: "1:4",
                    index: 3,
                    selected: false,
                    role: "assistant",
                    content: "I will inspect this.",
                    model: "codex",
                    tools: ["search_project", "read_file_at_path"],
                }}
            />
        ));

        const details =
            result.container.querySelector<HTMLDetailsElement>(
                ".tool-call-list",
            )!;
        expect(details.open).toBe(false);
        expect(screen.getByText("2 tool calls")).toBeTruthy();
    });
    it("confirms application quit while unsaved buffers exist", async () => {
        const result = render(() => <App />);
        fireEvent.keyDown(window, { key: "q", metaKey: true });
        expect(
            screen.getByRole("dialog", {
                name: "Save changes before leaving?",
            }),
        ).toBeTruthy();
        expect(
            screen.getByRole("button", { name: "Save All and Quit" }),
        ).toBeTruthy();
        expect(
            screen.getByRole("button", { name: "Quit Without Saving" }),
        ).toBeTruthy();
        fireEvent.keyDown(screen.getByRole("dialog"), { key: "Escape" });
        expect(
            screen.queryByRole("dialog", {
                name: "Save changes before leaving?",
            }),
        ).toBeNull();
        result.unmount();
    });
});
