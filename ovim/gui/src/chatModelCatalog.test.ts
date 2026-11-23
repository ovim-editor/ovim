import { describe, expect, it } from "vitest";
import {
    chatModelOptionLabel,
    findChatModelChoice,
    groupChatModels,
} from "./chatModelCatalog";

describe("chat model catalog", () => {
    const providers = groupChatModels([
        {
            id: "codex_sol",
            label: "Sol",
            provider: "codex",
            model: "gpt-6-sol",
        },
        {
            id: "codex_terra",
            label: "Terra",
            provider: "codex",
            model: "gpt-5.6-terra",
        },
        {
            id: "claude_code",
            label: "Claude Agent",
            provider: "claude_code",
            model: "default",
        },
        {
            id: "claude_code",
            label: "Claude Agent",
            provider: "claude_code",
            model: "claude-opus-5-5",
        },
    ]);

    it("groups selectable tuples by provider while retaining profile identity", () => {
        expect(providers.map(({ id, label }) => ({ id, label }))).toEqual([
            { id: "codex", label: "Codex" },
            { id: "claude_code", label: "Claude Code" },
        ]);
        expect(providers[0].models).toMatchObject([
            { profileId: "codex_sol", model: "gpt-6-sol" },
            { profileId: "codex_terra", model: "gpt-5.6-terra" },
        ]);
    });

    it("finds the exact profile/model tuple and keeps configuration visible", () => {
        const selected = findChatModelChoice(
            providers,
            "codex_terra",
            "gpt-5.6-terra",
        )!;
        expect(selected.profileId).toBe("codex_terra");
        expect(chatModelOptionLabel(selected, providers[0].models)).toBe(
            "gpt-5.6-terra — Terra",
        );
    });
});
