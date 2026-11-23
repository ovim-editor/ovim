import { describe, expect, it } from "vitest";
import {
    readWorkbenchLayout,
    workspaceLayoutIdentity,
    writeWorkbenchLayout,
} from "./layoutPersistence";

describe("workspace layout persistence", () => {
    it("derives a stable workspace path on Unix and Windows", () => {
        expect(
            workspaceLayoutIdentity({
                filePath: "/work/ovim/ovim/src/gui/mod.rs",
                workspacePath: "/work/ovim",
                projectName: "ovim",
            }),
        ).toBe("/work/ovim");
        expect(
            workspaceLayoutIdentity({
                filePath: "C:\\work\\ovim\\src\\main.rs",
                workspacePath: "C:\\work\\ovim",
                projectName: "ovim",
            }),
        ).toBe("C:/work/ovim");
    });

    it("round-trips valid preferences and ignores corrupt storage", () => {
        const values = new Map<string, string>();
        const storage = {
            getItem: (key: string) => values.get(key) ?? null,
            setItem: (key: string, value: string) => values.set(key, value),
        };
        const preference = {
            activeDock: "context" as const,
            activeContextPanel: "debug" as const,
        };

        writeWorkbenchLayout(storage, "/work/ovim", preference);
        expect(readWorkbenchLayout(storage, "/work/ovim")).toEqual(preference);

        values.set("ovim.gui.layout.v1.%2Fwork%2Fbroken", "not json");
        expect(readWorkbenchLayout(storage, "/work/broken")).toBeUndefined();
    });

    it("persists the terminal context tab and migrates old diff layouts", () => {
        const values = new Map<string, string>();
        const storage = {
            getItem: (key: string) => values.get(key) ?? null,
            setItem: (key: string, value: string) => values.set(key, value),
        };
        const preference = {
            activeDock: "context" as const,
            activeContextPanel: "terminal" as const,
        };
        writeWorkbenchLayout(storage, "/work/ovim", preference);
        expect(readWorkbenchLayout(storage, "/work/ovim")).toEqual(preference);
        expect(
            readWorkbenchLayout(
                {
                    getItem: () =>
                        JSON.stringify({
                            activeDock: "context",
                            activeContextPanel: "diff",
                        }),
                },
                "/work/ovim",
            ),
        ).toEqual({ activeDock: "explorer", activeContextPanel: "ai" });
    });
});

it("reads legacy layouts and discards only invalid widths", () => {
    const base = { activeDock: "explorer", activeContextPanel: "ai" };
    for (const explorerWidth of [-1, 0, 239, 801, "320", null]) {
        expect(
            readWorkbenchLayout(
                { getItem: () => JSON.stringify({ ...base, explorerWidth }) },
                "workspace",
            ),
        ).toEqual(base);
    }
    expect(
        readWorkbenchLayout(
            { getItem: () => JSON.stringify({ ...base, explorerWidth: 480 }) },
            "workspace",
        ),
    ).toEqual({ ...base, explorerWidth: 480 });
});

it("persists context width and ignores invalid stored values", () => {
    const base = {
        activeDock: "context" as const,
        activeContextPanel: "tests" as const,
    };
    for (const contextWidth of [-1, 0, 279, 801, "480", null]) {
        expect(
            readWorkbenchLayout(
                { getItem: () => JSON.stringify({ ...base, contextWidth }) },
                "workspace",
            ),
        ).toEqual(base);
    }
    expect(
        readWorkbenchLayout(
            { getItem: () => JSON.stringify({ ...base, contextWidth: 600 }) },
            "workspace",
        ),
    ).toEqual({ ...base, contextWidth: 600 });
});
