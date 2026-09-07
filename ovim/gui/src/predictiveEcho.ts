import type {
    GuiConnection,
    GuiKeyInput,
    GuiLine,
    GuiSegment,
    GuiSnapshot,
} from "./types";

/**
 * Speculative local echo, in the shape mosh gave it.
 *
 * When the editor is on another host every keystroke round-trips before the
 * character appears, and a modal editor is keystroke-dense enough that the
 * delay stops being a delay and starts being the interface. The cure is to
 * draw the character locally, mark it as unconfirmed, and let the next
 * authoritative frame absorb it or overrule it.
 *
 * The whole design is bounded by one asymmetry: **showing text that is not
 * there costs more than showing text late**. Lag is annoying and honest; a
 * wrong character is a lie about a file, and a user who has seen one cannot
 * trust the rest. Every rule below therefore fails towards "draw nothing".
 *
 * Two facts on the frame make that possible, and neither could be worked out
 * from the projection alone:
 *
 * * `predictableInsert` -- the editor's own answer to "is the next plain
 *   printable character certain to be inserted literally?". A pending
 *   `i_CTRL-R`, half a typed mapping and the modal consent dialogs all change
 *   what a character means without changing a single rendered cell, so the
 *   editor has to say. See `ovim/src/gui/echo.rs`.
 * * `inputEpoch` -- how many commands the editor has taken in. Subtracting it
 *   from the count this client has sent gives the run the frame does not yet
 *   account for, which is exactly the run worth speaking for. It is mosh's
 *   acknowledged state number, and without it a client cannot tell a frame
 *   that already reflects its typing from one that predates it -- which is the
 *   difference between drawing a character in the right place and drawing it
 *   in the wrong one. It counts every client and every command, so it can only
 *   overstate what the editor has consumed of this one's typing, and
 *   overstating leaves less outstanding: the safe direction to be wrong in.
 *
 * The run is therefore *derived* on every frame rather than carried across
 * frames and patched. There is no rebase step, because there is nothing to
 * rebase: either the newest frame certifies a state whose outstanding keys are
 * all plain characters, and the run follows from it, or it does not and
 * nothing is drawn.
 */

/**
 * How long a keystroke may stand unconfirmed before it is presumed lost.
 *
 * A key whose answer never comes -- a dropped frame, a session that went quiet,
 * a bug -- must not leave a character on screen for the rest of the session.
 * One second is far longer than any link this feature is for, and far shorter
 * than a user would spend wondering.
 */
export const PREDICTION_LIFETIME_MS = 1000;

/**
 * The longest run this will ever draw ahead of the editor.
 *
 * Reached only when the far side has gone quiet without the link noticing, and
 * by then the expiry above is about to fire anyway. The cap keeps the failure
 * to a word rather than a paragraph.
 */
export const MAX_OUTSTANDING_PREDICTIONS = 32;

/** The mode string the editor projects while inserting. */
const INSERT_MODE = "INSERT";

/**
 * Typing one of these on an otherwise blank line makes Ovim re-indent the line
 * (`electric_dedent_close_bracket` in `input/helpers.rs`), which moves text a
 * client cannot compute. The geometry is checked rather than the character
 * alone, so `)` stays predictable everywhere it is only a character.
 */
const ELECTRIC_CLOSERS = new Set(["}", ")", "]"]);

/** One key on its way to the editor. */
export interface SentKey {
    /**
     * What it will insert, or `undefined` for a key whose meaning this client
     * does not model -- which is almost all of them.
     */
    character?: string;
    sentAt: number;
}

/** Where an outstanding run is being drawn. */
interface EchoAnchor {
    /** The buffer line, as the frame that certified the run reported it. */
    line: number;
    column: number;
}

/** A run being drawn ahead of the editor, and where it starts. */
interface EchoRun {
    anchor: EchoAnchor;
    keys: SentKey[];
}

export interface EchoState {
    /**
     * The keys sent recently, oldest first.
     *
     * Trimmed to what could still be outstanding: a longer run is refused
     * anyway, so remembering more would only be a leak.
     */
    journal: SentKey[];
    /**
     * What `inputEpoch` will read once the editor has taken in every key this
     * client has sent. `undefined` until the first frame gives it a starting
     * point.
     */
    sentEpoch?: number;
    /** The run currently drawn. Absent when nothing is being said. */
    run?: EchoRun;
}

