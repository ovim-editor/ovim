import { describe, expect, it } from "vitest";
import { mockSnapshot } from "./mock";
import {
    MAX_OUTSTANDING_PREDICTIONS,
    PREDICTION_LIFETIME_MS,
    discard,
    expire,
    nextExpiry,
    noPredictions,
    recordSent,
    reconcile,
    withEcho,
    type EchoOptions,
    type EchoState,
} from "./predictiveEcho";
import type { GuiConnection, GuiKeyInput, GuiLine, GuiSnapshot } from "./types";

/**
 * One rendered line, in the shape `segments_for_line` produces it.
 *
 * The synthetic trailing cell the editor adds when the cursor sits past the end
 * of a line is reproduced on purpose: it is indistinguishable from a real
 * trailing space in the projection, and the column arithmetic under test has to
 * survive that.
 */
const projectLine = (
    text: string,
    number: number,
    cursorColumn?: number,
): GuiLine => {
    const glyphs = [...text];
    const run = (slice: string[], cursor: boolean) => ({
        text: slice.join(""),
        cells: slice.length,
        cursor,
        selected: false,
        searchMatch: false,
    });
    const segments =
        cursorColumn === undefined
            ? [run(glyphs.length ? glyphs : [" "], false)]
            : cursorColumn < glyphs.length
              ? [
                    ...(cursorColumn
                        ? [run(glyphs.slice(0, cursorColumn), false)]
                        : []),
                    run([glyphs[cursorColumn]], true),
                    ...(cursorColumn + 1 < glyphs.length
                        ? [run(glyphs.slice(cursorColumn + 1), false)]
                        : []),
                ]
              : [
                    ...(glyphs.length ? [run(glyphs, false)] : []),
                    run([" "], true),
                ];
    return {
        number,
        continuation: false,
        displayStart: 0,
        current: cursorColumn !== undefined,
        segments,
    };
};

interface FrameOptions {
    text?: string[];
    line?: number;
    column?: number;
    /** How many keystrokes the editor says it has taken in. */
    epoch?: number;
}

/** A frame of a plain insert-mode session, ready to be spoken for. */
const frame = (
    options: FrameOptions = {},
    overrides: Partial<GuiSnapshot> = {},
): GuiSnapshot => {
    const text = options.text ?? ["fn main() {", "", "}"];
    const line = options.line ?? 1;
    const column = options.column ?? 0;
    const lines = text.map((row, index) =>
        projectLine(row, index + 1, index === line ? column : undefined),
    );
    const cursor = { line, column, displayColumn: column };
    return {
        ...mockSnapshot,
        revision: 1,
        mode: "INSERT",
        predictableInsert: true,
        inputEpoch: options.epoch ?? 0,
        readOnly: false,
        dashboard: false,
        horizontalOffset: 0,
        prompt: undefined,
        picker: undefined,
        completion: undefined,
        lspManager: undefined,
        aiChat: undefined,
        cursor,
        firstLine: 0,
        totalLines: text.length,
        lines,
        panes: [
            {
                index: 0,
                bufferId: 1,
                focused: true,
                fileName: "main.rs",
                modified: true,
                cursor,
                firstLine: 0,
                scrollSubrow: 0,
                horizontalOffset: 0,
                totalLines: text.length,
                lines,
            },
        ],
        ...overrides,
    };
};

const connected: GuiConnection = { state: "connected" };

const options = (overrides: Partial<EchoOptions> = {}): EchoOptions => ({
    remote: true,
    connection: connected,
    now: 0,
    ...overrides,
});

const typed = (
    key: string,
    overrides: Partial<GuiKeyInput> = {},
): GuiKeyInput => ({
    key,
    shift: false,
    control: false,
    alt: false,
    meta: false,
    ...overrides,
});

/** A client that has seen one frame and is ready to speak for the next key. */
const ready = (snapshot: GuiSnapshot, at = 0) =>
    reconcile(noPredictions, snapshot, options({ now: at })).state;

