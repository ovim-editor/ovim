import { describe, expect, it, vi } from "vitest";
import {
    createMouseChannel,
    pointerCell,
    trackSelectionDrag,
    type GuiMouseMessage,
    type PointerCell,
    type PointerRow,
} from "./editorPointer";

const rows: PointerRow[] = [0, 1, 2].map((index) => ({
    number: index + 10,
    displayStart: 0,
    top: 100 + index * 22,
    bottom: 122 + index * 22,
    contentLeft: 50,
}));

describe("pointerCell", () => {
    it("maps a coordinate to a 0-based line and display column", () => {
        expect(pointerCell(rows, 50 + 8 * 3 + 2, 125, 8)).toEqual({
            line: 10,
            displayColumn: 3,
            outside: undefined,
        });
    });

    it("clamps left of the text and reports rows above or below as outside", () => {
        expect(pointerCell(rows, 10, 101, 8)).toMatchObject({
            line: 9,
            displayColumn: 0,
        });
        expect(pointerCell(rows, 60, 10, 8)).toMatchObject({
            line: 9,
            outside: "above",
        });
        expect(pointerCell(rows, 60, 900, 8)).toMatchObject({
            line: 11,
            outside: "below",
        });
    });

    it("adds a soft-wrapped continuation row's display offset", () => {
        const wrapped = [{ ...rows[0], displayStart: 40 }];
        expect(pointerCell(wrapped, 50 + 16, 110, 8)?.displayColumn).toBe(42);
    });

    it("has no cell without rendered rows or a measured cell width", () => {
        expect(pointerCell([], 0, 0, 8)).toBeUndefined();
        expect(pointerCell(rows, 0, 0, 0)).toBeUndefined();
    });
});

describe("createMouseChannel", () => {
    const message = (
        kind: GuiMouseMessage["kind"],
        line = 0,
    ): GuiMouseMessage => ({
        pane: 0,
        line,
        displayColumn: 0,
        kind,
        clickCount: 1,
        shift: false,
    });

    it("delivers in order and collapses queued drags into the latest", async () => {
        const posted: string[] = [];
        let release!: () => void;
        const first = new Promise<void>((resolve) => (release = resolve));
        const channel = createMouseChannel(async (m) => {
            posted.push(`${m.kind}:${m.line}`);
            if (m.kind === "down") await first;
        });
        channel.send(message("down"));
        channel.send(message("drag", 1));
        channel.send(message("drag", 2));
        channel.send(message("drag", 3));
        channel.send(message("up", 3));
        release();
        await channel.idle();
        expect(posted).toEqual(["down:0", "up:3"]);
    });

    it("keeps the latest drag when it is not followed by a release", async () => {
        const posted: string[] = [];
        let release!: () => void;
        const first = new Promise<void>((resolve) => (release = resolve));
        const channel = createMouseChannel(async (m) => {
            posted.push(`${m.kind}:${m.line}`);
            if (m.kind === "down") await first;
        });
        channel.send(message("down"));
        channel.send(message("drag", 1));
        channel.send(message("drag", 2));
        release();
        await channel.idle();
        expect(posted).toEqual(["down:0", "drag:2"]);
    });

    it("survives a failing post", async () => {
        const post = vi
            .fn<(m: GuiMouseMessage) => Promise<unknown>>()
            .mockRejectedValueOnce(new Error("boom"))
            .mockResolvedValue(undefined);
        const channel = createMouseChannel(post);
        channel.send(message("down"));
        channel.send(message("up"));
        await channel.idle();
        expect(post).toHaveBeenCalledTimes(2);
    });
});