/** Nothing sent, nothing outstanding, nothing being spoken for. */
export const noPredictions: EchoState = { journal: [] };

export interface EchoOptions {
    /** Whether keystrokes cross a network to reach the editor. */
    remote: boolean;
    connection: GuiConnection;
    now: number;
}

export interface RecordOutcome {
    state: EchoState;
    /** Whether the character went on screen without waiting for the editor. */
    predicted: boolean;
}

export interface ReconcileOutcome {
    state: EchoState;
    /** How many drawn characters this frame accounted for. */
    confirmed: number;
    /**
     * The frame left nothing that could be spoken for while a run was on
     * screen, so the run went and the authority is rendered as it arrived.
     * Above zero during ordinary typing means the safe set is too wide.
     */
    dropped: boolean;
}

const runText = (run: EchoRun) =>
    run.keys.map((key) => key.character ?? "").join("");

/**
 * The single rendered row of one buffer line, if it has one.
 *
 * A wrapped line has several rows and a scrolled one starts part-way through,
 * and in both cases the rendered offsets stop being the buffer's columns --
 * which is all this module does arithmetic on.
 */
const renderedLine = (snapshot: GuiSnapshot, line: number) => {
    const rows = snapshot.lines.filter((row) => row.number === line + 1);
    if (rows.length !== 1 || rows[0].continuation || rows[0].displayStart !== 0)
        return undefined;
    return rows[0];
};

const lineText = (line: GuiLine) =>
    line.segments.map((segment) => segment.text).join("");

/**
 * Whether a line's columns, code points and cells are the same thing.
 *
 * Double-width glyphs and multi-code-point graphemes each break that equality,
 * and with it every offset computed here.
 */
const linesUpWithItsColumns = (line: GuiLine) =>
    line.segments.every(
        (segment) => [...segment.text].length === segment.cells,
    );

/**
 * Whether the text before the cursor occupies one cell per column.
 *
 * A tab survives {@link linesUpWithItsColumns} because the editor projects it
 * as the spaces it fills -- one buffer column arriving as four rendered ones --
 * so the segments alone cannot tell it from four spaces. The cursor can: the
 * grapheme column and the display column are sent separately, and a tab is
 * exactly what makes them disagree.
 */
const columnsAreCells = (snapshot: GuiSnapshot) =>
    snapshot.cursor.displayColumn === snapshot.cursor.column;

/**
 * Whether this character will certainly be inserted as itself.
 *
 * ASCII printables plus the Latin range up to the combining marks: every code
 * point admitted here is one column wide and stands on its own, which is what
 * the column arithmetic assumes. Anything else -- CJK, emoji, a decomposed
 * accent -- round-trips.
 */
const isPredictableCharacter = (key: GuiKeyInput) => {
    if (key.control || key.alt || key.meta) return false;
    const points = [...key.key];
    if (points.length !== 1) return false;
    const point = points[0].codePointAt(0) ?? 0;
    return (
        (point >= 0x20 && point <= 0x7e) || (point >= 0xa1 && point <= 0x2ff)
    );
};

const isInsertMode = (snapshot: GuiSnapshot) => snapshot.mode === INSERT_MODE;

/**
 * Whether this frame describes a state a client may speak for at all.
 *
 * The overlay clauses repeat part of what `predictableInsert` already answers.
 * They are kept because they are the ones a reader of this file can check, and
 * because a frontend that grew an overlay of its own would otherwise inherit
 * permission it was never granted.
 */
const isSpeakableFor = (snapshot: GuiSnapshot, options: EchoOptions) =>
    options.remote &&
    options.connection.state === "connected" &&
    snapshot.predictableInsert &&
    isInsertMode(snapshot) &&
    !snapshot.readOnly &&
    !snapshot.dashboard &&
    !snapshot.prompt &&
    !snapshot.picker &&
    !snapshot.completion &&
    !snapshot.lspManager &&
    !snapshot.horizontalOffset &&
    columnsAreCells(snapshot);

/**
 * Whether typing this closing bracket would re-indent the line rather than
 * merely extend it.
 *
 * Transcribed from `electric_dedent_close_bracket`: it fires only when
 * everything before the cursor is whitespace and not empty, and everything
 * after it is whitespace too.
 */
