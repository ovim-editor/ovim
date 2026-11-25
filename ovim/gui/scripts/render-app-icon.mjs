// Renders icons/icon.svg to the 1024px PNG that `tauri icon` turns into
// every platform size. Chromium draws the SVG filters (glow, shadow) that
// simpler rasterisers skip.
import { readFile } from "node:fs/promises";
import { fileURLToPath } from "node:url";
import { chromium } from "@playwright/test";

const size = 1024;
const source = fileURLToPath(new URL("../../icons/icon.svg", import.meta.url));
const output = fileURLToPath(new URL("../../icons/icon.png", import.meta.url));

const svg = await readFile(source, "utf8");
const browser = await chromium.launch();
const page = await browser.newPage({
    viewport: { width: size, height: size },
});
await page.setContent(
    `<body style="margin:0;background:transparent">${svg}</body>`,
);
await page.screenshot({
    path: output,
    omitBackground: true,
    clip: { x: 0, y: 0, width: size, height: size },
});
await browser.close();
console.log(`Wrote ${output}`);
