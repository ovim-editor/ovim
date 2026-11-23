import { For, createEffect, createMemo } from "solid-js";
import DOMPurify from "dompurify";
import { Marked, type RendererObject } from "marked";
import type { GuiMarkdownDocument, GuiMarkdownHighlight } from "./types";

const escapeHtml = (text: string) =>
    text.replace(
        /[&<>"']/g,
        (char) =>
            ({
                "&": "&amp;",
                "<": "&lt;",
                ">": "&gt;",
                '"': "&quot;",
                "'": "&#39;",
            })[char]!,
    );

// Raw HTML remains readable source. Images are descriptive links rather than
// network requests during reading. All rendered HTML still passes the sanitizer.
const renderers: RendererObject = {
    html: ({ text }) => `<pre class="document-html">${escapeHtml(text)}</pre>`,
    image: ({ href, text }) =>
        `<a href="${escapeHtml(href)}">Image: ${escapeHtml(text)}</a>`,
};

function highlightedCode(text: string, ranges: GuiMarkdownHighlight[]) {
    if (text.length > 16384) return escapeHtml(text);
    const bytes = new TextEncoder().encode(text);
    const boundaries = [
        ...new Set([
            0,
            bytes.length,
            ...ranges.flatMap((range) => [range.start, range.end]),
        ]),
    ]
        .filter((byte) => byte >= 0 && byte <= bytes.length)
        .sort((a, b) => a - b);
    const decoder = new TextDecoder();
    return boundaries
        .slice(0, -1)
        .map((start, index) => {
            const end = boundaries[index + 1];
            const token = [...ranges]
                .reverse()
                .find(
                    (range) => range.start <= start && range.end >= end,
                )?.token;
            const content = escapeHtml(
                decoder.decode(bytes.subarray(start, end)),
            );
            return token && /^[a-z-]+$/.test(token)
                ? `<span class="document-syntax-${token}">${content}</span>`
                : content;
        })
        .join("");
}

export function documentBlocks(document: GuiMarkdownDocument) {
    let blockOffset = 0;
    const sourceLines = document.text.split("\n");
    const lineStarts = [0];
    for (let i = 0; i < document.text.length; i += 1) {
        if (document.text[i] === "\n") lineStarts.push(i + 1);
    }
    const markdown = new Marked({
        gfm: true,
        breaks: false,
        renderer: {
            ...renderers,
            code: (token) => {
                const found = document.text.indexOf(token.raw, blockOffset);
                if (found < 0)
                    return `<pre><code>${escapeHtml(token.text)}</code></pre>`;
                let low = 0,
                    high = lineStarts.length;
                while (low < high) {
                    const mid = (low + high) >>> 1;
                    if (lineStarts[mid] <= found) low = mid + 1;
                    else high = mid;
                }
                const first =
                    low - 1 + (/^ {0,3}(`{3,}|~{3,})/.test(token.raw) ? 1 : 0);
                const lines = token.text.split("\n").map((text, index) => {
                    const original = sourceLines[first + index] ?? "";
                    const prefix = original.endsWith(text)
                        ? new TextEncoder().encode(
                              original.slice(0, original.length - text.length),
                          ).length
                        : 0;
                    const ranges = (
                        document.highlights?.[first + index] ?? []
                    ).map((range) => ({
                        ...range,
                        start: range.start - prefix,
                        end: range.end - prefix,
                    }));
                    const viewLine = document.viewLines[first + index];
                    const content = highlightedCode(text, ranges);
                    return Number.isSafeInteger(viewLine)
                        ? `<span data-view-line="${viewLine}">${content}</span>`
                        : content;
                });
                return `<pre><code>${lines.join("\n")}\n</code></pre>`;
            },
        },
    });
    let line = 0;
    let offset = 0;
    return markdown.lexer(document.text).flatMap((token) => {
        // Link definitions are consumed by the lexer without producing tokens.
        // Locate raw blocks monotonically so those hidden lines still count.
        const found = document.text.indexOf(token.raw, offset);
        if (found >= offset) {
            line += (document.text.slice(offset, found).match(/\n/g) ?? [])
                .length;
            offset = found;
        }
        blockOffset = offset;
        const start = line;
        offset += token.raw.length;
        line += (token.raw.match(/\n/g) ?? []).length;
        if (token.type === "space" || token.type === "def") return [];
        const contentLine =
            token.type === "code" && /^ {0,3}(`{3,}|~{3,})/.test(token.raw)
                ? start + 1
                : start;
        const viewLine =
            document.viewLines[contentLine] ?? document.viewLines[start] ?? 0;
        return [
            {
                viewLine,
                html: DOMPurify.sanitize(markdown.parser([token]), {
                    USE_PROFILES: { html: true },
                    FORBID_TAGS: ["style", "form", "button", "iframe", "img"],
                    FORBID_ATTR: ["style", "id", "name"],
                }),
            },
        ];
    });
}

export default function MarkdownDocument(props: {
    document: GuiMarkdownDocument;
    cursorLine: number;
    focused: boolean;
    firstLine?: number;
    syntax?: Record<string, string>;
    onSelect: (line: number) => void;
    onOpenLink: (url: string) => void;
}) {
    let article!: HTMLElement;
    const blocks = createMemo(() => documentBlocks(props.document));
    const active = createMemo(() => {
        const items = blocks();
        const line = props.cursorLine;
        for (let index = items.length - 1; index >= 0; index -= 1) {
            if (items[index].viewLine <= line) return index;
        }
        return -1;
    });
    let previousLine = props.cursorLine;
    let previousFirst = props.firstLine ?? 0;
    createEffect(() => {
        const cursor = props.cursorLine;
        const first = props.firstLine ?? 0;
        if (
            props.focused &&
            cursor === previousLine &&
            first !== previousFirst
        ) {
            article?.scrollBy?.({ top: (first - previousFirst) * 26 });
        }
        previousLine = cursor;
        previousFirst = first;
    });
    createEffect(() => {
        const index = active();
        const line = props.cursorLine;
        if (!props.focused) return;
        // Wait for Solid to mount the block list before following a core motion.
        queueMicrotask(() => {
            const block =
                article?.querySelector<HTMLElement>(
                    `[data-view-line="${line}"]`,
                ) ??
                article?.querySelector<HTMLElement>(`[data-block="${index}"]`);
            if (!block) return;
            const bounds = block.getBoundingClientRect();
            const viewport = article.getBoundingClientRect();
            if (bounds.top < viewport.top || bounds.bottom > viewport.bottom) {
                block.scrollIntoView?.({
                    block:
                        bounds.height > viewport.height ? "start" : "nearest",
                });
            }
        });
    });
    return (
        <article
            ref={article}
            class="markdown-document"
            style={Object.fromEntries(
                Object.entries(props.syntax ?? {}).map(([token, color]) => [
                    `--document-syntax-${token}`,
                    color,
                ]),
            )}
            aria-label="Formatted Markdown reading view"
        >
            <div class="document-hint">
                PSEUDO{" "}
                <span>Click a block · Enter to open source · q to return</span>
            </div>
            <div class="document-content">
                <For each={blocks()}>
                    {(block, index) => (
                        <div
                            class="document-block"
                            classList={{
                                active: props.focused && active() === index(),
                            }}
                            data-block={index()}
                            onMouseDown={(event) => {
                                if (
                                    event.button !== 0 ||
                                    (event.target as Element).closest("a")
                                )
                                    return;
                                event.preventDefault();
                                const codeLine = (
                                    event.target as Element
                                ).closest<HTMLElement>("[data-view-line]")
                                    ?.dataset.viewLine;
                                props.onSelect(
                                    codeLine === undefined
                                        ? block.viewLine
                                        : Number(codeLine),
                                );
                            }}
                            onClick={(event) => {
                                const link = (event.target as Element).closest(
                                    "a[href]",
                                );
                                if (!link) return;
                                event.preventDefault();
                                const href = link.getAttribute("href") ?? "";
                                if (href.startsWith("#")) {
                                    let target: string;
                                    try {
                                        target = decodeURIComponent(
                                            href.slice(1),
                                        );
                                    } catch {
                                        return;
                                    }
                                    const used = new Map<string, number>();
                                    for (const heading of article.querySelectorAll<HTMLElement>(
                                        "h1, h2, h3, h4, h5, h6",
                                    )) {
                                        const base = (heading.textContent ?? "")
                                            .toLowerCase()
                                            .replace(/[^\p{L}\p{N}\s_-]/gu, "")
                                            .replace(/\s/g, "-");
                                        const count = used.get(base) ?? 0;
                                        used.set(base, count + 1);
                                        if (
                                            (count
                                                ? `${base}-${count}`
                                                : base) !== target
                                        )
                                            continue;
                                        const index = Number(
                                            heading.closest<HTMLElement>(
                                                "[data-block]",
                                            )?.dataset.block,
                                        );
                                        const block = blocks()[index];
                                        if (block)
                                            props.onSelect(block.viewLine);
                                        heading.scrollIntoView?.({
                                            block: "start",
                                        });
                                        return;
                                    }
                                    return;
                                }
                                props.onOpenLink(href);
                            }}
                            innerHTML={block.html}
                        />
                    )}
                </For>
            </div>
        </article>
    );
}
