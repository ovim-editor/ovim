export const EXPLORER_DEFAULT_WIDTH = 320;
export const EXPLORER_MIN_WIDTH = 240;
export const EXPLORER_MAX_WIDTH = 800;

/** Keep the editor usable when docked; overlays can use the available window. */
export const explorerWidthLimit = (windowWidth: number, contextWidth = 0) =>
    Math.max(
        EXPLORER_MIN_WIDTH,
        Math.min(
            EXPLORER_MAX_WIDTH,
            windowWidth < 1100
                ? windowWidth - 60
                : windowWidth - 540 - contextWidth,
        ),
    );

/** Fit both ends without splitting Unicode code points or relying on character widths. */
export const middleEllipsis = (
    name: string,
    width: number,
    measure: (text: string) => number,
) => {
    if (measure(name) <= width) return name;
    const characters = Array.from(name);
    let low = 0;
    let high = characters.length - 1;
    let result = "…";
    while (low <= high) {
        const count = Math.floor((low + high) / 2);
        const start = Math.ceil(count / 2);
        const end = Math.floor(count / 2);
        const candidate =
            characters.slice(0, start).join("") +
            "…" +
            (end ? characters.slice(-end).join("") : "");
        if (measure(candidate) <= width) {
            result = candidate;
            low = count + 1;
        } else high = count - 1;
    }
    return result;
};
