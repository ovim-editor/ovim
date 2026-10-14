import {
    For,
    Show,
    createEffect,
    createSignal,
    onCleanup,
    onMount,
} from "solid-js";
import { guiKeyInput } from "./guiInput";
import { Icon, IconButton } from "./Icon";
import { utf16OffsetFromUtf8, utf8OffsetFromTextArea } from "./textEncoding";
import type { GuiAiChat, GuiKeyInput } from "./types";

const FALLBACK_CELL_WIDTH = 8.15;

const chatInputColumns = (root: HTMLElement) => {
    const style = getComputedStyle(root);
    const usableWidth =
        root.clientWidth -
        (Number.parseFloat(style.paddingLeft) || 0) -
        (Number.parseFloat(style.paddingRight) || 0);
    if (usableWidth <= 0) return 0;
    const probe = document.createElement("span");
    probe.textContent = "M".repeat(32);
    probe.style.cssText = `position:fixed;visibility:hidden;white-space:pre;font:${style.font};`;
    document.body.append(probe);
    const measured = probe.getBoundingClientRect().width / 32;
    probe.remove();
    return Math.max(
        1,
        Math.floor(usableWidth / (measured || FALLBACK_CELL_WIDTH)),
    );
};

export type ChatInputUpdate = {
    expectedInput: string;
    expectedCursor: number;
    input: string;
    cursor: number;
    action?: GuiKeyInput;
};

export { utf16OffsetFromUtf8, utf8OffsetFromTextArea } from "./textEncoding";

