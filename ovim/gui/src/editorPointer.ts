// Pointer selection plumbing for the editor text: the DOM only reports which
// buffer cell is under the pointer; Ovim core owns what a press, drag or
// multi-click selects (`gui_mouse`).

export type GuiMouseKind = "down" | "drag" | "up";

/** The `gui_mouse` command arguments (Tauri maps the camelCase keys to Rust). */
export interface GuiMouseMessage {
    pane: number;
    /** 0-based buffer line. */
    line: number;
    displayColumn: number;
    kind: GuiMouseKind;
    clickCount: number;
    shift: boolean;
}

export interface PointerRow {
    /** 1-based buffer line number of the rendered row. */
    number: number;
    /** Display column of the row's first cell (soft-wrap continuations). */
    displayStart: number;
    top: number;
    bottom: number;
    /** Left edge of the row's text: display column `displayStart` starts here. */
    contentLeft: number;
}

export interface PointerCell {
    line: number;
    displayColumn: number;
    /** The pointer is above the first or below the last rendered row. */
    outside?: "above" | "below";
}

/** The buffer cell under a viewport coordinate, clamped to the rendered rows. */
export function pointerCell(
    rows: readonly PointerRow[],
    x: number,
    y: number,
    cellWidth: number,
): PointerCell | undefined {
    if (rows.length === 0 || !(cellWidth > 0)) return undefined;
    let outside: PointerCell["outside"];
    let row: PointerRow;
    if (y < rows[0].top) {
        row = rows[0];
        outside = "above";
    } else {
        const found = rows.find((candidate) => y < candidate.bottom);
        row = found ?? rows[rows.length - 1];
        if (!found) outside = "below";
    }
    const column = Math.max(0, Math.floor((x - row.contentLeft) / cellWidth));
    return {
        line: row.number - 1,
        displayColumn: row.displayStart + column,
        outside,
    };
}

/**
 * Sends `gui_mouse` messages in order, one at a time. Presses and releases are
 * never dropped; drags that pile up while one is in flight collapse into the
 * latest, so a slow editor never builds a backlog of stale positions.
 */
export function createMouseChannel(
    post: (message: GuiMouseMessage) => Promise<unknown>,
) {
    let chain: Promise<unknown> = Promise.resolve();
    let pendingDrag: GuiMouseMessage | undefined;
    const enqueue = (message: GuiMouseMessage) => {
        chain = chain.then(() => post(message)).catch(() => {});
    };
    return {
        send(message: GuiMouseMessage) {
            if (message.kind !== "drag") {
                pendingDrag = undefined;
                enqueue(message);
                return;
            }
            const queued = pendingDrag !== undefined;
            pendingDrag = message;
            if (queued) return;
            chain = chain
                .then(() => {
                    const next = pendingDrag;
                    pendingDrag = undefined;
                    return next ? post(next) : undefined;
                })
                .catch(() => {});
        },
        /** Resolves once everything sent so far has been delivered. */
        idle: () => chain,
    };
}

export interface SelectionDragOptions {
    pane: number;
    start: PointerCell & { clickCount: number; shift: boolean };
    /** Cell under a viewport coordinate right now (the DOM changes as the view scrolls). */
    cellAt: (x: number, y: number) => PointerCell | undefined;
    channel: Pick<ReturnType<typeof createMouseChannel>, "send">;
    /** Window-like event source; tests pass their own. */
    target?: Pick<Window, "addEventListener" | "removeEventListener">;
    requestFrame?: (callback: () => void) => number;
    cancelFrame?: (handle: number) => void;
    /** Repeats a drag while the pointer rests outside the rows (auto-scroll). */
    setRepeat?: (callback: () => void, ms: number) => number;
    clearRepeat?: (handle: number) => void;
}

/**
 * Tracks one primary-button gesture from press to release. Listens on the
 * window so the drag keeps following the pointer outside the pane (a held
 * button routes mouse events to the page even over other elements), throttles
 * moves to one message per animation frame, and ends on release, a lost
 * button, or window blur. Returns a function that ends the gesture early.
 */
export function trackSelectionDrag(options: SelectionDragOptions): () => void {
    const target = options.target ?? window;
    const requestFrame =
        options.requestFrame ?? ((cb) => requestAnimationFrame(cb));
    const cancelFrame = options.cancelFrame ?? ((h) => cancelAnimationFrame(h));
    const setRepeat =
        options.setRepeat ?? ((cb, ms) => window.setInterval(cb, ms));
    const clearRepeat =
        options.clearRepeat ?? ((handle) => window.clearInterval(handle));
    const { pane, channel } = options;
    const { clickCount, shift } = options.start;

    let last: PointerCell = options.start;
    let latest: { x: number; y: number } | undefined;
    let frame: number | undefined;
    let repeat: number | undefined;
    let ended = false;

    const message = (
        kind: GuiMouseKind,
        cell: PointerCell,
    ): GuiMouseMessage => ({
        pane,
        line: cell.line,
        displayColumn: cell.displayColumn,
        kind,
        clickCount,
        shift,
    });
    const stopRepeat = () => {
        if (repeat !== undefined) clearRepeat(repeat);
        repeat = undefined;
    };
    // Beyond the first/last row, keep dragging one line further each tick so
    // the editor scrolls while the pointer rests there.
    const autoscroll = () => {
        if (ended || !latest) return;
        const cell = options.cellAt(latest.x, latest.y);
        if (!cell?.outside) return stopRepeat();
        const line = Math.max(
            0,
            cell.line + (cell.outside === "above" ? -1 : 1),
        );
        last = { ...cell, line };
        channel.send(message("drag", last));
    };
    const flush = () => {
        frame = undefined;
        if (ended || !latest) return;
        const cell = options.cellAt(latest.x, latest.y);
        if (!cell) return;
        if (cell.outside) {
            repeat ??= setRepeat(autoscroll, 70);
        } else {
            stopRepeat();
        }
        if (
            cell.line === last.line &&
            cell.displayColumn === last.displayColumn
        )
            return;
        last = cell;
        channel.send(message("drag", cell));
    };
    const finish = (cell: PointerCell | undefined) => {
        if (ended) return;
        // Past the rows the release lands on the last delivered cell, so a
        // move still waiting for its frame is delivered first.
        if (frame !== undefined && (!cell || cell.outside)) {
            cancelFrame(frame);
            flush();
        }
        ended = true;
        if (frame !== undefined) cancelFrame(frame);
        stopRepeat();
        target.removeEventListener("mousemove", onMove, true);
        target.removeEventListener("mouseup", onUp, true);
        target.removeEventListener("blur", onBlur);
        // Past the rows the last drag already scrolled to the right line.
        channel.send(message("up", cell && !cell.outside ? cell : last));
    };
    const onMove = (event: Event) => {
        const mouse = event as MouseEvent;
        if ((mouse.buttons & 1) === 0) {
            // The release happened where this page could not see it.
            finish(options.cellAt(mouse.clientX, mouse.clientY));
            return;
        }
        latest = { x: mouse.clientX, y: mouse.clientY };
        frame ??= requestFrame(flush);
    };
    const onUp = (event: Event) => {
        const mouse = event as MouseEvent;
        if (mouse.button !== 0) return;
        finish(options.cellAt(mouse.clientX, mouse.clientY));
    };
    const onBlur = () => finish(undefined);
    target.addEventListener("mousemove", onMove, true);
    target.addEventListener("mouseup", onUp, true);
    target.addEventListener("blur", onBlur);
    return () => finish(undefined);
}