const wouldReIndent = (
    snapshot: GuiSnapshot,
    run: EchoRun,
    character: string,
) => {
    if (!ELECTRIC_CLOSERS.has(character)) return false;
    const line = renderedLine(snapshot, run.anchor.line);
    if (!line) return true;
    const rendered = [...lineText(line)];
    const before = rendered.slice(0, run.anchor.column).join("") + runText(run);
    const after = rendered.slice(run.anchor.column).join("");
    return before.length > 0 && before.trim() === "" && after.trim() === "";
};

/**
 * Record a key on its way to the editor, drawing it if it can be vouched for.
 *
 * Extends the run the last frame certified. A key that cannot be modelled ends
 * the run rather than being skipped over inside it, so what is drawn is always
 * a contiguous stretch of text with a known start.
 *
 * Under a local transport this does nothing at all and hands back the state it
 * was given, so an in-process editor takes exactly the path it took before any
 * of this existed.
 */
export const recordSent = (
    state: EchoState,
    snapshot: GuiSnapshot,
    key: GuiKeyInput,
    options: EchoOptions,
): RecordOutcome => {
    if (!options.remote) return { state, predicted: false };

    const character = isPredictableCharacter(key) ? key.key : undefined;
    const entry: SentKey = { character, sentAt: options.now };
    const run = state.run;
    const extendable =
        character !== undefined &&
        run !== undefined &&
        !wouldReIndent(snapshot, run, character);
    // At the cap the run stops growing but stays on screen: erasing what has
    // already been drawn would be a worse answer to typing too far ahead than
    // simply not drawing any more of it.
    const room = (run?.keys.length ?? 0) < MAX_OUTSTANDING_PREDICTIONS;
    return {
        state: {
            journal: [...state.journal, entry].slice(
                -MAX_OUTSTANDING_PREDICTIONS,
            ),
            sentEpoch:
                state.sentEpoch === undefined ? undefined : state.sentEpoch + 1,
            run:
                extendable && room
                    ? { anchor: run.anchor, keys: [...run.keys, entry] }
                    : extendable
                      ? run
                      : undefined,
        },
        predicted: extendable && room,
    };
};

/**
 * Fold an authoritative frame in, and re-derive what may be drawn on top of it.
 *
 * `inputEpoch` says how much of what this client has sent the frame accounts
 * for; the remainder is the outstanding run. If every key in it is a plain
 * printable character and the frame certifies a plain insert state, the run is
 * drawn from the frame's own cursor. Otherwise nothing is: a frame that
 * contradicts the speculation, or that cannot be read, drops **every**
 * outstanding prediction and renders the authority as it arrived, rather than
 * repairing part of it.
 */
export const reconcile = (
    state: EchoState,
    snapshot: GuiSnapshot,
    options: EchoOptions,
): ReconcileOutcome => {
    const drawn = state.run?.keys.length ?? 0;
    // A client that has sent nothing takes the frame's count as its own
    // starting point; every key it sends afterwards raises its side by one.
    const sentEpoch = state.sentEpoch ?? snapshot.inputEpoch;
    // Clamped because the editor's count also rises for other clients and for
    // commands this one never counted -- a click, a paste, a resize. All of
    // those overstate, and overstating leaves less outstanding.
    const outstanding = Math.max(0, sentEpoch - snapshot.inputEpoch);
    const settled: EchoState = {
        journal: state.journal,
        sentEpoch: snapshot.inputEpoch + outstanding,
    };
    const confirmed = Math.min(drawn, Math.max(0, drawn - outstanding));
    const nothingDrawn: ReconcileOutcome = {
        state: settled,
        confirmed,
        dropped: drawn > confirmed,
    };

    const keys = outstanding ? state.journal.slice(-outstanding) : [];
    if (
        outstanding > MAX_OUTSTANDING_PREDICTIONS ||
        keys.length !== outstanding ||
        keys.some((key) => key.character === undefined) ||
        keys.some(
            (key) => options.now - key.sentAt >= PREDICTION_LIFETIME_MS,
        ) ||
        !isSpeakableFor(snapshot, options)
    )
        return nothingDrawn;

    const line = renderedLine(snapshot, snapshot.cursor.line);
    if (!line || !linesUpWithItsColumns(line)) return nothingDrawn;

    // The same refusal {@link recordSent} makes, applied to the run this frame
    // implies rather than to the key just typed: an outstanding `}` typed while
    // an unmodelled key was in flight -- the `{<CR>}` shape, which is how the
    // bracket is usually typed -- was never offered to that check.
    const anchor = {
        line: snapshot.cursor.line,
        column: snapshot.cursor.column,
    };
    const grown: SentKey[] = [];
    for (const key of keys) {
        if (
            wouldReIndent(
                snapshot,
                { anchor, keys: grown },
                key.character ?? "",
            )
        )
            return nothingDrawn;
        grown.push(key);
    }

    return {
        state: {
            ...settled,
            run: { anchor, keys },
        },
        confirmed,
        dropped: false,
    };
};

