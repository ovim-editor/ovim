/** @vitest-environment jsdom */
import { fireEvent, render, screen } from "@solidjs/testing-library";
import { describe, expect, it, vi } from "vitest";
import MarkdownDocument, { documentBlocks } from "./MarkdownDocument";

const doc = (text: string) => ({
    text,
    viewLines: text.split("\n").map((_, line) => line * 2),
});

describe("Markdown document reading view", () => {
    it("renders document structure and selects its mapped source block", () => {
        const onSelect = vi.fn();
        const onOpenLink = vi.fn();
        const document = doc(
            "# Guide\n\nRead **carefully**, with *emphasis*.\n\n- [x] Done\n- Next\n\n| Name | Value |\n| --- | --- |\n| A | 1 |\n\n```java\nsize(name):\n    return name.length()\n```\n\n[Visit](https://example.com)\n",
        );
        const { container } = render(() => (
            <MarkdownDocument
                document={document}
                cursorLine={0}
                focused
                onSelect={onSelect}
                onOpenLink={onOpenLink}
            />
        ));
        expect(
            screen.getByRole("heading", { name: "Guide", level: 1 }),
        ).toBeTruthy();
        expect(container.querySelector("strong")?.textContent).toBe(
            "carefully",
        );
        expect(container.querySelector("em")?.textContent).toBe("emphasis");
        expect(screen.getByRole("table")).toBeTruthy();
        expect((screen.getByRole("checkbox") as HTMLInputElement).checked).toBe(
            true,
        );
        expect(container.querySelector("pre code")?.textContent).toContain(
            "return name.length()",
        );
        fireEvent.mouseDown(container.querySelector("strong")!, { button: 0 });
        expect(onSelect).toHaveBeenCalledWith(4);
        const codeLine =
            document.text.split("\n").indexOf("    return name.length()") * 2;
        fireEvent.mouseDown(
            container.querySelector(`[data-view-line="${codeLine}"]`)!,
            { button: 0 },
        );
        expect(onSelect).toHaveBeenLastCalledWith(codeLine);
        fireEvent.click(screen.getByRole("link", { name: "Visit" }));
        expect(onOpenLink).toHaveBeenCalledWith("https://example.com");
    });

    it("keeps source coordinates after reference definitions and repeated blocks", () => {
        const blocks = documentBlocks(
            doc("[ref]: https://example.com\n\nSame\n\nSame\n\n[Link][ref]\n"),
        );
        expect(blocks.map((block) => block.viewLine)).toEqual([4, 8, 12]);
        expect(blocks[2].html).toContain('href="https://example.com"');
    });

    it("preserves hard breaks and literal code whitespace", () => {
        const blocks = documentBlocks(
            doc("First  \nsecond\n\n```text\na  \n\n\nb\n```\n"),
        );
        expect(blocks[0].html).toContain("<br>");
        const container = document.createElement("div");
        container.innerHTML = blocks[1].html;
        expect(container.querySelector("code")?.textContent).toBe(
            "a  \n\n\nb\n",
        );
    });

    it("renders raw HTML literally and does not fetch images or keep unsafe links", () => {
        const blocks = documentBlocks(
            doc(
                '<script>alert(1)</script>\n\n<img src="https://example.com/a.png" onerror="alert(1)">\n\n![Photo](https://example.com/photo.png)\n\n[Bad](javascript:alert%281%29)\n',
            ),
        );
        const html = blocks.map((block) => block.html).join("");
        expect(html).not.toMatch(/<script|<img|href="javascript:/);
        expect(html).toContain("&lt;script&gt;");
        expect(html).toContain("Image: Photo");
    });
    it("colors Unicode code from source byte ranges and selects code rather than its hidden fence", () => {
        const document = {
            ...doc("```java\n名 = 1\n```\n"),
            highlights: [
                [],
                [
                    { start: 0, end: 3, token: "variable" },
                    { start: 6, end: 7, token: "number" },
                ],
                [],
            ],
        };
        const blocks = documentBlocks(document);
        expect(blocks[0].viewLine).toBe(2);
        expect(blocks[0].html).toContain(
            '<span class="document-syntax-variable">名</span>',
        );
        expect(blocks[0].html).toContain(
            '<span class="document-syntax-number">1</span>',
        );
        expect(blocks[0].html).not.toContain("�");
    });

    it("follows heading links within the document", () => {
        const onSelect = vi.fn();
        const onOpenLink = vi.fn();
        render(() => (
            <MarkdownDocument
                document={doc("[Jump](#details)\n\n## Details\n")}
                cursorLine={0}
                focused
                onSelect={onSelect}
                onOpenLink={onOpenLink}
            />
        ));
        fireEvent.click(screen.getByRole("link", { name: "Jump" }));
        expect(onSelect).toHaveBeenCalledWith(4);
        expect(onOpenLink).not.toHaveBeenCalled();
    });
});