describe("trackSelectionDrag", () => {
    class FakeWindow {
        private listeners = new Map<string, Set<(event: Event) => void>>();
        addEventListener(type: string, listener: (event: Event) => void) {
            const set = this.listeners.get(type) ?? new Set();
            set.add(listener);
            this.listeners.set(type, set);
        }
        removeEventListener(type: string, listener: (event: Event) => void) {
            this.listeners.get(type)?.delete(listener);
        }
        emit(type: string, init: Partial<MouseEvent> = {}) {
            for (const listener of this.listeners.get(type) ?? [])
                listener(init as Event);
        }
        count() {
            return [...this.listeners.values()].reduce(
                (sum, set) => sum + set.size,
                0,
            );
        }
    }

    const setup = (
        cellAt: (x: number, y: number) => PointerCell | undefined,
    ) => {
        const target = new FakeWindow();
        const sent: GuiMouseMessage[] = [];
        let frame: (() => void) | undefined;
        let repeat: (() => void) | undefined;
        const stop = trackSelectionDrag({
            pane: 1,
            start: { line: 4, displayColumn: 2, clickCount: 2, shift: true },
            cellAt,
            channel: { send: (m) => sent.push(m) },
            target: target as unknown as Window,
            requestFrame: (callback) => {
                frame = callback;
                return 1;
            },
            cancelFrame: () => (frame = undefined),
            setRepeat: (callback) => {
                repeat = callback;
                return 2;
            },
            clearRepeat: () => (repeat = undefined),
        });
        return {
            target,
            sent,
            stop,
            runFrame: () => {
                const run = frame;
                frame = undefined;
                run?.();
            },
            runRepeat: () => repeat?.(),
            repeating: () => repeat !== undefined,
        };
    };

    it("throttles moves to one drag per frame and carries click count and shift", () => {
        const t = setup((x, y) => ({ line: y, displayColumn: x }));
        t.target.emit("mousemove", { buttons: 1, clientX: 5, clientY: 6 });
        t.target.emit("mousemove", { buttons: 1, clientX: 7, clientY: 8 });
        expect(t.sent).toEqual([]);
        t.runFrame();
        expect(t.sent).toEqual([
            {
                pane: 1,
                line: 8,
                displayColumn: 7,
                kind: "drag",
                clickCount: 2,
                shift: true,
            },
        ]);
    });

    it("does not resend an unchanged cell", () => {
        const t = setup(() => ({ line: 4, displayColumn: 2 }));
        t.target.emit("mousemove", { buttons: 1, clientX: 5, clientY: 6 });
        t.runFrame();
        expect(t.sent).toEqual([]);
    });

    it("releases at the final cell, even outside the pane, and stops listening", () => {
        const t = setup((x, y) => ({ line: y, displayColumn: x }));
        t.target.emit("mousemove", { buttons: 1, clientX: 1, clientY: 9 });
        t.target.emit("mouseup", { button: 0, clientX: 3, clientY: 12 });
        expect(t.sent.map((m) => [m.kind, m.line, m.displayColumn])).toEqual([
            ["up", 12, 3],
        ]);
        expect(t.target.count()).toBe(0);
        t.target.emit("mouseup", { button: 0, clientX: 3, clientY: 13 });
        expect(t.sent).toHaveLength(1);
    });

    it("releases past the rows at a move that was still waiting for its frame", () => {
        const t = setup((x, y) =>
            y < 0
                ? { line: 0, displayColumn: x, outside: "above" }
                : { line: y, displayColumn: x },
        );
        t.target.emit("mousemove", { buttons: 1, clientX: 3, clientY: -5 });
        t.target.emit("mouseup", { button: 0, clientX: 3, clientY: -9 });
        expect(t.sent.map((m) => [m.kind, m.line])).toEqual([
            ["drag", 0],
            ["up", 0],
        ]);
        expect(t.repeating()).toBe(false);
    });

    it("ignores other buttons' releases and ends when the button was lost", () => {
        const t = setup((x, y) => ({ line: y, displayColumn: x }));
        t.target.emit("mouseup", { button: 2, clientX: 1, clientY: 1 });
        expect(t.sent).toEqual([]);
        t.target.emit("mousemove", { buttons: 0, clientX: 6, clientY: 7 });
        expect(t.sent.map((m) => m.kind)).toEqual(["up"]);
        expect(t.target.count()).toBe(0);
    });

    it("keeps dragging past the edge while the pointer rests outside", () => {
        let cell: PointerCell = {
            line: 20,
            displayColumn: 4,
            outside: "below",
        };
        const t = setup(() => cell);
        t.target.emit("mousemove", { buttons: 1, clientX: 5, clientY: 999 });
        t.runFrame();
        expect(t.repeating()).toBe(true);
        t.runRepeat();
        expect(t.sent.map((m) => m.line)).toEqual([20, 21]);
        cell = { line: 18, displayColumn: 4 };
        t.target.emit("mousemove", { buttons: 1, clientX: 5, clientY: 5 });
        t.runFrame();
        expect(t.repeating()).toBe(false);
        t.stop();
        expect(t.sent.at(-1)?.kind).toBe("up");
    });
});
