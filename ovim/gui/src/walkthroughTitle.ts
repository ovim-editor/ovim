/** Keep the leaf and its nearest context; collapse distant directories first. */
export function walkthroughPath(path: string, budget: number): string {
    const chars = Array.from(path);
    budget = Math.max(8, Math.floor(budget));
    if (chars.length <= budget) return path;
    const parts = path.split(/[\\/]/).filter(Boolean);
    const leaf = parts.pop() || path;
    const parent = parts.pop();
    const tail = parent ? `${parent}/${leaf}` : leaf;
    const candidates = [
        ...(parts.length ? [`${parts[0]}/…/${tail}`] : []),
        ...(parent ? [`…/${tail}`] : []),
        `…/${leaf}`,
    ];
    for (const candidate of candidates) {
        if (Array.from(candidate).length <= budget) return candidate;
    }
    // A single long filename needs middle truncation too: retain its extension
    // and distinguishing suffix as well as its beginning. Full text is a tooltip.
    const name = Array.from(leaf);
    const available = budget - 3;
    const suffix = Math.min(Math.ceil(available / 2), name.length);
    return `…/${name.slice(0, available - suffix).join("")}…${name.slice(-suffix).join("")}`;
}
