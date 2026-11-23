/** @vitest-environment jsdom */

import { fireEvent, render, screen, within } from "@solidjs/testing-library";
import { afterEach, describe, expect, it, vi } from "vitest";
import WorkbenchTabStrip from "./WorkbenchTabStrip";
import type { BrowserSession } from "./browserProtocol";
import type { WorkbenchTabReference } from "./workbench";

afterEach(() => document.body.replaceChildren());

const source = (id: number, title: string, modified = false) => ({
    id,
    index: id - 1,
    title,
    active: id === 1,
    modified,
});

const session = (id: string, url: string): BrowserSession => ({
    sessionId: id,
    url,
    title: "",
    visible: true,
    loading: false,
    documentId: 1,
    vimKeysEnabled: true,
    keyMode: "normal",
});

function strip(
    overrides: Partial<Parameters<typeof WorkbenchTabStrip>[0]> = {},
) {
    const props = {
        onCloseSource: vi.fn(),
        onCloseBrowser: vi.fn(),
        onRestoreBrowser: vi.fn(),
        onNewBrowser: vi.fn(),
    };
    const sourceTabs = [source(1, "main.rs"), source(2, "lib.rs", true)];
    const tabs: WorkbenchTabReference[] = [
        { id: "source:1", kind: "source", index: 0, tabId: 1 },
        { id: "source:2", kind: "source", index: 1, tabId: 2 },
        { id: "browser:b1", kind: "browser", sessionId: "b1" },
    ];
    const result = render(() => (
        <WorkbenchTabStrip
            native
            sourceTabs={sourceTabs}
            tabs={tabs}
            selection={{ kind: "source", tabId: 1 }}
            browserState={{
                revision: 3,
                sessions: [session("b1", "https://example.com/a")],
                maxSessions: 8,
            }}
            browserOpening={false}
            canRestoreBrowser
            onSelect={vi.fn()}
            onSourceFocus={vi.fn()}
            onNavigate={vi.fn()}
            {...props}
            {...overrides}
        />
    ));
    return { ...props, result };
}

describe("WorkbenchTabStrip browser recovery", () => {
    it("offers one-click closed-tab recovery when history is available", () => {
        const { onRestoreBrowser, result } = strip();
        try {
            fireEvent.click(
                screen.getByRole("button", {
                    name: "Restore closed browser tab",
                }),
            );
            expect(onRestoreBrowser).toHaveBeenCalledOnce();
        } finally {
            result.unmount();
        }
    });
});

describe("WorkbenchTabStrip structure", () => {
    it("keeps the restore and new-tab buttons out of the tablist", () => {
        const { result } = strip();
        try {
            const list = screen.getByRole("tablist", { name: "Open tabs" });
            expect(within(list).getAllByRole("tab")).toHaveLength(3);
            for (const name of [
                "Restore closed browser tab",
                "New browser tab",
            ])
                expect(within(list).queryByRole("button", { name })).toBeNull();
            expect(
                screen.getByRole("button", { name: "New browser tab" }),
            ).toBeTruthy();
            // Tab controls never nest inside the tab button.
            for (const tab of screen.getAllByRole("tab"))
                expect(tab.querySelector("button")).toBeNull();
        } finally {
            result.unmount();
        }
    });
});

describe("WorkbenchTabStrip closing", () => {
    it("closes a source tab from its button without selecting it", () => {
        const onSelect = vi.fn();
        const { onCloseSource, result } = strip({ onSelect });
        try {
            fireEvent.click(
                screen.getByRole("button", { name: "Close lib.rs" }),
            );
            expect(onCloseSource).toHaveBeenCalledWith(
                expect.objectContaining({ id: 2, index: 1, modified: true }),
            );
            expect(onSelect).not.toHaveBeenCalled();
        } finally {
            result.unmount();
        }
    });

    it("closes a browser tab from its button", () => {
        const { onCloseBrowser, result } = strip();
        try {
            fireEvent.click(
                screen.getByRole("button", {
                    name: "Close example.com",
                }),
            );
            expect(onCloseBrowser).toHaveBeenCalledWith("b1");
        } finally {
            result.unmount();
        }
    });

    it("closes on middle click only", () => {
        const { onCloseSource, onCloseBrowser, result } = strip();
        try {
            const tabs = screen.getAllByRole("tab");
            fireEvent(
                tabs[0],
                new MouseEvent("auxclick", { bubbles: true, button: 2 }),
            );
            expect(onCloseSource).not.toHaveBeenCalled();
            fireEvent(
                tabs[0],
                new MouseEvent("auxclick", { bubbles: true, button: 1 }),
            );
            expect(onCloseSource).toHaveBeenCalledWith(
                expect.objectContaining({ id: 1 }),
            );
            fireEvent(
                tabs[2],
                new MouseEvent("auxclick", { bubbles: true, button: 1 }),
            );
            expect(onCloseBrowser).toHaveBeenCalledWith("b1");
        } finally {
            result.unmount();
        }
    });

    it("never offers to close the only source tab", () => {
        const { onCloseSource, result } = strip({
            sourceTabs: [source(1, "main.rs")],
            tabs: [{ id: "source:1", kind: "source", index: 0, tabId: 1 }],
        });
        try {
            expect(
                screen.queryByRole("button", { name: "Close main.rs" }),
            ).toBeNull();
            fireEvent(
                screen.getByRole("tab", { name: "main.rs" }),
                new MouseEvent("auxclick", { bubbles: true, button: 1 }),
            );
            expect(onCloseSource).not.toHaveBeenCalled();
        } finally {
            result.unmount();
        }
    });
});
