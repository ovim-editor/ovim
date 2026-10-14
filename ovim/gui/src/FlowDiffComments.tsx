import { createSignal, For, Show, type JSX } from "solid-js";
import type { GuiDiffComment } from "./types";

export type CommentTarget = { path: string; line: number; side: "old" | "new" };
export type CommentInput = CommentTarget & {
    before: boolean;
    text: string;
    id?: number;
};
export function createDiffComments(options: {
    comments: () => GuiDiffComment[];
    save?: (comment: CommentInput) => Promise<void>;
    remove?: (id: number) => void;
}) {
    const [selected, select] = createSignal<CommentTarget>();
    const [draft, setDraft] = createSignal<CommentInput>();
    const [saving, setSaving] = createSignal(false);
    const [error, setError] = createSignal("");
    const begin = (
        target: CommentTarget,
        before = false,
        comment?: GuiDiffComment,
    ) => {
        if (draft() || !options.save) return;
        select(target);
        setError("");
        setDraft({
            ...target,
            before,
            text: comment?.text ?? "",
            id: comment?.id,
        });
    };
    const focusReview = () => {
        const panel =
            document.activeElement?.closest<HTMLElement>(".flow-diff");
        queueMicrotask(() => panel?.focus({ preventScroll: true }));
    };
    const cancel = () => {
        if (!saving()) {
            focusReview();
            setDraft(undefined);
            setError("");
        }
    };
    const save = async () => {
        const value = draft();
        if (!value || saving()) return;
        if (!value.text.trim()) {
            cancel();
            return;
        }
        setSaving(true);
        setError("");
        try {
            await options.save?.(value);
            focusReview();
            setDraft(undefined);
        } catch (reason) {
            setError(String(reason));
        } finally {
            setSaving(false);
        }
    };
    const matches = (a: CommentTarget | undefined, b: CommentTarget) =>
        a?.path === b.path && a.line === b.line && a.side === b.side;
    return {
        ...options,
        enabled: () => Boolean(options.save),
        selected,
        select,
        draft,
        setDraft,
        saving,
        error,
        begin,
        cancel,
        save,
        matches,
    };
}
export type DiffCommentsController = ReturnType<typeof createDiffComments>;

export function CommentedLine(props: {
    comments?: DiffCommentsController;
    target?: CommentTarget;
    sourceText: string;
    children: JSX.Element;
}) {
    const context = props.comments;
    const target = () => props.target;
    const comments = (before: boolean) =>
        context
            ?.comments()
            .filter(
                (c) =>
                    target() &&
                    context.matches(c, target()!) &&
                    c.before === before &&
                    c.excerpt === props.sourceText,
            ) ?? [];
    const editor = (before: boolean) => (
        <Show
            when={
                target() &&
                context?.matches(context.draft(), target()!) &&
                context.draft()?.before === before
            }
        >
            <div
                class="flow-comment-editor"
                onClick={(e) => e.stopPropagation()}
            >
                <label>
                    Comment {before ? "above" : "below"} {target()!.path}:
                    {target()!.line}
                </label>
                <textarea
                    aria-label="Diff comment"
                    maxLength={32768}
                    value={context!.draft()!.text}
                    ref={(el) => queueMicrotask(() => el.focus())}
                    onInput={(e) =>
                        context!.setDraft(
                            (value) =>
                                value && {
                                    ...value,
                                    text: e.currentTarget.value,
                                },
                        )
                    }
                    onKeyDown={(e) => {
                        e.stopPropagation();
                        if (
                            e.key === "Escape" ||
                            (e.key === "Enter" && (e.ctrlKey || e.metaKey))
                        ) {
                            e.preventDefault();
                            void context!.save();
                        }
                    }}
                    disabled={context!.saving()}
                    placeholder="Ask about this change…"
                />
                <Show when={context!.error()}>
                    <p role="alert">{context!.error()}</p>
                </Show>
                <div class="flow-comment-actions">
                    <span>Esc or Ctrl+Enter saves · Enter adds a line</span>
                    <button
                        type="button"
                        disabled={context!.saving()}
                        onClick={() => context!.cancel()}
                    >
                        Cancel
                    </button>
                    <button
                        type="button"
                        disabled={context!.saving()}
                        onClick={() => void context!.save()}
                    >
                        {context!.saving() ? "Saving…" : "Save comment"}
                    </button>
                </div>
            </div>
        </Show>
    );
    const annotations = (before: boolean) => (
        <>
            <For each={comments(before)}>
                {(comment) => (
                    <Show when={context?.draft()?.id !== comment.id}>
                        <div
                            class="flow-user-comment"
                            aria-label={`Comment on ${comment.path}:${comment.line}`}
                        >
                            <span class="flow-comment-label">
                                Your comment ·{" "}
                                {comment.side === "old" ? "Old" : "New"} line{" "}
                                {comment.line}
                            </span>
                            <p>{comment.text}</p>
                            <div class="flow-comment-actions">
                                <button
                                    type="button"
                                    onClick={() =>
                                        context!.begin(
                                            comment,
                                            comment.before,
                                            comment,
                                        )
                                    }
                                >
                                    Edit
                                </button>
                                <button
                                    type="button"
                                    onClick={() =>
                                        context!.remove?.(comment.id)
                                    }
                                >
                                    Delete
                                </button>
                            </div>
                        </div>
                    </Show>
                )}
            </For>
            {editor(before)}
        </>
    );
    return (
        <div
            class="flow-commented-line"
            classList={{
                "comment-selected": Boolean(
                    target() && context?.matches(context.selected(), target()!),
                ),
            }}
            onClick={() => {
                if (target()) context?.select(target()!);
            }}
        >
            {annotations(true)}
            <div class="flow-comment-source">
                {props.children}
                <Show when={target() && context?.enabled()}>
                    <button
                        type="button"
                        class="flow-add-comment"
                        aria-label={`Comment on ${target()!.side} line ${target()!.line}`}
                        title="Add comment below (o) · above (O)"
                        onClick={(e) => {
                            e.stopPropagation();
                            context!.begin(target()!);
                        }}
                    >
                        +
                    </button>
                </Show>
            </div>
            {annotations(false)}
        </div>
    );
}
