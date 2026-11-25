import { createEffect, createSignal, For, Show } from "solid-js";
import type { GuiPickerPreview } from "./types";

/** The code pane beside the picker results: the selected file around the
 *  matched line, coloured like the editor. */
export default function PickerPreview(props: {
    preview?: GuiPickerPreview;
    /** The picker has a result to preview. */
    hasSelection: boolean;
    syntax: Record<string, string>;
}) {
    let code: HTMLDivElement | undefined;
    // While the selected file loads, keep the last one on screen, dimmed,
    // instead of flashing a loading message on every move.
    const [lastShown, setLastShown] = createSignal<GuiPickerPreview>();
    createEffect(() => {
        if (props.preview) setLastShown(props.preview);
    });
    const shown = () =>
        props.preview ?? (props.hasSelection ? lastShown() : undefined);
    const stale = () => !props.preview || props.preview.stale;

    // Keep the matched line in the middle of the pane. A file result starts
    // at its first line.
    createEffect(() => {
        const preview = shown();
        if (!code || !preview) return;
        const target = code.querySelector<HTMLElement>(".target");
        if (target) {
            code.scrollTop =
                target.offsetTop -
                code.clientHeight / 2 +
                target.offsetHeight / 2;
        } else {
            code.scrollTop = 0;
        }
    });

    return (
        <aside
            class="picker-preview"
            classList={{ stale: Boolean(shown()) && stale() }}
            aria-label="Preview"
        >
            <Show
                when={shown()}
                fallback={
                    <p class="picker-preview-empty">
                        {props.hasSelection
                            ? "Loading preview…"
                            : "No file selected"}
                    </p>
                }
            >
                {(preview) => (
                    <>
                        <header>
                            <span>{preview().path}</span>
                            <Show when={preview().targetLine}>
                                {(line) => <small>line {line()}</small>}
                            </Show>
                        </header>
                        <div class="picker-preview-code" ref={code}>
                            <For each={preview().lines}>
                                {(line) => (
                                    <div
                                        class="picker-preview-line"
                                        classList={{ target: line.target }}
                                    >
                                        <span class="picker-preview-number">
                                            {line.number}
                                        </span>
                                        <span class="picker-preview-text">
                                            <For each={line.segments}>
                                                {(segment) => (
                                                    <span
                                                        style={{
                                                            color: segment.token
                                                                ? props.syntax[
                                                                      segment
                                                                          .token
                                                                  ]
                                                                : undefined,
                                                        }}
                                                    >
                                                        {segment.text}
                                                    </span>
                                                )}
                                            </For>
                                        </span>
                                    </div>
                                )}
                            </For>
                        </div>
                    </>
                )}
            </Show>
        </aside>
    );
}
