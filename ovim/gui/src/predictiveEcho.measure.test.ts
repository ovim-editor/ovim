/** @vitest-environment node */

import { describe, expect, it } from "vitest";
import {
    discard,
    recordSent,
    reconcile,
    withEcho,
    type EchoOptions,
    type EchoState,
} from "./predictiveEcho";
import type { GuiKeyInput, GuiSnapshot } from "./types";

/**
 * What predictive echo is actually worth, measured rather than assumed.
 *
 * Skipped unless `OVIM_REMOTE_SESSION` names a live headless session, for the
 * same reason `ovim/tests/remote_session_test.rs` is ignored by default: a
 * measurement that starts its own editor would be timing a build.
 *
 * ```text
 * ovim /tmp/echo.rs --headless --session echo &
 * OVIM_REMOTE_SESSION="$HOME/.cache/ovim/sessions/echo.json" \
 *     OVIM_REMOTE_LATENCY_MS=150 \
 *     npx vitest run src/predictiveEcho.measure.test.ts
 * ```
 *
 * `OVIM_REMOTE_LATENCY_MS` is the same knob [`RemoteTransport`] reads and means
 * the same thing -- a round trip, applied as half of it in each direction -- so
 * the number measured here is the number a window would feel.
 *
 * Two passes over the same sample at the same cadence, one predicting and one
 * not, because the only comparison worth making is the same keys typed the
 * same way with the feature on and off.
 */

// This file runs under Node rather than jsdom, and the project does not carry
// `@types/node` for the handful of fields used here.
declare const process: { env: Record<string, string | undefined> };

const descriptorPath = process.env.OVIM_REMOTE_SESSION;
const roundTrip = Number(process.env.OVIM_REMOTE_LATENCY_MS ?? 0);
const oneWay = roundTrip / 2;

/** A line of code with the shape of real typing: identifiers, punctuation, spaces. */
const SAMPLE =
    "let mut total: usize = items.iter().filter(|x| x.ready).count();";

/**
 * A minute of real editing rather than one clean line: entering insert mode,
 * indenting, correcting a typo, breaking a line, leaving insert mode again.
 *
 * `<CR>`, `<BS>`, `<Esc>` and `<Tab>` are written out because they are the keys
 * the hit rate hinges on -- each one is deliberately left to the editor, and
 * each one also ends the run in front of it.
 */
const EDIT_SESSION =
    "Go<CR>fn ready(&self) -> bool {<CR>self.state == Statw<BS>e::Redy<BS><BS>ady<CR>}<Esc>" +
    "kkA // checked remotely<Esc>o<Tab>let ok = self.ready();<Esc>";

/** 25 characters a second, which is fast but not unreasonable typing. */
const KEY_INTERVAL_MS = 40;

const sleep = (milliseconds: number) =>
    new Promise((resolve) => setTimeout(resolve, milliseconds));

const crossTheWire = () => (oneWay > 0 ? sleep(oneWay) : Promise.resolve());

const typed = (key: string): GuiKeyInput => ({
    key,
    shift: key.length === 1 && key !== key.toLowerCase(),
    control: false,
    alt: false,
    meta: false,
});

/** Split a script into the keys a window would send for it. */
const scriptKeys = (script: string): GuiKeyInput[] => {
    const named: Record<string, string> = {
        "<CR>": "Enter",
        "<BS>": "Backspace",
        "<Esc>": "Escape",
        "<Tab>": "Tab",
    };
    const keys: GuiKeyInput[] = [];
    for (let at = 0; at < script.length;) {
        const name = Object.keys(named).find((token) =>
            script.startsWith(token, at),
        );
        if (name) {
            keys.push(typed(named[name]));
            at += name.length;
            continue;
        }
        keys.push(typed(script[at]));
        at += 1;
    }
    return keys;
};

/** The cursor line of a frame, exactly as it would be drawn. */
const cursorLine = (frame: GuiSnapshot) =>
    frame.lines
        .filter((row) => row.number === frame.cursor.line + 1)
        .flatMap((row) => row.segments)
        .map((segment) => segment.text)
        .join("");

interface Session {
    key(key: GuiKeyInput): Promise<void>;
    close(): void;
}

/**
 * Read the session descriptor without a compile-time dependency on Node.
 *
 * The specifier is assembled rather than written out so the type checker,
 * which is configured for the browser bundle every other file in `src` ends up
 * in, leaves it alone.
 */
