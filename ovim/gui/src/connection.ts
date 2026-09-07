import type { GuiConnection } from "./types";

/**
 * Whether a keystroke typed now can reach the editor.
 *
 * Keys typed while the link is down are dropped rather than queued. Queueing
 * reads as the kinder option and is the dangerous one: in a modal editor a key
 * means whatever the mode, pending operator, count and cursor position make it
 * mean at the moment it arrives, and the remote session keeps running while
 * this window cannot see it. Replaying `dd` against a buffer that moved
 * underneath deletes the wrong line with no way for the user to notice. The
 * indicator says the link is down while it is, so the loss is visible.
 */
export const acceptsInput = (connection: GuiConnection) =>
    connection.state === "connected";

export interface ConnectionAction {
    label: string;
    allowNewSession: boolean;
    /** Shown when the action costs something that cannot be undone. */
    warning?: string;
}

export interface ConnectionIndicator {
    tone: "reconnecting" | "lost";
    /** The short form, for the status line. */
    label: string;
    headline: string;
    detail: string;
    actions: ConnectionAction[];
}

const seconds = (milliseconds: number) =>
    Math.max(1, Math.round(milliseconds / 1000));

const tryAgain: ConnectionAction = {
    label: "Try again",
    allowNewSession: false,
};

/**
 * What to show about the link, or nothing at all while it is healthy.
 *
 * Quiet when healthy and unmissable when not: a working link gets no chrome,
 * and every other state gets a headline that says which of the four very
 * different problems this is. "Disconnected" alone would cover both a hiccup
 * that is already over and a session that has ended taking an afternoon's undo
 * history with it, and those want opposite reactions from the user.
 */
export const connectionIndicator = (
    connection: GuiConnection,
): ConnectionIndicator | undefined => {
    if (connection.state === "connected") return undefined;
    if (connection.state === "reconnecting")
        return {
            tone: "reconnecting",
            label: `reconnecting · attempt ${connection.attempt}`,
            headline: "Reconnecting to the remote session",
            detail: `${connection.detail} · next attempt in ${seconds(
                connection.retryInMs,
            )}s`,
            actions: [{ label: "Retry now", allowNewSession: false }],
        };

    switch (connection.reason) {
        case "sessionGone":
            return {
                tone: "lost",
                label: "session ended",
                headline: "The remote session has ended",
                // Named rather than hidden: a new session would come up
                // looking identical and holding none of this, so the user has
                // to know what they are choosing between.
                detail: `${connection.detail} Its open buffers, undo history and language servers went with it.`,
                actions: connection.canStartASession
                    ? [
                          tryAgain,
                          {
                              label: "Start a new session",
                              allowNewSession: true,
                              warning:
                                  "The old session's state is not recoverable.",
                          },
                      ]
                    : [tryAgain],
            };
        case "authentication":
            return {
                tone: "lost",
                label: "refused",
                headline: "The remote host refused the connection",
                detail: `${connection.detail} Retrying on its own will not change that.`,
                actions: [tryAgain],
            };
        case "unusable":
            return {
                tone: "lost",
                label: "unreachable",
                headline: "This Ovim cannot talk to the remote session",
                detail: connection.detail,
                actions: [tryAgain],
            };
        case "gaveUp":
        default:
            return {
                tone: "lost",
                label: "disconnected",
                headline: "The remote session is still unreachable",
                detail: `${connection.detail} It is no longer being retried automatically; the session on the far side may well still be running.`,
                actions: [tryAgain],
            };
    }
};
