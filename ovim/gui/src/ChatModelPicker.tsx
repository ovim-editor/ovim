import {
    For,
    Show,
    createEffect,
    createMemo,
    createSignal,
    onCleanup,
    onMount,
} from "solid-js";
import type { GuiAiProfileOption } from "./types";
import { Icon } from "./Icon";
import { trapDialogFocus } from "./focus";
import {
    chatModelOptionLabel,
    findChatModelChoice,
    groupChatModels,
} from "./chatModelCatalog";

type PermissionMode = {
    id: string;
    label: string;
    description: string;
};

type Props = {
    profile: string;
    model?: string;
    profiles: GuiAiProfileOption[];
    reasoningEffort: string;
    reasoningEffortSelection: string;
    reasoningEffortDefault?: string;
    reasoningEfforts: string[];
    permissionMode?: string;
    permissionModes: PermissionMode[];
    /** True while a turn is running; core rejects run-setting changes then. */
    turnActive?: boolean;
    /** A handler may return a promise resolving to `false` when core rejected
     * the change, so the control can snap back to the live snapshot. */
    onProfile?: (profile: string, model?: string) => unknown;
    onReasoningEffort?: (effort: string) => unknown;
    onPermissionMode?: (mode: string) => unknown;
    focusInput: () => void;
};

const TURN_ACTIVE_TITLE =
    "Wait for or stop the active turn before changing run settings";

/** Run `revert` when a change handler reports that core rejected it. */
function revertIfRejected(result: unknown, revert: () => void) {
    void Promise.resolve(result).then((accepted) => {
        if (accepted === false) revert();
    });
}

function effortLabel(effort: string): string {
    return effort === "default"
        ? "Default"
        : effort[0]?.toUpperCase() + effort.slice(1);
}