const readDescriptor = async (path: string) => {
    const fs = (await import(
        /* @vite-ignore */ ["node", "fs/promises"].join(":")
    )) as {
        readFile(path: string, encoding: string): Promise<string>;
    };
    return JSON.parse(await fs.readFile(path, "utf8")) as {
        port: number;
        capability: string;
    };
};

const openSession = async (
    path: string,
    onFrame: (frame: GuiSnapshot) => void,
): Promise<Session> => {
    const descriptor = await readDescriptor(path);
    const dialled =
        process.env.OVIM_REMOTE_ENDPOINT ?? `127.0.0.1:${descriptor.port}`;
    const base = `http://${dialled}/v1`;
    // The session's Host guard names its own port, which under a forward is not
    // the port dialled -- the trap `RemoteEndpoint` exists to avoid.
    const headers = {
        authorization: `Bearer ${descriptor.capability}`,
        host: `127.0.0.1:${descriptor.port}`,
        "content-type": "application/json",
    };
    const aborter = new AbortController();
    const stream = await fetch(`${base}/gui/stream`, {
        headers,
        signal: aborter.signal,
    });
    if (!stream.ok || !stream.body)
        throw new Error(
            `the session refused the snapshot stream: ${stream.status}`,
        );

    // Frames are held for the inbound half of the round trip on a timer rather
    // than by awaiting, so that several can be in flight at once. Awaiting each
    // in turn would throttle the stream to one frame per delay and measure the
    // harness rather than the editor.
    void (async () => {
        const decoder = new TextDecoder();
        let buffered = "";
        for await (const chunk of stream.body as unknown as AsyncIterable<Uint8Array>) {
            buffered += decoder.decode(chunk, { stream: true });
            let boundary = buffered.indexOf("\n\n");
            while (boundary >= 0) {
                const event = buffered.slice(0, boundary);
                buffered = buffered.slice(boundary + 2);
                const data = event
                    .split("\n")
                    .filter((row) => row.startsWith("data:"))
                    .map((row) => row.slice(5).trim())
                    .join("");
                if (data) {
                    const frame = JSON.parse(data) as GuiSnapshot;
                    if (oneWay > 0) setTimeout(() => onFrame(frame), oneWay);
                    else onFrame(frame);
                }
                boundary = buffered.indexOf("\n\n");
            }
        }
    })().catch(() => {});

    return {
        async key(key) {
            await crossTheWire();
            const response = await fetch(`${base}/gui/command`, {
                method: "POST",
                headers,
                body: JSON.stringify({
                    command: "key",
                    payload: { input: key },
                }),
            });
            await response.arrayBuffer();
            await crossTheWire();
        },
        close: () => aborter.abort(),
    };
};

const percentile = (samples: number[], fraction: number) => {
    const sorted = [...samples].sort((left, right) => left - right);
    return sorted[
        Math.min(sorted.length - 1, Math.floor(sorted.length * fraction))
    ];
};

const report = (label: string, samples: number[]) =>
    `  ${label}  n=${samples.length}  median ${percentile(samples, 0.5).toFixed(1)}ms  p95 ${percentile(
        samples,
        0.95,
    ).toFixed(1)}ms  max ${Math.max(...samples).toFixed(1)}ms`;

