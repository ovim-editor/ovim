/** @vitest-environment jsdom */
import { cleanup, fireEvent, render, screen } from "@solidjs/testing-library";
import { createSignal } from "solid-js";
import { afterEach, expect, it, vi } from "vitest";
import ChatModelPicker from "./ChatModelPicker";

afterEach(cleanup);

it("selects a Claude model independently of its profile and displays the returned selection", async () => {
    const onProfile = vi.fn();
    const focusInput = vi.fn();
    const [model, setModel] = createSignal("default");
    render(() => (
        <ChatModelPicker
            profile="claude_code"
            model={model()}
            profiles={[
                "default",
                "claude-sonnet-5",
                "claude-opus-5",
                "claude-fable-5-1",
                "claude-haiku-4-5-20251001",
                "claude-custom-version[1m]",
            ].map((model) => ({
                id: "claude_code",
                label: "Claude Agent",
                provider: "claude_code",
                model,
            }))}
            reasoningEffort="default"
            reasoningEffortSelection="default"
            reasoningEfforts={["default", "high"]}
            onProfile={(profile, selectedModel) => {
                onProfile(profile, selectedModel);
                setModel(selectedModel!);
            }}
            focusInput={focusInput}
        />
    ));
    fireEvent.click(
        screen.getByRole("button", {
            name: /Claude Agent.*claude_code\/default/,
        }),
    );
    expect(screen.getAllByRole("option", { selected: true })).toHaveLength(1);
    fireEvent.click(screen.getByRole("option", { name: /claude-fable-5-1/ }));
    expect(onProfile).toHaveBeenCalledWith("claude_code", "claude-fable-5-1");
    await Promise.resolve();
    expect(focusInput).toHaveBeenCalledOnce();
    fireEvent.click(
        screen.getByRole("button", {
            name: /Claude Agent.*claude_code\/claude-fable-5-1/,
        }),
    );
    expect(
        screen.getByRole("option", { selected: true }).textContent,
    ).toContain("claude-fable-5-1");
    fireEvent.input(screen.getByLabelText("Model profile"), {
        target: { value: "custom-version" },
    });
    fireEvent.click(
        screen.getByRole("option", { name: /claude-custom-version/ }),
    );
    expect(onProfile).toHaveBeenLastCalledWith(
        "claude_code",
        "claude-custom-version[1m]",
    );
});
