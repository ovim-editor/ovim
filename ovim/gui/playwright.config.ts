import { defineConfig } from "@playwright/test";

export default defineConfig({
    testDir: "./e2e",
    use: {
        baseURL: "http://127.0.0.1:5180",
        viewport: { width: 1440, height: 900 },
    },
    projects: [
        { name: "chromium", use: { browserName: "chromium" } },
        { name: "webkit", use: { browserName: "webkit" } },
    ],
    webServer: {
        command: "npm run dev -- --port 5180 --strictPort",
        url: "http://127.0.0.1:5180",
        reuseExistingServer: !process.env.CI,
    },
});
