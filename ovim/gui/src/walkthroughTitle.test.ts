import { expect, it } from "vitest";
import { walkthroughPath } from "./walkthroughTitle";
it("keeps filenames and nearest context before distant directories", () => {
    expect(walkthroughPath("src/main.rs", 30)).toBe("src/main.rs");
    expect(walkthroughPath("packages/editor/src/deep/input/main.rs", 30)).toBe(
        "packages/…/input/main.rs",
    );
    expect(walkthroughPath("packages/editor/src/deep/input/main.rs", 20)).toBe(
        "…/input/main.rs",
    );
});
it("shortens Unicode filenames while keeping the suffix", () => {
    const title = walkthroughPath(`src/${"解析".repeat(50)}.test.ts`, 30);
    expect(Array.from(title).length).toBeLessThanOrEqual(30);
    expect(title).toMatch(/^…\/解析/);
    expect(title).toMatch(/\.test\.ts$/);
    expect(title).not.toContain("�");
});
