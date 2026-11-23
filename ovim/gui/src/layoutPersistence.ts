import { CONTEXT_MIN_WIDTH, CONTEXT_MAX_WIDTH } from "./contextDockLayout";
import type { GuiSnapshot } from "./types";
import { EXPLORER_MIN_WIDTH, EXPLORER_MAX_WIDTH } from "./explorerLayout";

export type WorkbenchLayoutPreference = {
    explorerWidth?: number;
    contextWidth?: number;
    activeDock: "explorer" | "context";
    activeContextPanel: "ai" | "tests" | "debug" | "run" | "terminal";
};

export const workspaceLayoutIdentity = (
    snapshot: Pick<GuiSnapshot, "filePath" | "workspacePath" | "projectName">,
) => {
    const workspace = snapshot.workspacePath?.replaceAll("\\", "/");
    if (workspace) return workspace;
    const path = snapshot.filePath?.replaceAll("\\", "/");
    if (path) {
        const parts = path.split("/").filter(Boolean);
        const projectIndex = parts.lastIndexOf(snapshot.projectName);
        if (projectIndex >= 0) {
            const prefix = path.startsWith("/") ? "/" : "";
            return prefix + parts.slice(0, projectIndex + 1).join("/");
        }
    }
    return snapshot.projectName || "ovim";
};

const storageKey = (workspace: string) =>
    `ovim.gui.layout.v1.${encodeURIComponent(workspace)}`;

export const readWorkbenchLayout = (
    storage: Pick<Storage, "getItem"> | undefined,
    workspace: string,
): WorkbenchLayoutPreference | undefined => {
    if (!storage) return undefined;
    try {
        const stored = JSON.parse(
            storage.getItem(storageKey(workspace)) ?? "",
        ) as
            | (Omit<
                  Partial<WorkbenchLayoutPreference>,
                  "activeContextPanel"
              > & {
                  activeContextPanel?: string;
              })
            | undefined;
        const parsed: typeof stored =
            stored?.activeContextPanel === "diff"
                ? {
                      ...stored,
                      activeDock: "explorer",
                      activeContextPanel: "ai",
                  }
                : stored;
        if (
            !parsed ||
            !["explorer", "context"].includes(parsed.activeDock ?? "") ||
            !["ai", "tests", "debug", "run", "terminal"].includes(
                parsed.activeContextPanel ?? "",
            )
        )
            return undefined;
        return {
            activeDock: parsed.activeDock!,
            activeContextPanel:
                parsed.activeContextPanel as WorkbenchLayoutPreference["activeContextPanel"],
            ...(typeof parsed.contextWidth === "number" &&
            Number.isFinite(parsed.contextWidth) &&
            parsed.contextWidth >= CONTEXT_MIN_WIDTH &&
            parsed.contextWidth <= CONTEXT_MAX_WIDTH
                ? { contextWidth: parsed.contextWidth }
                : {}),
            ...(typeof parsed.explorerWidth === "number" &&
            Number.isFinite(parsed.explorerWidth) &&
            parsed.explorerWidth >= EXPLORER_MIN_WIDTH &&
            parsed.explorerWidth <= EXPLORER_MAX_WIDTH
                ? { explorerWidth: parsed.explorerWidth }
                : {}),
        };
    } catch {
        return undefined;
    }
};

export const writeWorkbenchLayout = (
    storage: Pick<Storage, "setItem"> | undefined,
    workspace: string,
    preference: WorkbenchLayoutPreference,
) => {
    if (!storage) return;
    try {
        storage.setItem(storageKey(workspace), JSON.stringify(preference));
    } catch {
        // Layout persistence is optional when storage is unavailable or full.
    }
};