export default function ChatModelPicker(props: Props) {
    const [open, setOpen] = createSignal(false);
    const [draftProvider, setDraftProvider] = createSignal("");
    const [popoverHeight, setPopoverHeight] = createSignal(620);
    let root!: HTMLDivElement;
    let trigger!: HTMLButtonElement;
    let firstSelect!: HTMLSelectElement;

    const providers = createMemo(() => groupChatModels(props.profiles));
    const selectedModel = createMemo(() =>
        findChatModelChoice(providers(), props.profile, props.model),
    );
    const selectedProvider = createMemo(() =>
        providers().find((provider) =>
            provider.models.some(
                (choice) => choice.profileId === selectedModel()?.profileId,
            ),
        ),
    );
    const activeProvider = createMemo(
        () =>
            providers().find((provider) => provider.id === draftProvider()) ??
            selectedProvider() ??
            providers()[0],
    );
    const activeModel = createMemo(() => {
        const selected = selectedModel();
        const provider = activeProvider();
        if (
            selected &&
            provider?.models.some((choice) => choice.key === selected.key)
        ) {
            return selected;
        }
        return provider?.models[0];
    });
    const permission = createMemo(() =>
        props.permissionModes.find(
            (option) => option.id === props.permissionMode,
        ),
    );
    const supportsReasoningEffort = createMemo(
        () =>
            props.reasoningEfforts.length > 1 ||
            props.reasoningEfforts[0] !== "default",
    );

    createEffect(() => {
        setDraftProvider(selectedProvider()?.id ?? providers()[0]?.id ?? "");
    });

    const measurePopover = () => {
        const bottom = trigger?.getBoundingClientRect().bottom ?? 0;
        setPopoverHeight(Math.max(180, window.innerHeight - bottom - 8));
    };

    const close = (returnToComposer = false) => {
        setOpen(false);
        queueMicrotask(
            returnToComposer ? props.focusInput : () => trigger.focus(),
        );
    };

    onMount(() => {
        const dismiss = (event: PointerEvent) => {
            if (!open() || root.contains(event.target as Node)) return;
            setOpen(false);
        };
        document.addEventListener("pointerdown", dismiss);
        window.addEventListener("resize", measurePopover);
        onCleanup(() => {
            document.removeEventListener("pointerdown", dismiss);
            window.removeEventListener("resize", measurePopover);
        });
    });

    return (
        <div class="chat-run-settings" ref={root!}>
            <button
                ref={trigger!}
                type="button"
                class="chat-run-trigger"
                title="Configure AI provider, model, and run settings"
                aria-haspopup="dialog"
                aria-expanded={open()}
                onClick={() => {
                    setOpen((value) => !value);
                    if (!open()) return;
                    measurePopover();
                    queueMicrotask(() => firstSelect.focus());
                }}
            >
                <span>
                    <b>{selectedProvider()?.label ?? props.profile}</b>
                    <small>{selectedModel()?.model ?? props.model}</small>
                </span>
                <em>
                    <Show when={supportsReasoningEffort()}>
                        {props.reasoningEffortSelection === "default" &&
                        props.reasoningEffort !== "default"
                            ? `Default · ${props.reasoningEffort}`
                            : effortLabel(props.reasoningEffort)}
                    </Show>
                    <Show when={supportsReasoningEffort() && permission()}>
                        {" · "}
                    </Show>
                    <Show when={permission()}>{(mode) => mode().label}</Show>
                </em>
                <Icon name="settings" size={16} />
            </button>

            <Show when={open()}>
                <section
                    class="chat-run-popover"
                    style={{ "max-height": `${popoverHeight()}px` }}
                    role="dialog"
                    aria-label="AI run settings"
                    onKeyDown={(event) => {
                        if (trapDialogFocus(event, event.currentTarget)) return;
                        if (event.key === "Escape") {
                            event.preventDefault();
                            close();
                        }
                    }}
                >
                    <header class="chat-run-popover-header">
                        <span>
                            <b>Run settings</b>
                            <small>Choose how this conversation runs.</small>
                        </span>
                        <Icon name="ai-spark" size={20} tone="accent" />
                    </header>

                    <div class="chat-setting-fields">
                        <label class="chat-setting-field">
                            <span>
                                <b>Provider</b>
                                <small>Runtime and account configuration</small>
                            </span>
                            <select
                                ref={firstSelect!}
                                aria-label="AI provider"
                                disabled={props.turnActive}
                                title={
                                    props.turnActive
                                        ? TURN_ACTIVE_TITLE
                                        : undefined
                                }
                                value={activeProvider()?.id ?? ""}
                                onChange={(event) => {
                                    const provider = providers().find(
                                        (candidate) =>
                                            candidate.id ===
                                            event.currentTarget.value,
                                    );
                                    const choice = provider?.models[0];
                                    if (!provider || !choice) return;
                                    setDraftProvider(provider.id);
                                    revertIfRejected(
                                        props.onProfile?.(
                                            choice.profileId,
                                            choice.model,
                                        ),
                                        () =>
                                            setDraftProvider(
                                                selectedProvider()?.id ??
                                                    providers()[0]?.id ??
                                                    "",
                                            ),
                                    );
                                }}
                            >
                                <For each={providers()}>
                                    {(provider) => (
                                        <option value={provider.id}>
                                            {provider.label}
                                        </option>
                                    )}
                                </For>
                            </select>
                        </label>

                        <label class="chat-setting-field">
                            <span>
                                <b>Model</b>
                                <small>Available for this provider</small>
                            </span>
                            <select
                                aria-label="AI model"
                                disabled={props.turnActive}
                                title={
                                    props.turnActive
                                        ? TURN_ACTIVE_TITLE
                                        : undefined
                                }
                                value={activeModel()?.key ?? ""}
                                onChange={(event) => {
                                    const select = event.currentTarget;
                                    const choice =
                                        activeProvider()?.models.find(
                                            (candidate) =>
                                                candidate.key ===
                                                event.currentTarget.value,
                                        );
                                    if (!choice) return;
                                    revertIfRejected(
                                        props.onProfile?.(
                                            choice.profileId,
                                            choice.model,
                                        ),
                                        () => {
                                            select.value =
                                                activeModel()?.key ?? "";
                                        },
                                    );
                                }}
                            >
                                <For each={activeProvider()?.models ?? []}>
                                    {(choice) => (
                                        <option value={choice.key}>
                                            {chatModelOptionLabel(
                                                choice,
                                                activeProvider()?.models ?? [],
                                            )}
                                        </option>
                                    )}
                                </For>
                            </select>
                        </label>

                        <Show when={supportsReasoningEffort()}>
                            <label class="chat-setting-field">
                                <span>
                                    <b>Reasoning effort</b>
                                    <small>
                                        Default uses{" "}
                                        {props.reasoningEffortDefault ??
                                            "the profile setting"}
                                    </small>
                                </span>
                                <select
                                    aria-label="Reasoning effort"
                                    disabled={props.turnActive}
                                    title={
                                        props.turnActive
                                            ? TURN_ACTIVE_TITLE
                                            : undefined
                                    }
                                    value={props.reasoningEffortSelection}
                                    onChange={(event) => {
                                        const select = event.currentTarget;
                                        revertIfRejected(
                                            props.onReasoningEffort?.(
                                                select.value,
                                            ),
                                            () => {
                                                select.value =
                                                    props.reasoningEffortSelection;
                                            },
                                        );
                                    }}
                                >
                                    <For each={props.reasoningEfforts}>
                                        {(effort) => (
                                            <option value={effort}>
                                                {effortLabel(effort)}
                                            </option>
                                        )}
                                    </For>
                                </select>
                            </label>
                        </Show>

                        <Show when={props.permissionModes.length > 0}>
                            <label class="chat-setting-field">
                                <span>
                                    <b>Permissions</b>
                                    <small>
                                        {permission()?.description ??
                                            "Provider approval behavior"}
                                    </small>
                                </span>
                                <select
                                    aria-label="Permission mode"
                                    disabled={props.turnActive}
                                    title={
                                        props.turnActive
                                            ? TURN_ACTIVE_TITLE
                                            : undefined
                                    }
                                    value={props.permissionMode}
                                    onChange={(event) => {
                                        const select = event.currentTarget;
                                        revertIfRejected(
                                            props.onPermissionMode?.(
                                                select.value,
                                            ),
                                            () => {
                                                select.value =
                                                    props.permissionMode ?? "";
                                            },
                                        );
                                    }}
                                >
                                    <For each={props.permissionModes}>
                                        {(option) => (
                                            <option value={option.id}>
                                                {option.label}
                                            </option>
                                        )}
                                    </For>
                                </select>
                            </label>
                        </Show>
                    </div>

                    <footer class="chat-run-popover-footer">
                        <small>
                            {props.turnActive
                                ? TURN_ACTIVE_TITLE
                                : "Profile and permissions are remembered; effort is per chat."}
                        </small>
                        <button type="button" onClick={() => close(true)}>
                            Done
                        </button>
                    </footer>
                </section>
            </Show>
        </div>
    );
}
