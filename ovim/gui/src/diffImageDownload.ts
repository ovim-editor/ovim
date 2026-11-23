export type DiffImageFile = { filename: string; blob: Blob };

export function downloadDiffImages(file: DiffImageFile): void {
    const url = URL.createObjectURL(file.blob);
    const anchor = document.createElement("a");
    anchor.href = url;
    anchor.download = file.filename;
    document.body.append(anchor);
    anchor.click();
    anchor.remove();
    // WebKit may not consume the download URL until after the click dispatch.
    setTimeout(() => URL.revokeObjectURL(url), 30_000);
}