/** Type a run of characters, returning whatever the module allowed. */
const typeRun = (
    state: EchoState,
    snapshot: GuiSnapshot,
    run: string,
    at = 0,
) => {
    let result = state;
    for (const character of run)
        result = recordSent(
            result,
            snapshot,
            typed(character),
            options({ now: at }),
        ).state;
    return result;
};

const drawnLine = (snapshot: GuiSnapshot, line: number) =>
    snapshot.lines
        .filter((row) => row.number === line + 1)
        .flatMap((row) => row.segments)
        .map((segment) => segment.text)
        .join("");

const speculativeText = (snapshot: GuiSnapshot) =>
    snapshot.lines
        .flatMap((row) => row.segments)
        .filter((segment) => segment.speculative)
        .map((segment) => segment.text)
        .join("");

describe("predictive echo", () => {
    it("draws a printable character typed into a plain insert state at once", () => {
        const authority = frame();
        const state = typeRun(ready(authority), authority, "le");

        expect(state.run?.keys).toHaveLength(2);
        expect(drawnLine(withEcho(authority, state), 1)).toBe("le ");
        expect(speculativeText(withEcho(authority, state))).toBe("le");
    });

    it("leaves the cursor after the text the editor has not agreed to yet", () => {
        const authority = frame();
        const echoed = withEcho(
            authority,
            typeRun(ready(authority), authority, "let"),
        );

        expect(echoed.cursor.column).toBe(3);
        expect(echoed.cursor.displayColumn).toBe(3);
        expect(echoed.panes[0].cursor.column).toBe(3);
        const segments = echoed.lines[1].segments;
        const speculative = segments.findIndex(
            (segment) => segment.speculative,
        );
        const cursor = segments.findIndex((segment) => segment.cursor);
        expect(speculative).toBeLessThan(cursor);
    });

    it("absorbs a confirmed prediction without a flicker of duplicated text", () => {
        const authority = frame();
        const state = typeRun(ready(authority), authority, "let");
        // The editor has taken in all three keys and says so.
        const confirming = frame({
            text: ["fn main() {", "let", "}"],
            column: 3,
            epoch: 3,
        });

        const settled = reconcile(state, confirming, options());

        expect(settled.dropped).toBe(false);
        expect(settled.confirmed).toBe(3);
        expect(settled.state.run?.keys ?? []).toHaveLength(0);
        // The drawn frame is the frame itself, so a character cannot appear
        // once as authority and once as speculation.
        expect(withEcho(confirming, settled.state)).toBe(confirming);
        expect(drawnLine(confirming, 1)).toBe("let ");
    });

    it("keeps drawing only the part the editor has not caught up with", () => {
        const authority = frame();
        const state = typeRun(ready(authority), authority, "let");
        const partial = frame({
            text: ["fn main() {", "l", "}"],
            column: 1,
            epoch: 1,
        });

        const settled = reconcile(state, partial, options());

        expect(settled.confirmed).toBe(1);
        expect(settled.dropped).toBe(false);
        expect(settled.state.run?.keys.map((key) => key.character)).toEqual([
            "e",
            "t",
        ]);
        expect(drawnLine(withEcho(partial, settled.state), 1)).toBe("let ");
    });

    it("drops the whole run when the frame it lands on cannot be spoken for", () => {
        const authority = frame();
        const state = typeRun(ready(authority), authority, "let");
        // The editor left insert mode under the window -- an agent ran a macro,
        // another client pressed Escape -- so nothing outstanding can be
        // vouched for any more.
        const disagreeing = frame(
            { text: ["fn main() {", "l", "}"], column: 1, epoch: 1 },
            { mode: "NORMAL", predictableInsert: false },
        );

        const settled = reconcile(state, disagreeing, options());

        expect(settled.dropped).toBe(true);
        expect(settled.state.run).toBeUndefined();
    });

    it("leaves no residue on screen after a drop", () => {
        const authority = frame();
        const state = typeRun(ready(authority), authority, "let");
        const disagreeing = frame(
            { text: ["fn main() {", "XYZ", "}"], column: 3, epoch: 1 },
            { mode: "NORMAL", predictableInsert: false },
        );

        const settled = reconcile(state, disagreeing, options()).state;

        expect(withEcho(disagreeing, settled)).toBe(disagreeing);
        expect(speculativeText(withEcho(disagreeing, settled))).toBe("");
        expect(drawnLine(disagreeing, 1)).toBe("XYZ ");
    });

    it("says nothing while a key it could not model is still unaccounted for", () => {
        // The trap the input epoch exists to close: `o` opened a line, its
        // effect has not come back yet, and a character predicted against the
        // frame that predates it would be drawn in the wrong place entirely.
        const before = frame({
            text: ["fn main() {", "}"],
            line: 1,
            column: 0,
        });
        const opened = recordSent(
            ready(before),
            before,
            typed("Enter"),
            options(),
        ).state;

        const attempt = recordSent(opened, before, typed("l"), options());

        expect(attempt.predicted).toBe(false);
        expect(withEcho(before, attempt.state)).toBe(before);
    });

    it("adopts the characters it could not vouch for as soon as a frame accounts for the rest", () => {
        // Typing straight through an unmodelled key: the run cannot be drawn
        // while `o` is outstanding, and appears in one go once the frame that
        // counted it arrives.
        const before = frame({
            text: ["fn main() {", "}"],
            line: 1,
            column: 0,
        });
        let state = recordSent(
            ready(before),
            before,
            typed("Enter"),
            options(),
        ).state;
        state = typeRun(state, before, "let");
        expect(withEcho(before, state)).toBe(before);

        const opened = frame({
            text: ["fn main() {", "", "}"],
            line: 1,
            column: 0,
            epoch: 1,
        });
        const settled = reconcile(state, opened, options()).state;

        expect(settled.run?.keys.map((key) => key.character)).toEqual([
            "l",
            "e",
            "t",
        ]);
        expect(drawnLine(withEcho(opened, settled), 1)).toBe("let ");
    });

    it("expires a run rather than leaving phantom text on screen", () => {
        const authority = frame();
        const state = typeRun(ready(authority, 100), authority, "let", 100);

        expect(nextExpiry(state)).toBe(100 + PREDICTION_LIFETIME_MS);
        expect(expire(state, 100 + PREDICTION_LIFETIME_MS - 1)).toBe(state);
        const expired = expire(state, 100 + PREDICTION_LIFETIME_MS);
        expect(expired.run).toBeUndefined();
        expect(withEcho(authority, expired)).toBe(authority);
        expect(nextExpiry(noPredictions)).toBeUndefined();
    });

    it("expires the whole run rather than a gap in the middle of it", () => {
        const authority = frame();
        const early = typeRun(ready(authority), authority, "le", 0);
        const late = typeRun(early, authority, "t", PREDICTION_LIFETIME_MS - 1);

        expect(expire(late, PREDICTION_LIFETIME_MS).run).toBeUndefined();
    });

    it("does not resurrect an expired run from a later frame", () => {
        const authority = frame();
        const state = typeRun(ready(authority), authority, "let");

        const settled = reconcile(
            state,
            frame({ epoch: 0 }),
            options({ now: PREDICTION_LIFETIME_MS }),
        );

        expect(settled.state.run).toBeUndefined();
    });

    it("is inert under a local transport", () => {
        const authority = frame();
        const local = options({ remote: false });
        const state = reconcile(noPredictions, authority, local).state;
        const attempt = recordSent(state, authority, typed("l"), local);

        expect(attempt.predicted).toBe(false);
        expect(attempt.state).toBe(state);
        expect(withEcho(authority, attempt.state)).toBe(authority);
    });

    it("says nothing for a link that is not carrying the keystroke anyway", () => {
        const authority = frame();
        const down: GuiConnection[] = [
            {
                state: "reconnecting",
                attempt: 1,
                retryInMs: 500,
                detail: "dropped",
            },
            {
                state: "lost",
                reason: "sessionGone",
                detail: "ended",
                canStartASession: false,
            },
        ];

        for (const connection of down) {
            const state = reconcile(
                noPredictions,
                authority,
                options({ connection }),
            ).state;
            expect(
                recordSent(
                    state,
                    authority,
                    typed("l"),
                    options({ connection }),
                ).predicted,
            ).toBe(false);
        }
    });

    it("discards everything, keys included, when the link goes down", () => {
        const authority = frame();
        const state = typeRun(ready(authority), authority, "let");

        expect(discard().run).toBeUndefined();
        expect(discard().journal).toHaveLength(0);
        expect(withEcho(authority, discard())).toBe(authority);
        expect(state.run?.keys).toHaveLength(3);
    });

    it("caps how far ahead of the editor it will ever get", () => {
        const authority = frame();
        const state = typeRun(
            ready(authority),
            authority,
            "x".repeat(MAX_OUTSTANDING_PREDICTIONS + 5),
        );

        expect(state.run?.keys).toHaveLength(MAX_OUTSTANDING_PREDICTIONS);
    });

    it("leaves a closing bracket on an otherwise blank line to the editor even when a frame is what would draw it", () => {
        // The `{<CR>}` shape, which is how anyone types a closing brace: the
        // bracket is refused while `<CR>` is outstanding, and the frame that
        // accounts for `<CR>` must not adopt it either --
        // `electric_dedent_close_bracket` will re-indent the line rather than
        // insert at the cursor.
        const before = frame({
            text: ["fn main() {", "}"],
            line: 0,
            column: 11,
        });
        let state = recordSent(
            ready(before),
            before,
            typed("Enter"),
            options(),
        ).state;
        state = recordSent(state, before, typed("}"), options()).state;
        expect(state.run?.keys ?? []).toHaveLength(0);

        const opened = frame({
            text: ["fn main() {", "    ", "}"],
            line: 1,
            column: 4,
            epoch: 1,
        });
        const settled = reconcile(state, opened, options()).state;

        expect(settled.run?.keys ?? []).toHaveLength(0);
        expect(withEcho(opened, settled)).toBe(opened);
    });

    it("still adopts a closing bracket that is only a character", () => {
        // The same shape, except the run puts something in front of the
        // bracket: by the time the editor reads it the line is no longer
        // blank, so it is an insertion like any other.
        const before = frame({
            text: ["fn main() {", "}"],
            line: 0,
            column: 11,
        });
        let state = recordSent(
            ready(before),
            before,
            typed("Enter"),
            options(),
        ).state;
        state = typeRun(state, before, "f()");

        const opened = frame({
            text: ["fn main() {", "    ", "}"],
            line: 1,
            column: 4,
            epoch: 1,
        });
        const settled = reconcile(state, opened, options()).state;

        expect(settled.run?.keys.map((key) => key.character)).toEqual([
            "f",
            "(",
            ")",
        ]);
    });

    it("leaves a closing bracket on an otherwise blank line to the editor", () => {
        // `electric_dedent_close_bracket` re-indents the line, which is not an
        // insertion at all.
        const indented = frame({
            text: ["fn main() {", "    ", "}"],
            column: 4,
        });
        for (const bracket of ["}", ")", "]"])
            expect(
                recordSent(ready(indented), indented, typed(bracket), options())
                    .predicted,
            ).toBe(false);
        // The same bracket anywhere it is only a character stays predictable.
        const inline = frame({
            text: ["fn main() {", "    foo(", "}"],
            column: 8,
        });
        expect(
            recordSent(ready(inline), inline, typed(")"), options()).predicted,
        ).toBe(true);
    });
});

