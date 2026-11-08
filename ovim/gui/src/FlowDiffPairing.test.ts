import { describe, expect, it } from "vitest";
import {
    lineSimilarity,
    pairReplacementLines,
    sectionPairing,
} from "./FlowDiffPairing";
import type { FlowDiffLine } from "./FlowDiffModel";

const lines = (kind: "removed" | "added", texts: string[]): FlowDiffLine[] =>
    texts.map((text, index) => ({
        kind,
        text,
        [kind === "removed" ? "oldLine" : "newLine"]: index + 1,
    }));

describe("replacement line pairing", () => {
    it("scores token overlap, ignoring whitespace", () => {
        expect(lineSimilarity("foo(a, b);", "foo(a, c);")).toBeGreaterThan(0.6);
        expect(lineSimilarity("  foo();", "foo();")).toBe(1);
        expect(lineSimilarity("let x = 1;", "return value")).toBe(0);
    });

    it("does not shift every pair when a line is inserted at the top", () => {
        const removed = lines("removed", [
            "const alpha = compute(first);",
            "const beta = compute(second);",
            "return alpha + beta;",
        ]);
        const added = lines("added", [
            "// brand new comment",
            "const alpha = compute(first, extra);",
            "const beta = compute(second, extra);",
            "return alpha + beta + 1;",
        ]);
        const pairing = pairReplacementLines(removed, added);
        expect(pairing.right[0]).toBeUndefined();
        expect(pairing.left.map((line) => line?.text)).toEqual([
            added[1].text,
            added[2].text,
            added[3].text,
        ]);
        expect(pairing.right[1]).toBe(removed[0]);
    });

    it("leaves unrelated lines unpaired", () => {
        const pairing = pairReplacementLines(
            lines("removed", ["import fs from 'fs';"]),
            lines("added", ["export default function main() {}"]),
        );
        expect(pairing.left).toEqual([undefined]);
        expect(pairing.right).toEqual([undefined]);
    });

    it("keeps pairs in order and handles empty sides", () => {
        const removed = lines("removed", ["a(one);", "b(two);"]);
        const added = lines("added", ["b(two, 2);", "a(one, 1);"]);
        const pairing = pairReplacementLines(removed, added);
        // Crossing pairs are not allowed; exactly one survives.
        expect(pairing.left.filter(Boolean)).toHaveLength(1);
        expect(pairReplacementLines([], added).right).toEqual([
            undefined,
            undefined,
        ]);
    });

    it("pairs positionally with a threshold beyond the exact-alignment bound", () => {
        const texts = Array.from(
            { length: 250 },
            (_, i) => `value_${i} = go();`,
        );
        const removed = lines("removed", texts);
        const added = lines(
            "added",
            texts.map((text, i) => (i % 2 ? text : "completely different")),
        );
        const pairing = pairReplacementLines(removed, added);
        expect(pairing.left[1]).toBe(added[1]);
        expect(pairing.left[0]).toBeUndefined();
    });

    it("memoizes per section", () => {
        const removed = lines("removed", ["x(1);"]);
        const added = lines("added", ["x(2);"]);
        expect(sectionPairing(removed, added)).toBe(
            sectionPairing(removed, added),
        );
    });
});
