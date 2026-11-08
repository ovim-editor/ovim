import { expect, test } from "@playwright/test";

// Identical source with different cursor and syntax boundaries. Browser Range
// measurements check actual glyph centers against an unsegmented reference.
for (const mode of ["NORMAL", "INSERT"]) {
    test(`text stays on the same grid across ${mode} cursor and highlight boundaries`, async ({
        page,
    }) => {
        await page.route("**/src/mock.ts", async (route) => {
            const response = await route.fetch();
            await route.fulfill({
                response,
                body:
                    (await response.text()) +
                    `\n
const sample = "const result = value !== other;    return result;";
mockSnapshot.mode = ${JSON.stringify(mode)};
mockSnapshot.lines.splice(0, mockSnapshot.lines.length, ...[0, 6, 20, -1].map((cursor, row) => ({
    number: row + 1, continuation: false, displayStart: 0, current: cursor >= 0,
    segments: Array.from(sample).reduce((segments, text, index) => {
        const token = index < 5 ? "keyword" : index < 13 ? "variable" : undefined;
        const previous = segments.at(-1);
        if (previous && !previous.cursor && index !== cursor && previous.token === token) {
            previous.text += text; previous.cells++;
        } else segments.push({text, cells: 1, token, cursor: index === cursor, selected: false, searchMatch: false});
        return segments;
    }, [])
})));`,
            });
        });
        await page.goto("/");
        await expect(page.locator(".code-line")).toHaveCount(4);
        await page.evaluate(() => document.fonts.ready);
        const measure = () =>
            page.locator(".line-content").evaluateAll((lines) => {
                const reference = document.createElement("span");
                reference.textContent = lines[0].textContent;
                reference.style.cssText =
                    "position:absolute;white-space:pre;left:0";
                lines[0].append(reference);

                const positions = (element: Element) => {
                    const origin = element.getBoundingClientRect().left;
                    const walker = document.createTreeWalker(
                        element,
                        NodeFilter.SHOW_TEXT,
                    );
                    const xs: number[] = [];
                    while (walker.nextNode()) {
                        if (
                            reference.contains(walker.currentNode) &&
                            element !== reference
                        )
                            continue;
                        const node = walker.currentNode;
                        for (let i = 0; i < node.textContent!.length; i++) {
                            const range = document.createRange();
                            range.setStart(node, i);
                            range.setEnd(node, i + 1);
                            const bounds = range.getBoundingClientRect();
                            // WebKit quantizes each text run's Range edges.
                            // The center avoids a whole-pixel edge-rounding
                            // difference for otherwise aligned fractional cells.
                            xs.push(bounds.left + bounds.width / 2 - origin);
                        }
                    }
                    return xs;
                };
                const expected = positions(reference);
                const cell =
                    reference.getBoundingClientRect().width /
                    reference.textContent!.length;
                for (const line of lines) {
                    let column = 0;
                    for (const segment of line.querySelectorAll<HTMLElement>(
                        ".code-segment",
                    )) {
                        const x =
                            segment.getBoundingClientRect().left -
                            line.getBoundingClientRect().left;
                        if (Math.abs(x - column * cell) > 0.15)
                            throw new Error(
                                `Segment off grid: ${x} at column ${column}, cell ${cell}`,
                            );
                        column += segment.textContent!.length;
                    }
                }
                const errors = lines.map((line) =>
                    Math.max(
                        ...positions(line).map((x, i) =>
                            Math.abs(x - expected[i]),
                        ),
                    ),
                );

                reference.remove();
                return errors;
            });
        // WebKit rounds Range glyph bounds to pixels within each text run.
        for (const error of await measure()) expect(error).toBeLessThan(1);
        // Cursor paint and blinking cannot change any character's layout.
        await page.addStyleTag({
            content:
                ".code-segment.cursor { animation: none !important; box-shadow: none !important; }",
        });
        for (const error of await measure()) expect(error).toBeLessThan(1);
        // Font completion must reflow already-mounted segments, too.
        await page.addStyleTag({
            content: ".code-viewport { font-size: 17.5px; }",
        });
        await page.evaluate(() =>
            document.fonts.dispatchEvent(new Event("loadingdone")),
        );
        for (const error of await measure()) expect(error).toBeLessThan(1);
        // A proportional font substitution must not break the fixed-cell grid.
        await page.addStyleTag({
            content: ".code-viewport { font-family: sans-serif; }",
        });
        await page.evaluate(() =>
            document.fonts.dispatchEvent(new Event("loadingdone")),
        );
        await expect(page.locator(".code-viewport")).toHaveCSS(
            "font-family",
            "monospace",
        );
        for (const error of await measure()) expect(error).toBeLessThan(1);
        await page.screenshot({
            path: test.info().outputPath("text-placement.png"),
        });
    });
}

