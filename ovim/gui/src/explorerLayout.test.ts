import { describe, expect, it } from "vitest";
import { explorerWidthLimit, middleEllipsis } from "./explorerLayout";

describe("explorer layout", () => {
    it("preserves complete names when they fit and both ends when they do not", () => {
        const measure = (text: string) => Array.from(text).length * 10;
        expect(middleEllipsis("index.ts", 80, measure)).toBe("index.ts");
        expect(middleEllipsis("long-module-name.ts", 90, measure)).toBe(
            "long…e.ts",
        );
        expect(middleEllipsis("🌲🌲module.ts", 70, measure)).toBe("🌲🌲m….ts");
    });
    it("uses actual text measurements", () => {
        const measure = (text: string) =>
            Array.from(text).reduce(
                (sum, char) => sum + (char === "W" ? 20 : 5),
                0,
            );
        const result = middleEllipsis("WWWiiiiii.ts", 65, measure);
        expect(measure(result)).toBeLessThanOrEqual(65);
        expect(result).toContain("…");
        expect(result.startsWith("W")).toBe(true);
        expect(result.endsWith("s")).toBe(true);
    });
    it("reserves editor space when docked and clamps overlays to the window", () => {
        expect(explorerWidthLimit(1440)).toBe(800);
        expect(explorerWidthLimit(1100)).toBe(560);
        expect(explorerWidthLimit(1440, 490)).toBe(410);
        expect(explorerWidthLimit(720)).toBe(660);
    });
});
