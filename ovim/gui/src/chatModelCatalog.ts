import type { GuiAiProfileOption } from "./types";

export interface ChatModelChoice {
    key: string;
    profileId: string;
    profileLabel: string;
    model: string;
}

export interface ChatProviderChoice {
    id: string;
    label: string;
    models: ChatModelChoice[];
}

const PROVIDER_LABELS: Record<string, string> = {
    anthropic: "Anthropic",
    claude_code: "Claude Code",
    codex: "Codex",
    codex_app_server: "Codex app server",
    ollama: "Ollama",
    openai: "OpenAI",
};

export function providerLabel(provider: string): string {
    return (
        PROVIDER_LABELS[provider] ??
        provider
            .split("_")
            .filter(Boolean)
            .map((part) => part[0]?.toUpperCase() + part.slice(1))
            .join(" ")
    );
}

function modelKey(profileId: string, model: string): string {
    return JSON.stringify([profileId, model]);
}

/**
 * Builds the human-facing hierarchy without weakening the profile-backed
 * runtime contract. Every model choice retains the exact profile/model tuple
 * that core validates and persists.
 */
export function groupChatModels(
    options: GuiAiProfileOption[],
): ChatProviderChoice[] {
    const providers = new Map<string, ChatProviderChoice>();

    for (const option of options) {
        let provider = providers.get(option.provider);
        if (!provider) {
            provider = {
                id: option.provider,
                label: providerLabel(option.provider),
                models: [],
            };
            providers.set(option.provider, provider);
        }
        provider.models.push({
            key: modelKey(option.id, option.model),
            profileId: option.id,
            profileLabel: option.label ?? option.id,
            model: option.model,
        });
    }

    return [...providers.values()];
}

export function findChatModelChoice(
    providers: ChatProviderChoice[],
    profileId: string,
    model?: string,
): ChatModelChoice | undefined {
    const profileModels = providers
        .flatMap((provider) => provider.models)
        .filter((choice) => choice.profileId === profileId);
    return (
        profileModels.find((choice) => !model || choice.model === model) ??
        profileModels[0]
    );
}

export function chatModelOptionLabel(
    choice: ChatModelChoice,
    siblings: ChatModelChoice[],
): string {
    const modelIsAmbiguous =
        siblings.filter((sibling) => sibling.model === choice.model).length > 1;
    const profileAddsContext =
        new Set(siblings.map((sibling) => sibling.profileId)).size > 1;
    return modelIsAmbiguous || profileAddsContext
        ? `${choice.model} — ${choice.profileLabel}`
        : choice.model;
}