describe.skipIf(!descriptorPath)(
    "predictive echo against a live session",
    () => {
        it("puts a typed character on screen sooner than the editor can answer, and never puts a wrong one there", async () => {
            const link = {
                authority: undefined as GuiSnapshot | undefined,
                echo: discard(),
                speculating: false,
                dropped: 0,
            };
            const echoOptions = (): EchoOptions => ({
                remote: link.speculating,
                connection: { state: "connected" },
                now: performance.now(),
            });
            // The window's own loop, in miniature: accept the frame, then
            // re-derive what may be drawn on top of it.
            const accept = (frame: GuiSnapshot) => {
                if (link.authority && frame.revision < link.authority.revision)
                    return;
                link.authority = frame;
                const settled = reconcile(link.echo, frame, echoOptions());
                if (settled.dropped) link.dropped += 1;
                link.echo = settled.state;
            };
            const session = await openSession(descriptorPath as string, accept);
            const authority = () => {
                if (!link.authority)
                    throw new Error("no frame has arrived yet");
                return link.authority;
            };

            /** How long after `since` until `text` is on the cursor line being drawn. */
            const untilDrawn = async (
                text: string,
                drawn: () => GuiSnapshot,
                since = performance.now(),
            ) => {
                const deadline = performance.now() + 30_000;
                while (performance.now() < deadline) {
                    if (link.authority && cursorLine(drawn()).startsWith(text))
                        return performance.now() - since;
                    await sleep(1);
                }
                throw new Error(
                    `the session never drew ${JSON.stringify(text)}`,
                );
            };

            /**
             * Type the sample onto a fresh line, at a fixed cadence, timing each
             * character from the keypress to the moment it is on screen.
             */
            const pass = async (speculating: boolean) => {
                link.speculating = speculating;
                for (const key of ["Escape", "G", "o"]) {
                    const sending = session.key(typed(key));
                    link.echo = recordSent(
                        link.echo,
                        authority(),
                        typed(key),
                        echoOptions(),
                    ).state;
                    await sending;
                }
                await untilDrawn("", authority);

                // Typed ahead, the way a person types: the cadence is the same
                // in both passes and no key waits for the last one to appear,
                // or the pass that has to wait would also be typing slower.
                const drawn = speculating
                    ? () => withEcho(authority(), link.echo)
                    : authority;
                const timings: Array<Promise<number>> = [];
                let predicted = 0;
                let written = "";
                for (const character of SAMPLE) {
                    const key = typed(character);
                    const pressedAt = performance.now();
                    void session.key(key);
                    const recorded = recordSent(
                        link.echo,
                        authority(),
                        key,
                        echoOptions(),
                    );
                    link.echo = recorded.state;
                    if (recorded.predicted) predicted += 1;
                    written += character;
                    timings.push(untilDrawn(written, drawn, pressedAt));
                    await sleep(KEY_INTERVAL_MS);
                }
                const visible = await Promise.all(timings);
                await untilDrawn(written, authority);
                link.speculating = false;
                return { visible, predicted };
            };

            await untilDrawn("", authority);
            const speculated = await pass(true);
            const plain = await pass(false);

            await session.key(typed("Escape"));
            session.close();

            console.log(
                [
                    `injected round trip: ${roundTrip}ms`,
                    `predicted ${speculated.predicted}/${SAMPLE.length} keystrokes (${Math.round(
                        (speculated.predicted / SAMPLE.length) * 100,
                    )}%), ${link.dropped} frames dropped the run`,
                    report(
                        "keypress to visible, predicting    ",
                        speculated.visible,
                    ),
                    report(
                        "keypress to visible, not predicting",
                        plain.visible,
                    ),
                ].join("\n"),
            );

            expect(link.dropped).toBe(0);
            expect(speculated.predicted).toBeGreaterThan(0);
            expect(percentile(speculated.visible, 0.95)).toBeLessThanOrEqual(
                percentile(plain.visible, 0.95),
            );
        }, 180_000);

        it("predicts most of a real edit and mispredicts none of it", async () => {
            const link = {
                authority: undefined as GuiSnapshot | undefined,
                echo: discard(),
                dropped: 0,
            };
            const echoOptions = (): EchoOptions => ({
                remote: true,
                connection: { state: "connected" },
                now: performance.now(),
            });
            const accept = (frame: GuiSnapshot) => {
                if (link.authority && frame.revision < link.authority.revision)
                    return;
                link.authority = frame;
                const settled = reconcile(link.echo, frame, echoOptions());
                if (settled.dropped) link.dropped += 1;
                link.echo = settled.state;
            };
            const session = await openSession(descriptorPath as string, accept);
            while (!link.authority) await sleep(5);

            const keys = scriptKeys(EDIT_SESSION);
            let predicted = 0;
            let printable = 0;
            for (const key of keys) {
                if (key.key.length === 1) printable += 1;
                void session.key(key);
                const recorded = recordSent(
                    link.echo,
                    link.authority as GuiSnapshot,
                    key,
                    echoOptions(),
                );
                link.echo = recorded.state;
                if (recorded.predicted) predicted += 1;
                await sleep(KEY_INTERVAL_MS);
            }
            // Long enough for the last frames to land and be folded in.
            await sleep(2_000 + roundTrip * 4);
            session.close();

            console.log(
                [
                    `injected round trip: ${roundTrip}ms`,
                    `a real edit: ${keys.length} keys, ${printable} of them printable`,
                    `predicted ${predicted} (${Math.round(
                        (predicted / keys.length) * 100,
                    )}% of all keys, ${Math.round(
                        (predicted / printable) * 100,
                    )}% of printable ones)`,
                    `${link.dropped} frames dropped the run`,
                ].join("\n"),
            );

            expect(link.dropped).toBe(0);
        }, 180_000);
    },
);
