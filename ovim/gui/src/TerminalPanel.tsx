import { Channel, invoke } from "@tauri-apps/api/core";
import { FitAddon } from "@xterm/addon-fit";
import { Terminal } from "@xterm/xterm";
import { createEffect, createSignal, onCleanup, onMount, Show } from "solid-js";
import "@xterm/xterm/css/xterm.css";

type TerminalEvent =
    | { type: "data"; id: number; data: number[] }
    | { type: "exit"; id: number; code?: number }
    | { type: "error"; id: number; message: string };

/**
 * xterm.js does not parse `rgb()` strings (it silently falls back to black),
 * so computed CSS colours are converted to hex before reaching its theme.
 */
export const cssColorToHex = (color: string): string | undefined => {
    const match = color.match(
        /^rgba?\(\s*(\d+)[\s,]+(\d+)[\s,]+(\d+)(?:[\s,/]+([\d.]+%?))?\s*\)$/,
    );
    if (!match) return color.startsWith("#") ? color : undefined;
    const alpha = match[4];
    if (alpha !== undefined && parseFloat(alpha) === 0) return undefined;
    return `#${match
        .slice(1, 4)
        .map((part) => Number(part).toString(16).padStart(2, "0"))
        .join("")}`;
};

export default function TerminalPanel(props: {
    native: boolean;
    active: boolean;
    /** Changes whenever the editor theme changes; colours are re-read then. */
    theme?: unknown;
}) {
    let surface!: HTMLDivElement;
    let terminal: Terminal | undefined;
    let fit: FitAddon | undefined;
    let session: number | undefined;
    let generation = 0;
    let lastSize: { columns: number; rows: number } | undefined;
    let pendingWrite = Promise.resolve();
    const [status, setStatus] = createSignal<
        "starting" | "running" | "exited" | "error" | "unavailable"
    >(props.native ? "starting" : "unavailable");
    const [message, setMessage] = createSignal("");

    const surfaceTheme = () => {
        const style = getComputedStyle(surface);
        return {
            background: cssColorToHex(style.backgroundColor),
            foreground: cssColorToHex(style.color),
        };
    };

    createEffect(() => {
        props.theme;
        // Theme variables land on the app root in the same render pass; read
        // the computed colours once the DOM reflects them.
        queueMicrotask(() => {
            if (terminal?.options) terminal.options.theme = surfaceTheme();
        });
    });

    const resize = () => {
        if (
            !props.active ||
            !terminal ||
            !fit ||
            !surface.clientWidth ||
            !surface.clientHeight
        )
            return;
        fit.fit();
        if (session !== undefined) {
            const size = { columns: terminal.cols, rows: terminal.rows };
            if (
                lastSize?.columns === size.columns &&
                lastSize.rows === size.rows
            )
                return;
            lastSize = size;
            const current = generation;
            void invoke("gui_terminal_resize", {
                id: session,
                ...size,
            }).catch((reason) => {
                if (current !== generation) return;
                setStatus("error");
                setMessage(String(reason));
            });
        }
    };

    const close = async () => {
        generation += 1;
        if (session === undefined) return;
        const id = session;
        session = undefined;
        await invoke("gui_terminal_close", { id }).catch(() => {});
    };

    const start = async () => {
        if (!terminal || !fit) return;
        const previous = close();
        const current = generation;
        lastSize = undefined;
        setStatus("starting");
        setMessage("");
        await previous;
        if (current !== generation || !terminal) return;
        terminal.reset();
        resize();
        let ended = false;
        const onEvent = new Channel<TerminalEvent>();
        onEvent.onmessage = (event) => {
            if (current !== generation) return;
            switch (event.type) {
                case "data":
                    terminal?.write(new Uint8Array(event.data), () => {
                        if (current !== generation) return;
                        void invoke("gui_terminal_ack", {
                            id: event.id,
                            bytes: event.data.length,
                        }).catch((reason) => {
                            if (current !== generation) return;
                            setStatus("error");
                            setMessage(String(reason));
                        });
                    });
                    break;
                case "exit":
                    ended = true;
                    setStatus("exited");
                    setMessage(
                        `Process exited${event.code === undefined ? "" : ` with code ${event.code}`}.`,
                    );
                    session = undefined;
                    break;
                case "error":
                    ended = true;
                    setStatus("error");
                    setMessage(event.message);
                    session = undefined;
                    break;
            }
        };
        try {
            const id = await invoke<number>("gui_terminal_open", {
                columns: terminal.cols,
                rows: terminal.rows,
                onEvent,
            });
            if (current !== generation) {
                void invoke("gui_terminal_close", { id }).catch(() => {});
                return;
            }
            if (ended) {
                void invoke("gui_terminal_close", { id }).catch(() => {});
                return;
            }
            session = id;
            setStatus("running");
            resize();
            if (props.active) terminal.focus();
        } catch (reason) {
            if (current !== generation) return;
            setStatus("error");
            setMessage(String(reason));
        }
    };

    onMount(() => {
        if (!props.native) return;
        terminal = new Terminal({
            cursorBlink: true,
            screenReaderMode: true,
            fontFamily:
                getComputedStyle(surface)
                    .getPropertyValue("--editor-font")
                    .trim() || "monospace",
            fontSize: 12,
            scrollback: 5000,
            theme: surfaceTheme(),
        });
        fit = new FitAddon();
        terminal.loadAddon(fit);
        terminal.open(surface);
        const input = terminal.onData((data) => {
            if (session !== undefined) {
                const id = session;
                const current = generation;
                pendingWrite = pendingWrite
                    .then(async () => {
                        // Stay below the PTY's byte limit, including multibyte
                        // text, and wait for each chunk before sending another.
                        for (let offset = 0; offset < data.length;) {
                            if (current !== generation) return;
                            let end = Math.min(offset + 16 * 1024, data.length);
                            const last = data.charCodeAt(end - 1);
                            if (
                                end < data.length &&
                                last >= 0xd800 &&
                                last <= 0xdbff
                            )
                                end -= 1;
                            await invoke("gui_terminal_write", {
                                id,
                                data: data.slice(offset, end),
                            });
                            offset = end;
                        }
                    })
                    .catch((reason) => {
                        if (current !== generation) return;
                        setStatus("error");
                        setMessage(String(reason));
                    });
            }
        });
        const observer = new ResizeObserver(resize);
        observer.observe(surface);
        void start();
        onCleanup(() => {
            void close();
            observer.disconnect();
            input.dispose();
            terminal?.dispose();
        });
    });

    createEffect(() => {
        if (props.active && status() === "running" && terminal) {
            queueMicrotask(() => {
                if (!props.active) return;
                resize();
                terminal?.focus();
            });
        }
    });

    return (
        <section
            class="side-panel terminal-panel"
            aria-label="Terminal"
            data-gui-native-control
        >
            <header class="side-panel-header">
                <div>
                    <b>Terminal</b>
                    <small>Workspace shell</small>
                </div>
                <span classList={{ working: status() === "running" }}>
                    {status()}
                </span>
            </header>
            <Show when={!props.native}>
                <p class="terminal-notice">
                    Terminal is available in the desktop app.
                </p>
            </Show>
            <div
                class="terminal-surface"
                ref={surface!}
                hidden={!props.native}
            />
            <Show
                when={
                    props.native &&
                    (status() === "exited" || status() === "error")
                }
            >
                <div class="terminal-notice" role="status">
                    <span>{message()}</span>
                    <button type="button" onClick={() => void start()}>
                        Restart terminal
                    </button>
                </div>
            </Show>
        </section>
    );
}
