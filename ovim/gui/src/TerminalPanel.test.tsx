/** @vitest-environment jsdom */

import {
    cleanup,
    fireEvent,
    render,
    screen,
    waitFor,
} from "@solidjs/testing-library";
import { createSignal, Show } from "solid-js";
import { Channel, invoke } from "@tauri-apps/api/core";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import TerminalPanel, { cssColorToHex } from "./TerminalPanel";

const terminal = vi.hoisted(() => ({
    writes: [] as Uint8Array[],
    disposed: 0,
    input: undefined as ((data: string) => void) | undefined,
}));

vi.mock("@xterm/xterm", () => ({
    Terminal: class {
        cols = 80;
        rows = 24;
        constructor(_options: unknown) {}
        loadAddon() {}
        open() {}
        reset() {}
        focus() {}
        dispose() {
            terminal.disposed += 1;
        }
        write(data: Uint8Array, callback: () => void) {
            terminal.writes.push(data);
            callback();
        }
        onData(callback: (data: string) => void) {
            terminal.input = callback;
            return {
                dispose() {
                    terminal.input = undefined;
                },
            };
        }
    },
}));

vi.mock("@xterm/addon-fit", () => ({
    FitAddon: class {
        fit() {}
    },
}));
vi.mock("@tauri-apps/api/core", () => ({
    Channel: class {
        onmessage?: (event: unknown) => void;
    },
    invoke: vi.fn(),
}));

class ResizeObserverMock {
    observe() {}
    disconnect() {}
}

beforeEach(() => {
    vi.stubGlobal("ResizeObserver", ResizeObserverMock);
    vi.mocked(invoke).mockResolvedValue(1);
    terminal.writes = [];
    terminal.disposed = 0;
});

afterEach(() => {
    cleanup();
    vi.clearAllMocks();
    vi.unstubAllGlobals();
});