// Wide graphemes (CJK, emoji) must each be their own cell-sized segment so a
// fallback font's real advance cannot push later glyphs off the grid.
test("CJK and emoji segments pin every following glyph to the cell grid", async ({
    page,
}) => {
    await page.route("**/src/mock.ts", async (route) => {
        const response = await route.fetch();
        await route.fulfill({
            response,
            body:
                (await response.text()) +
                `\n
const runs = [["ab", 2], ["日", 2], ["本", 2], ["語", 2], ["cd", 2], ["😀", 2], ["e", 1], ["é", 1], ["x", 1]];
mockSnapshot.mode = "NORMAL";
mockSnapshot.lines.splice(0, mockSnapshot.lines.length, ...[-1, 5].map((cursor, row) => ({
    number: row + 1, continuation: false, displayStart: 0, current: cursor >= 0,
    segments: runs.map(([text, cells], index) => ({
        text, cells, token: undefined, cursor: index === cursor, selected: false, searchMatch: false,
    })),
})));`,
        });
    });
    await page.goto("/");
    await expect(page.locator(".code-line")).toHaveCount(2);
    await page.evaluate(() => document.fonts.ready);
    const geometry = await page
        .locator(".line-content")
        .evaluateAll((lines) => {
            const probe = document.createElement("span");
            probe.textContent = "0123456789";
            probe.style.cssText = "position:absolute;white-space:pre;left:0";
            lines[0].append(probe);
            const cell = probe.getBoundingClientRect().width / 10;
            probe.remove();
            return lines.map((line) => {
                const origin = line.getBoundingClientRect().left;
                let column = 0;
                return Array.from(
                    line.querySelectorAll<HTMLElement>(".code-segment"),
                ).map((segment) => {
                    const box = segment.getBoundingClientRect();
                    const range = document.createRange();
                    range.selectNodeContents(segment);
                    const glyph = range.getBoundingClientRect();
                    const cells = Math.round(box.width / cell);
                    const result = {
                        text: segment.textContent,
                        left: box.left - origin,
                        expectedLeft: column * cell,
                        cells,
                        glyphCenter: glyph.left + glyph.width / 2 - origin,
                        cell,
                    };
                    column += cells;
                    return result;
                });
            });
        });
    for (const line of geometry) {
        expect(line.map((segment) => segment.cells)).toEqual([
            2, 2, 2, 2, 2, 2, 1, 1, 1,
        ]);
        for (const segment of line)
            expect(Math.abs(segment.left - segment.expectedLeft)).toBeLessThan(
                0.15,
            );
        // Narrow glyphs after a wide one sit at their own cell's center (the
        // wide glyph itself may come from a fallback font with another
        // advance).
        for (const segment of line.filter((entry) => entry.cells === 1))
            expect(
                Math.abs(
                    segment.glyphCenter - (segment.left + segment.cell / 2),
                ),
            ).toBeLessThan(1);
    }
    // The cursor segment is exactly the wide glyph's own two cells.
    const cursor = page.locator(".code-segment.cursor");
    await expect(cursor).toHaveCount(1);
    await expect(cursor).toHaveText("😀");
});
