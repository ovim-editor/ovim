import type { FlowDiffLine } from "./FlowDiffModel";

/** Lines must share at least this fraction of their tokens to be called a
 * replacement of each other. Anything below reads as an unrelated line. */
export const PAIR_SIMILARITY = 0.5;
/** Above this many line pairs the exact alignment is skipped; scoring every
 * pair of a huge rewritten block would stall the UI for no visible gain. */
const MAX_PAIR_CELLS = 40_000;

const TOKEN = /[\p{L}\p{N}_]+|[^\s]/gu;
const WORD = /^[\p{L}\p{N}_]/u;

/** Identifiers and numbers carry a line's meaning; punctuation is shared by
 * almost every line of code, so it only counts when a line has no words. */
function tokens(text: string) {
    const words = new Map<string, number>();
    const all = new Map<string, number>();
    for (const token of text.match(TOKEN) ?? []) {
        all.set(token, (all.get(token) ?? 0) + 1);
        if (WORD.test(token)) words.set(token, (words.get(token) ?? 0) + 1);
    }
    return { words, all };
}

function size(counts: Map<string, number>): number {
    let total = 0;
    for (const count of counts.values()) total += count;
    return total;
}

/** Dice coefficient over word and punctuation tokens, ignoring whitespace. */
export function lineSimilarity(a: string, b: string): number {
    if (a === b) return 1;
    return tokenSimilarity(tokens(a), tokens(b));
}

type Tokens = ReturnType<typeof tokens>;

function tokenSimilarity(a: Tokens, b: Tokens) {
    const useWords = a.words.size > 0 && b.words.size > 0;
    return countSimilarity(
        useWords ? a.words : a.all,
        useWords ? b.words : b.all,
    );
}

function countSimilarity(a: Map<string, number>, b: Map<string, number>) {
    const total = size(a) + size(b);
    if (!total) return 1;
    let shared = 0;
    for (const [token, count] of a)
        shared += Math.min(count, b.get(token) ?? 0);
    return (2 * shared) / total;
}

export type LinePairing = {
    /** For each removed line, the added line it was edited into. */
    left: Array<FlowDiffLine | undefined>;
    /** For each added line, the removed line it was edited from. */
    right: Array<FlowDiffLine | undefined>;
};

/**
 * Pair removed and added lines of a replaced block by similarity, keeping
 * their order. A line inserted at the top of the block then no longer shifts
 * every later pair by one, and unrelated lines stay unpaired.
 */
export function pairReplacementLines(
    removed: FlowDiffLine[],
    added: FlowDiffLine[],
): LinePairing {
    const left: LinePairing["left"] = new Array(removed.length).fill(undefined);
    const right: LinePairing["right"] = new Array(added.length).fill(undefined);
    const rows = removed.length;
    const cols = added.length;
    if (!rows || !cols) return { left, right };
    const removedTokens = removed.map((line) => tokens(line.text));
    const addedTokens = added.map((line) => tokens(line.text));
    const score = (i: number, j: number) => {
        if (removed[i].text === added[j].text) return 1;
        const similarity = tokenSimilarity(removedTokens[i], addedTokens[j]);
        return similarity >= PAIR_SIMILARITY ? similarity : 0;
    };
    const link = (i: number, j: number) => {
        left[i] = added[j];
        right[j] = removed[i];
    };
    if (rows * cols > MAX_PAIR_CELLS) {
        // Bounded fallback: positional pairing, still gated by similarity.
        for (let i = 0; i < Math.min(rows, cols); i++)
            if (score(i, i) > 0) link(i, i);
        return { left, right };
    }
    // best[i][j]: highest total similarity pairing removed[i..] with added[j..].
    const width = cols + 1;
    const best = new Float64Array((rows + 1) * width);
    for (let i = rows - 1; i >= 0; i--) {
        for (let j = cols - 1; j >= 0; j--) {
            const paired = score(i, j);
            best[i * width + j] = Math.max(
                best[(i + 1) * width + j],
                best[i * width + j + 1],
                paired > 0 ? paired + best[(i + 1) * width + j + 1] : 0,
            );
        }
    }
    let i = 0;
    let j = 0;
    while (i < rows && j < cols) {
        const paired = score(i, j);
        const here = best[i * width + j];
        if (
            paired > 0 &&
            Math.abs(here - (paired + best[(i + 1) * width + j + 1])) < 1e-9
        ) {
            link(i, j);
            i++;
            j++;
        } else if (Math.abs(here - best[(i + 1) * width + j]) < 1e-9) i++;
        else j++;
    }
    return { left, right };
}

const cache = new WeakMap<
    FlowDiffLine[],
    { right: FlowDiffLine[]; pairing: LinePairing }
>();

/** Memoized per section: every rendered line asks for its counterpart. */
export function sectionPairing(
    removed: FlowDiffLine[],
    added: FlowDiffLine[],
): LinePairing {
    const hit = cache.get(removed);
    if (hit && hit.right === added) return hit.pairing;
    const pairing = pairReplacementLines(removed, added);
    cache.set(removed, { right: added, pairing });
    return pairing;
}
