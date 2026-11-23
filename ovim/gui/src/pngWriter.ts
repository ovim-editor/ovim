import { Zlib } from "fflate";

const crcTable = Uint32Array.from({ length: 256 }, (_, n) => {
    for (let bit = 0; bit < 8; bit++)
        n = n & 1 ? 0xedb88320 ^ (n >>> 1) : n >>> 1;
    return n >>> 0;
});
function chunk(type: string, data = new Uint8Array()): Uint8Array<ArrayBuffer> {
    const result = new Uint8Array(data.length + 12);
    const view = new DataView(result.buffer);
    view.setUint32(0, data.length);
    result.set(new TextEncoder().encode(type), 4);
    result.set(data, 8);
    let crc = 0xffffffff;
    for (const byte of result.subarray(4, -4))
        crc = crcTable[(crc ^ byte) & 255] ^ (crc >>> 8);
    view.setUint32(result.length - 4, (crc ^ 0xffffffff) >>> 0);
    return result;
}

/** One RGBA PNG, compressed incrementally without retaining raw image pixels. */
export class PngWriter {
    private parts: BlobPart[] = [
        new Uint8Array([137, 80, 78, 71, 13, 10, 26, 10]),
    ];
    private rows = 0;
    private finished = false;
    private compressor: Zlib;
    constructor(
        private width: number,
        private height: number,
    ) {
        if (
            ![width, height].every(
                (n) => Number.isInteger(n) && n > 0 && n <= 0x7fffffff,
            )
        )
            throw new Error("Invalid PNG dimensions.");
        const header = new Uint8Array(13);
        const view = new DataView(header.buffer);
        view.setUint32(0, width);
        view.setUint32(4, height);
        header[8] = 8;
        header[9] = 6; // 8-bit RGBA, no interlacing.
        this.parts.push(chunk("IHDR", header));
        this.compressor = new Zlib({ level: 6 }, (data) => {
            if (data.length) this.parts.push(chunk("IDAT", data));
        });
    }
    write(rgba: Uint8ClampedArray): void {
        const stride = this.width * 4;
        const rows = rgba.length / stride;
        if (
            this.finished ||
            !Number.isInteger(rows) ||
            this.rows + rows > this.height
        )
            throw new Error("Invalid PNG strip.");
        const filtered = new Uint8Array(rows * (stride + 1));
        for (let row = 0; row < rows; row++) {
            const start = row * stride,
                target = row * (stride + 1);
            filtered[target] = 1; // Sub filter preserves compression across flat backgrounds.
            for (let i = 0; i < stride; i++)
                filtered[target + 1 + i] =
                    rgba[start + i] - (i < 4 ? 0 : rgba[start + i - 4]);
        }
        this.rows += rows;
        this.compressor.push(filtered, false);
    }
    finish(): Blob {
        if (this.finished || this.rows !== this.height)
            throw new Error("Incomplete PNG image.");
        this.finished = true;
        this.compressor.push(new Uint8Array(), true);
        this.parts.push(chunk("IEND"));
        return new Blob(this.parts, { type: "image/png" });
    }
}
