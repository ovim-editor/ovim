/** @vitest-environment jsdom */
import {
    cleanup,
    fireEvent,
    render,
    screen,
    waitFor,
} from "@solidjs/testing-library";
import { createSignal } from "solid-js";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import ChatComposer, { type ChatInputUpdate } from "./ChatComposer";
import { mockSnapshot } from "./mock";
import type { GuiChatInputState } from "./types";

beforeEach(() => {
    vi.stubGlobal(
        "ResizeObserver",
        class {
            observe() {}
            disconnect() {}
        },
    );
});
afterEach(() => {
    cleanup();
    vi.unstubAllGlobals();
});

const deferred = () => {
    let resolve!: (state: GuiChatInputState) => void;
    const promise = new Promise<GuiChatInputState>((done) => {
        resolve = done;
    });
    return { promise, resolve };
};
const input = () =>
    screen.getByLabelText("AI chat input") as HTMLTextAreaElement;
const edit = (value: string) => {
    input().value = value;
    input().setSelectionRange(value.length, value.length);
    fireEvent.input(input());
};

describe("composer action acknowledgements", () => {
    it("restores a rejected or no-op submission without needing a new snapshot", async () => {
        const onUpdate = vi.fn(async (update: ChatInputUpdate) => ({
            input: update.input,
            cursor: update.cursor,
            revision: 7,
        }));
        render(() => (
            <ChatComposer
                chat={{
                    ...mockSnapshot.aiChat!,
                    input: "/unknown",
                    inputCursor: 8,
                }}
                revision={7}
                onUpdate={onUpdate}
            />
        ));
        fireEvent.keyDown(input(), { key: "Enter" });
        expect(input().value).toBe("");
        await waitFor(() => expect(input().value).toBe("/unknown"));
        expect(input().selectionStart).toBe(8);
        edit("/unknown corrected");
        await waitFor(() => expect(onUpdate).toHaveBeenCalledTimes(2));
        expect(onUpdate).toHaveBeenLastCalledWith(
            expect.objectContaining({
                expectedInput: "/unknown",
                expectedCursor: 8,
                input: "/unknown corrected",
            }),
        );
    });

    it("does not let an older edit response acknowledge a queued submit", async () => {
        const first = deferred(),
            second = deferred(),
            submit = deferred();
        const onUpdate = vi
            .fn()
            .mockReturnValueOnce(first.promise)
            .mockReturnValueOnce(second.promise)
            .mockReturnValueOnce(submit.promise);
        const [chat, setChat] = createSignal({
            ...mockSnapshot.aiChat!,
            input: "",
            inputCursor: 0,
        });
        const [revision, setRevision] = createSignal(1);
        render(() => (
            <ChatComposer
                chat={chat()}
                revision={revision()}
                onUpdate={onUpdate}
            />
        ));
        edit("a");
        await waitFor(() => expect(onUpdate).toHaveBeenCalledTimes(1));
        edit("ab");
        fireEvent.keyDown(input(), { key: "Enter" });
        first.resolve({ input: "a", cursor: 1, revision: 1 });
        setChat({ ...chat(), input: "a", inputCursor: 1 });
        setRevision(2);
        await waitFor(() => expect(onUpdate).toHaveBeenCalledTimes(2));
        expect(input().value).toBe("");
        second.resolve({ input: "ab", cursor: 2, revision: 2 });
        setChat({ ...chat(), input: "ab", inputCursor: 2 });
        setRevision(3);
        await waitFor(() => expect(onUpdate).toHaveBeenCalledTimes(3));
        expect(input().value).toBe("");
        submit.resolve({ input: "", cursor: 0, revision: 3 });
        setChat({ ...chat(), input: "", inputCursor: 0 });
        setRevision(4);
        await waitFor(() => expect(input().value).toBe(""));
    });

    it("rebases edits queued after a submission that retained the draft", async () => {
        const submit = deferred();
        const onUpdate = vi
            .fn()
            .mockReturnValueOnce(submit.promise)
            .mockImplementation(async (update: ChatInputUpdate) => ({
                input: update.input,
                cursor: update.cursor,
                revision: 2,
            }));
        render(() => (
            <ChatComposer
                chat={{
                    ...mockSnapshot.aiChat!,
                    input: "retained",
                    inputCursor: 8,
                }}
                revision={1}
                onUpdate={onUpdate}
            />
        ));
        fireEvent.keyDown(input(), { key: "Enter" });
        await waitFor(() => expect(onUpdate).toHaveBeenCalledTimes(1));
        edit("next draft");
        submit.resolve({ input: "retained", cursor: 8, revision: 1 });
        await waitFor(() => expect(onUpdate).toHaveBeenCalledTimes(2));
        expect(onUpdate).toHaveBeenLastCalledWith(
            expect.objectContaining({
                expectedInput: "retained",
                expectedCursor: 8,
                input: "next draft",
            }),
        );
        expect(input().value).toBe("next draft");
    });
});

describe("pending approvals", () => {
    it("never forwards Enter, Esc or Send while an approval card is shown", async () => {
        const onUpdate = vi.fn(async (update: ChatInputUpdate) => ({
            input: update.input,
            cursor: update.cursor,
            revision: 3,
        }));
        render(() => (
            <ChatComposer
                chat={{
                    ...mockSnapshot.aiChat!,
                    input: "",
                    inputCursor: 0,
                    approval: "Run `rm -rf build`?",
                }}
                revision={3}
                onUpdate={onUpdate}
            />
        ));
        edit("and then also");
        fireEvent.keyDown(input(), { key: "Enter" });
        fireEvent.keyDown(input(), { key: "Escape" });
        await waitFor(() => expect(onUpdate).toHaveBeenCalled());
        // Core resolves approvals from Enter/Esc; the draft must stay text.
        for (const [update] of onUpdate.mock.calls)
            expect(update.action).toBeUndefined();
        expect(input().value).toBe("and then also");
        expect(
            document.querySelector<HTMLButtonElement>(".chat-submit")!.disabled,
        ).toBe(true);
        expect(screen.getByText(/Approval pending/)).toBeTruthy();
    });
});