/**
 * Stop drawing a run whose oldest key has stood too long.
 *
 * All of it, not only the stale key: the run is contiguous text, and a gap in
 * the middle of it would be a guess about the far side rather than a report of
 * what was typed.
 */
export const expire = (state: EchoState, now: number): EchoState =>
    state.run?.keys.some((key) => now - key.sentAt >= PREDICTION_LIFETIME_MS)
        ? { journal: state.journal, sentEpoch: state.sentEpoch }
        : state;

/**
 * When the drawn run expires, or `undefined` if nothing is drawn.
 *
 * The caller owns the timer; this module owns the deadline.
 */
export const nextExpiry = (state: EchoState): number | undefined =>
    state.run?.keys.length
        ? state.run.keys[0].sentAt + PREDICTION_LIFETIME_MS
        : undefined;

/**
 * Forget everything, keys included.
 *
 * Used when the link goes down. Chunk R6 drops input while disconnected rather
 * than queueing it, so a run outstanding at that moment is speaking for keys
 * that were never sent -- and the epoch it was counting from belongs to a
 * conversation that has ended.
 */
export const discard = (): EchoState => noPredictions;

const spliceSegments = (
    segments: GuiSegment[],
    column: number,
    text: string,
): GuiSegment[] => {
    const speculative: GuiSegment = {
        text,
        cells: [...text].length,
        cursor: false,
        selected: false,
        searchMatch: false,
        speculative: true,
    };
    const spliced: GuiSegment[] = [];
    let seen = 0;
    let placed = false;
    for (const segment of segments) {
        const points = [...segment.text];
        if (!placed && seen + points.length >= column) {
            const split = column - seen;
            if (split > 0)
                spliced.push({
                    ...segment,
                    text: points.slice(0, split).join(""),
                    cells: split,
                });
            spliced.push(speculative);
            if (split < points.length)
                spliced.push({
                    ...segment,
                    text: points.slice(split).join(""),
                    cells: points.length - split,
                });
            placed = true;
        } else {
            spliced.push(segment);
        }
        seen += points.length;
    }
    if (!placed) spliced.push(speculative);
    return spliced;
};

const withEchoedLine = (
    lines: GuiLine[],
    line: number,
    column: number,
    text: string,
) =>
    lines.map((row) =>
        row.number === line + 1 && !row.continuation
            ? { ...row, segments: spliceSegments(row.segments, column, text) }
            : row,
    );

/**
 * The frame to draw: the authority with the outstanding run written into it.
 *
 * Returns the frame untouched when nothing is outstanding -- by reference, not
 * by copy -- so a local session is not merely fast here but inert: what it
 * renders is the identical object the editor produced.
 *
 * The run is spliced in *before* the cell carrying the cursor, so the cursor
 * ends up after the typed text with no separate bookkeeping.
 */
export const withEcho = (
    snapshot: GuiSnapshot,
    state: EchoState,
): GuiSnapshot => {
    const run = state.run;
    if (!run?.keys.length) return snapshot;
    const { line, column } = run.anchor;
    if (snapshot.cursor.line !== line || snapshot.cursor.column !== column)
        return snapshot;
    if (!columnsAreCells(snapshot)) return snapshot;
    const rendered = renderedLine(snapshot, line);
    if (!rendered || !linesUpWithItsColumns(rendered)) return snapshot;

    const text = runText(run);
    const cursor = {
        ...snapshot.cursor,
        column: column + text.length,
        displayColumn: snapshot.cursor.displayColumn + text.length,
    };
    return {
        ...snapshot,
        cursor,
        lines: withEchoedLine(snapshot.lines, line, column, text),
        panes: snapshot.panes.map((pane) =>
            pane.focused
                ? {
                      ...pane,
                      cursor,
                      lines: withEchoedLine(pane.lines, line, column, text),
                  }
                : pane,
        ),
    };
};
