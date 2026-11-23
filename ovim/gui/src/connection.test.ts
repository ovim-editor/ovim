import { describe, expect, it } from "vitest";
import { acceptsInput, connectionIndicator } from "./connection";

describe("the remote connection indicator", () => {
    it("shows nothing at all while the link is healthy", () => {
        expect(connectionIndicator({ state: "connected" })).toBeUndefined();
    });

    it("counts the attempts and says when the next one is", () => {
        const indicator = connectionIndicator({
            state: "reconnecting",
            attempt: 4,
            retryInMs: 4000,
            detail: "the stream failed",
        });

        expect(indicator?.tone).toBe("reconnecting");
        expect(indicator?.label).toBe("reconnecting · attempt 4");
        expect(indicator?.detail).toContain("next attempt in 4s");
    });

    it("tells a session that has ended apart from a host that said no", () => {
        // The two states a single boolean would flatten together, and they
        // want opposite reactions: one is worth waiting out, the other has
        // already taken the work with it.
        const ended = connectionIndicator({
            state: "lost",
            reason: "sessionGone",
            detail: "It ended.",
            canStartASession: true,
        });
        const refused = connectionIndicator({
            state: "lost",
            reason: "authentication",
            detail: "The key was refused.",
            canStartASession: true,
        });

        expect(ended?.headline).toBe("The remote session has ended");
        expect(ended?.detail).toContain("undo history");
        expect(refused?.headline).toBe(
            "The remote host refused the connection",
        );
        expect(refused?.detail).toContain("will not change that");
    });

    it("only offers to start a new session where one could be started", () => {
        // Starting one is the user answering a question this code must not
        // answer for them, and it is not even possible when the window did
        // not bring the session up.
        const owned = connectionIndicator({
            state: "lost",
            reason: "sessionGone",
            detail: "It ended.",
            canStartASession: true,
        });
        const borrowed = connectionIndicator({
            state: "lost",
            reason: "sessionGone",
            detail: "It ended.",
            canStartASession: false,
        });

        expect(owned?.actions.map((action) => action.allowNewSession)).toEqual([
            false,
            true,
        ]);
        expect(owned?.actions[1].warning).toContain("not recoverable");
        expect(
            borrowed?.actions.map((action) => action.allowNewSession),
        ).toEqual([false]);
    });

    it("always leaves a way back, whatever stopped the retrying", () => {
        // A state that can only be left by relaunching throws away the very
        // session the reconnection exists to protect.
        for (const reason of [
            "gaveUp",
            "sessionGone",
            "authentication",
            "unusable",
        ] as const) {
            const indicator = connectionIndicator({
                state: "lost",
                reason,
                detail: "detail",
                canStartASession: false,
            });
            expect(indicator?.actions.length).toBeGreaterThan(0);
        }
    });

    it("accepts input only while the editor is actually reachable", () => {
        expect(acceptsInput({ state: "connected" })).toBe(true);
        expect(
            acceptsInput({
                state: "reconnecting",
                attempt: 1,
                retryInMs: 500,
                detail: "",
            }),
        ).toBe(false);
        expect(
            acceptsInput({
                state: "lost",
                reason: "gaveUp",
                detail: "",
                canStartASession: false,
            }),
        ).toBe(false);
    });
});