describe("the keys predictive echo refuses", () => {
    const authority = frame();
    const refused: Array<[string, GuiKeyInput]> = [
        ["Enter, whose indentation the far side computes", typed("Enter")],
        ["Tab, whose width the far side computes", typed("Tab")],
        ["Backspace, which interacts with auto-indent", typed("Backspace")],
        ["Escape", typed("Escape")],
        ["an arrow key", typed("ArrowLeft")],
        ["a control chord", typed("r", { control: true })],
        ["an alt chord", typed("f", { alt: true })],
        ["a command chord", typed("s", { meta: true })],
        ["a wide glyph whose column is not its cell", typed("漢")],
        ["a combining mark", typed("́")],
    ];

    for (const [what, key] of refused)
        it(`refuses ${what}`, () => {
            expect(
                recordSent(ready(authority), authority, key, options())
                    .predicted,
            ).toBe(false);
        });
});

describe("the states predictive echo refuses", () => {
    const refused: Array<[string, Partial<GuiSnapshot>]> = [
        ["normal mode", { mode: "NORMAL", predictableInsert: false }],
        ["visual mode", { mode: "VISUAL", predictableInsert: false }],
        [
            "visual block mode",
            { mode: "VISUAL_BLOCK", predictableInsert: false },
        ],
        ["replace mode", { mode: "REPLACE", predictableInsert: false }],
        ["command mode", { mode: "COMMAND", predictableInsert: false }],
        ["search mode", { mode: "SEARCH", predictableInsert: false }],
        [
            "an editor that says a keystroke is not a plain insertion",
            { predictableInsert: false },
        ],
        [
            "a mode string the editor grew after this file was written",
            { mode: "OPERATOR" },
        ],
        ["the dashboard", { dashboard: true }],
        ["a read-only buffer", { readOnly: true }],
        ["a horizontally scrolled viewport", { horizontalOffset: 4 }],
        [
            "an open command line",
            { prompt: { prefix: ":", text: "w", cursor: 1 } },
        ],
        [
            "an open picker",
            {
                picker: {
                    title: "Files",
                    query: "",
                    selected: 0,
                    total: 0,
                    items: [],
                },
            },
        ],
        [
            "an open completion popup",
            { completion: { selected: 0, items: [] } },
        ],
        [
            "the language server manager",
            {
                lspManager: {
                    filter: "",
                    selected: 0,
                    showDetail: false,
                    items: [],
                },
            },
        ],
    ];

    for (const [what, overrides] of refused)
        it(`refuses ${what}`, () => {
            const authority = frame({}, overrides);
            expect(
                recordSent(ready(authority), authority, typed("l"), options())
                    .predicted,
            ).toBe(false);
        });

    it("refuses a wrapped line, whose columns no longer line up", () => {
        const plain = frame();
        const wrapped = {
            ...plain,
            lines: [
                plain.lines[0],
                plain.lines[1],
                { ...plain.lines[1], continuation: true },
                plain.lines[2],
            ],
        };

        expect(
            recordSent(ready(wrapped), wrapped, typed("l"), options())
                .predicted,
        ).toBe(false);
    });

    it("refuses a line indented with tabs, whose columns are not its cells", () => {
        // The editor renders a tab as the spaces it fills, so the segments look
        // exactly like four spaces. The cursor gives it away: one buffer
        // column, four display columns.
        const plain = frame({ text: ["fn main() {", "\t", "}"], column: 1 });
        const tabbed = {
            ...plain,
            cursor: { line: 1, column: 1, displayColumn: 4 },
            lines: plain.lines.map((row, index) =>
                index === 1
                    ? {
                          ...row,
                          segments: [
                              {
                                  text: "    ",
                                  cells: 4,
                                  cursor: false,
                                  selected: false,
                                  searchMatch: false,
                              },
                              ...row.segments.slice(1),
                          ],
                      }
                    : row,
            ),
        };

        expect(
            recordSent(ready(tabbed), tabbed, typed("l"), options()).predicted,
        ).toBe(false);
    });

    it("refuses a cursor line that is scrolled off the top of the viewport", () => {
        const plain = frame();
        const scrolled = { ...plain, lines: plain.lines.slice(2) };

        expect(
            recordSent(ready(scrolled), scrolled, typed("l"), options())
                .predicted,
        ).toBe(false);
    });
});