describe("TerminalPanel", () => {
    it("explains native availability in the web preview", () => {
        render(() => (
            <TerminalPanel native={false} active onClose={() => {}} />
        ));
        expect(
            screen.getByText("Terminal is available in the desktop app."),
        ).toBeTruthy();
        expect(invoke).not.toHaveBeenCalled();
    });

    it("streams output with backpressure and closes its session on unmount", async () => {
        const result = render(() => (
            <TerminalPanel native active onClose={() => {}} />
        ));
        await waitFor(() =>
            expect(invoke).toHaveBeenCalledWith(
                "gui_terminal_open",
                expect.any(Object),
            ),
        );
        const open = vi
            .mocked(invoke)
            .mock.calls.find(([command]) => command === "gui_terminal_open")!;
        const channel = (open[1] as { onEvent: Channel<unknown> }).onEvent;

        channel.onmessage?.({ type: "data", id: 1, data: [65, 66] });
        expect([...terminal.writes[0]]).toEqual([65, 66]);
        expect(invoke).toHaveBeenCalledWith("gui_terminal_ack", {
            id: 1,
            bytes: 2,
        });

        await waitFor(() => expect(screen.getByText("running")).toBeTruthy());
        terminal.input?.("echo hello\r");
        await waitFor(() =>
            expect(invoke).toHaveBeenCalledWith("gui_terminal_write", {
                id: 1,
                data: "echo hello\r",
            }),
        );

        result.unmount();
        await waitFor(() =>
            expect(invoke).toHaveBeenCalledWith("gui_terminal_close", {
                id: 1,
            }),
        );
        expect(terminal.disposed).toBe(1);
    });

    it("removes the surface on shell exit and reopens with a fresh session", async () => {
        const [open, setOpen] = createSignal(true);
        render(() => (
            <Show when={open()}>
                <TerminalPanel native active onClose={() => setOpen(false)} />
            </Show>
        ));
        await waitFor(() => expect(screen.getByText("running")).toBeTruthy());
        const request = vi
            .mocked(invoke)
            .mock.calls.find(([command]) => command === "gui_terminal_open")!;
        (request[1] as { onEvent: Channel<unknown> }).onEvent.onmessage?.({
            type: "exit",
            id: 1,
            code: 0,
        });
        expect(screen.queryByRole("region", { name: "Terminal" })).toBeNull();
        setOpen(true);
        await waitFor(() => expect(screen.getByText("running")).toBeTruthy());
        expect(
            vi
                .mocked(invoke)
                .mock.calls.filter(
                    ([command]) => command === "gui_terminal_open",
                ),
        ).toHaveLength(2);
        fireEvent.click(screen.getByRole("button", { name: "Close terminal" }));
        await waitFor(() =>
            expect(invoke).toHaveBeenCalledWith("gui_terminal_close", {
                id: 1,
            }),
        );
    });

    it("serializes a rapid close/reopen while the first launch is pending", async () => {
        let resolveOpen!: (id: number) => void;
        vi.mocked(invoke).mockImplementation((command) =>
            command === "gui_terminal_open"
                ? new Promise<number>((resolve) => {
                      resolveOpen = resolve;
                  })
                : Promise.resolve(),
        );
        const [open, setOpen] = createSignal(true);
        render(() => (
            <Show when={open()}>
                <TerminalPanel native active onClose={() => setOpen(false)} />
            </Show>
        ));
        const opens = () =>
            vi
                .mocked(invoke)
                .mock.calls.filter(
                    ([command]) => command === "gui_terminal_open",
                );
        await waitFor(() => expect(opens()).toHaveLength(1));
        setOpen(false);
        setOpen(true);
        expect(opens()).toHaveLength(1);
        resolveOpen(7);
        await waitFor(() => expect(opens()).toHaveLength(2));
        const calls = vi.mocked(invoke).mock.calls;
        expect(
            calls.findIndex(([command]) => command === "gui_terminal_close"),
        ).toBeLessThan(calls.lastIndexOf(opens()[1]));
        resolveOpen(8);
        await waitFor(() => expect(screen.getByText("running")).toBeTruthy());
    });

    it("keeps failures visible with a working restart action", async () => {
        vi.mocked(invoke).mockRejectedValueOnce("Shell unavailable");
        render(() => <TerminalPanel native active onClose={() => {}} />);
        await screen.findByText("Shell unavailable");
        fireEvent.click(
            screen.getByRole("button", { name: "Restart terminal" }),
        );
        await waitFor(() => expect(screen.getByText("running")).toBeTruthy());
    });

    it("paces large Unicode pastes without splitting characters", async () => {
        let releaseWrite!: () => void;
        vi.mocked(invoke).mockImplementation((command) =>
            command === "gui_terminal_write"
                ? new Promise<void>((resolve) => {
                      releaseWrite = resolve;
                  })
                : Promise.resolve(1),
        );
        render(() => <TerminalPanel native active onClose={() => {}} />);
        await waitFor(() => expect(screen.getByText("running")).toBeTruthy());
        const paste = "漢".repeat(16 * 1024 - 1) + "🙂終";
        terminal.input?.(paste);
        const writes = () =>
            vi
                .mocked(invoke)
                .mock.calls.filter(
                    ([command]) => command === "gui_terminal_write",
                );
        await waitFor(() => expect(writes()).toHaveLength(1));
        expect((writes()[0][1] as { data: string }).data).toBe(
            "漢".repeat(16 * 1024 - 1),
        );
        releaseWrite();
        await waitFor(() => expect(writes()).toHaveLength(2));
        expect((writes()[1][1] as { data: string }).data).toBe("🙂終");
        releaseWrite();
    });

    it("closes on exit even before open resolves and releases its session", async () => {
        let resolveOpen!: (id: number) => void;
        vi.mocked(invoke).mockImplementation((command) =>
            command === "gui_terminal_open"
                ? new Promise<number>((resolve) => {
                      resolveOpen = resolve;
                  })
                : Promise.resolve(),
        );
        const onClose = vi.fn();
        render(() => <TerminalPanel native active onClose={onClose} />);
        await waitFor(() =>
            expect(invoke).toHaveBeenCalledWith(
                "gui_terminal_open",
                expect.any(Object),
            ),
        );
        const open = vi
            .mocked(invoke)
            .mock.calls.find(([command]) => command === "gui_terminal_open")!;
        (open[1] as { onEvent: Channel<unknown> }).onEvent.onmessage?.({
            type: "exit",
            id: 7,
            code: 2,
        });
        resolveOpen(7);

        await waitFor(() =>
            expect(invoke).toHaveBeenCalledWith("gui_terminal_close", {
                id: 7,
            }),
        );
        expect(onClose).toHaveBeenCalledOnce();
        expect(screen.queryByText("exited")).toBeNull();
    });

    it("closes a shell whose open response arrives after unmount", async () => {
        let resolveOpen!: (id: number) => void;
        vi.mocked(invoke).mockImplementation((command) =>
            command === "gui_terminal_open"
                ? new Promise<number>((resolve) => {
                      resolveOpen = resolve;
                  })
                : Promise.resolve(),
        );
        const result = render(() => (
            <TerminalPanel native active onClose={() => {}} />
        ));
        await waitFor(() =>
            expect(invoke).toHaveBeenCalledWith(
                "gui_terminal_open",
                expect.any(Object),
            ),
        );
        result.unmount();
        resolveOpen(9);
        await waitFor(() =>
            expect(invoke).toHaveBeenCalledWith("gui_terminal_close", {
                id: 9,
            }),
        );
        expect(terminal.disposed).toBe(1);
    });
});

describe("cssColorToHex", () => {
    it("converts computed colours to the hex xterm.js understands", () => {
        expect(cssColorToHex("rgb(26, 27, 38)")).toBe("#1a1b26");
        expect(cssColorToHex("rgba(251, 241, 199, 1)")).toBe("#fbf1c7");
        expect(cssColorToHex("rgb(60 56 54 / 0.5)")).toBe("#3c3836");
        expect(cssColorToHex("#123456")).toBe("#123456");
        // Transparent or unknown values keep xterm's own default.
        expect(cssColorToHex("rgba(0, 0, 0, 0)")).toBeUndefined();
        expect(cssColorToHex("canvastext")).toBeUndefined();
    });
});