export default function ChatComposer(props: {
    chat: GuiAiChat;
    revision?: number;
    bindInput?: (input: HTMLTextAreaElement | undefined) => void;
    onUpdate?: (update: ChatInputUpdate) => Promise<void>;
    onWidth?: (columns: number) => void;
    onRemoveDiffComments?: () => void;
    onRemoveImage?: (index: number) => void;
}) {
    const [draft, setDraft] = createSignal(props.chat.input);
    let input!: HTMLTextAreaElement;
    let optimisticInput = props.chat.input;
    let optimisticCursor = props.chat.inputCursor;
    let awaiting:
        | {
              base: string;
              action: boolean;
              responseDone: boolean;
              revision: number;
          }
        | undefined;
    let syncedInput: string | undefined;
    let syncedCursor: number | undefined;
    let mutations = Promise.resolve();
    const hasActiveRun = () =>
        !props.chat.externalQuestion &&
        (props.chat.waiting || props.chat.activity !== "idle");

    const resize = () => {
        if (!input) return;
        // Measuring the live input at its default height temporarily expands the
        // transcript. WebKit clamps its scrollTop during that layout, moving the
        // conversation on every keystroke even when the final height is unchanged.
        const probe = input.cloneNode(false) as HTMLTextAreaElement;
        probe.removeAttribute("id");
        probe.removeAttribute("aria-label");
        probe.tabIndex = -1;
        probe.setAttribute("aria-hidden", "true");
        probe.value = input.value;
        Object.assign(probe.style, {
            position: "absolute",
            visibility: "hidden",
            pointerEvents: "none",
            width: `${input.getBoundingClientRect().width}px`,
            height: "0",
            minHeight: "0",
            maxHeight: "none",
            overflow: "hidden",
            top: "0",
            left: "0",
        });
        input.parentElement!.append(probe);
        const height = Math.min(probe.scrollHeight, 220);
        probe.remove();
        input.style.height = `${height}px`;
    };

    const applyRemote = () => {
        const remoteInput = props.chat.input;
        const remoteCursor = props.chat.inputCursor;
        let acknowledgedLocal = false;
        if (awaiting) {
            const matchesOptimistic =
                remoteInput === optimisticInput &&
                remoteCursor === optimisticCursor;
            const actionChangedInput =
                awaiting.action &&
                awaiting.responseDone &&
                (props.revision ?? 0) > awaiting.revision &&
                remoteInput !== awaiting.base;
            if (!matchesOptimistic && !actionChangedInput) return;
            acknowledgedLocal = matchesOptimistic;
            awaiting = undefined;
        }
        const externalChange =
            syncedInput !== remoteInput || syncedCursor !== remoteCursor;
        const inputChanged = remoteInput !== optimisticInput;
        syncedInput = remoteInput;
        syncedCursor = remoteCursor;
        optimisticInput = remoteInput;
        optimisticCursor = remoteCursor;
        setDraft(remoteInput);
        queueMicrotask(() => {
            if (!input) return;
            // An echo of a native edit, or an unrelated chat snapshot, must
            // leave the textarea's selection intact, even while focus is in a
            // menu. An editor action that changes the draft or cursor still
            // takes effect.
            if (inputChanged || (externalChange && !acknowledgedLocal)) {
                const cursor = utf16OffsetFromUtf8(remoteInput, remoteCursor);
                input.setSelectionRange(cursor, cursor);
            }
            resize();
        });
    };

    createEffect(applyRemote);

    const selectionFocus = () =>
        input.selectionDirection === "backward"
            ? input.selectionStart
            : input.selectionEnd;

    const publish = (
        nextInput: string,
        utf16Cursor: number,
        action?: GuiKeyInput,
    ) => {
        const nextCursor = utf8OffsetFromTextArea(nextInput, utf16Cursor);
        const update: ChatInputUpdate = {
            expectedInput: optimisticInput,
            expectedCursor: optimisticCursor,
            input: nextInput,
            cursor: nextCursor,
            action,
        };
        const base = optimisticInput;
        optimisticInput = action?.key === "Enter" ? "" : nextInput;
        optimisticCursor = action?.key === "Enter" ? 0 : nextCursor;
        if (action?.key === "Enter") setDraft("");
        awaiting = {
            base,
            action: Boolean(action),
            responseDone: false,
            revision: props.revision ?? 0,
        };
        mutations = mutations
            .then(() => props.onUpdate?.(update))
            .then(() => {
                if (awaiting) awaiting.responseDone = true;
                applyRemote();
            })
            .catch(() => {
                awaiting = undefined;
                optimisticInput = props.chat.input;
                optimisticCursor = props.chat.inputCursor;
                setDraft(props.chat.input);
            });
        return mutations;
    };

    onMount(() => {
        props.bindInput?.(input);
        onCleanup(() => props.bindInput?.(undefined));
        resize();
        if (!props.onWidth) return;
        let previous = 0;
        const report = () => {
            const columns = chatInputColumns(input);
            if (columns > 0 && columns !== previous) {
                previous = columns;
                props.onWidth?.(columns);
            }
        };
        const observer = new ResizeObserver(report);
        observer.observe(input);
        report();
        onCleanup(() => observer.disconnect());
    });

    return (
        <div
            class="chat-composer"
            classList={{ waiting: props.chat.waiting }}
            aria-busy={props.chat.waiting}
        >
            <Show when={props.chat.pendingDiffComments}>
                <details class="chat-diff-comments">
                    <summary>
                        Attached diff comments{" "}
                        <button
                            type="button"
                            aria-label="Remove diff comments attachment"
                            onClick={(event) => {
                                event.preventDefault();
                                props.onRemoveDiffComments?.();
                            }}
                        >
                            ×
                        </button>
                    </summary>
                    <pre>{props.chat.pendingDiffComments}</pre>
                </details>
            </Show>
            <Show when={props.chat.pendingImages.length}>
                <div
                    class="chat-attachments"
                    aria-label="Pending image attachments"
                >
                    <For each={props.chat.pendingImages}>
                        {(name, index) => (
                            <span class="chat-attachment" title={name}>
                                <Icon name="attach" size={16} />
                                <span>{name}</span>
                                <IconButton
                                    icon="close"
                                    size={16}
                                    label={`Remove ${name}`}
                                    onClick={() => {
                                        props.onRemoveImage?.(index());
                                        queueMicrotask(() => input.focus());
                                    }}
                                />
                            </span>
                        )}
                    </For>
                </div>
            </Show>
            <textarea
                ref={input!}
                class="chat-input"
                aria-label="AI chat input"
                value={draft()}
                placeholder={
                    props.chat.externalQuestion
                        ? "Answer Claude’s question…"
                        : props.chat.externalAgent
                          ? "Ask Claude about this code…"
                          : "Ask Ovim about this code…"
                }
                rows={2}
                autocomplete="off"
                autocapitalize="off"
                spellcheck={false}
                onInput={(event) => {
                    const target = event.currentTarget;
                    setDraft(target.value);
                    resize();
                    void publish(target.value, selectionFocus());
                }}
                onSelect={(event) => {
                    const target = event.currentTarget;
                    const cursor = utf8OffsetFromTextArea(
                        target.value,
                        selectionFocus(),
                    );
                    if (
                        target.value === optimisticInput &&
                        cursor !== optimisticCursor
                    )
                        void publish(target.value, selectionFocus());
                }}
                onKeyDown={(event) => {
                    if (event.isComposing) return;
                    const submit = event.key === "Enter" && !event.shiftKey;
                    const coreAction =
                        submit || event.key === "Tab" || event.key === "Escape";
                    if (!coreAction) return;
                    event.preventDefault();
                    const target = event.currentTarget;
                    void publish(
                        target.value,
                        selectionFocus(),
                        guiKeyInput(event),
                    );
                }}
            />
            <footer>
                <span>
                    {props.chat.externalQuestion
                        ? "Enter to answer · Esc to cancel"
                        : hasActiveRun()
                          ? "working"
                          : "Enter to send · drop images to attach · Esc to return"}
                </span>
                <span class="chat-composer-actions">
                    <b>{props.chat.reasoningEffort}</b>
                    <IconButton
                        class="chat-submit"
                        icon={hasActiveRun() ? "stop" : "send"}
                        size={16}
                        label={
                            hasActiveRun() ? "Stop generation" : "Send message"
                        }
                        disabled={
                            !hasActiveRun() &&
                            !draft().trim() &&
                            props.chat.pendingImages.length === 0
                        }
                        onClick={() => {
                            const key = hasActiveRun() ? "Escape" : "Enter";
                            void publish(input.value, selectionFocus(), {
                                key,
                                shift: false,
                                control: false,
                                alt: false,
                                meta: false,
                            });
                            queueMicrotask(() => input.focus());
                        }}
                    />
                </span>
            </footer>
        </div>
    );
}
