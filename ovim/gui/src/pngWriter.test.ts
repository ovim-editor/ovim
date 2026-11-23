// @vitest-environment node
import { unzlibSync } from "fflate";
import { describe, expect, it } from "vitest";
import { PngWriter } from "./pngWriter";

describe("incremental PNG encoding", () => {
    it("preserves exact pixels and CRCs across uneven strip boundaries", async () => {
        const width = 3,
            height = 5;
        const pixels = Uint8ClampedArray.from(
            { length: width * height * 4 },
            (_, i) => (i * 71) % 256,
        );
        const writer = new PngWriter(width, height);
        writer.write(pixels.subarray(0, 24));
        writer.write(pixels.subarray(24, 36));
        writer.write(pixels.subarray(36));
        const bytes = new Uint8Array(await writer.finish().arrayBuffer());
        const view = new DataView(bytes.buffer);
        expect([...bytes.slice(0, 8)]).toEqual([
            137, 80, 78, 71, 13, 10, 26, 10,
        ]);
        const compressed: number[] = [];
        let last = "";
        for (let offset = 8; offset < bytes.length;) {
            const length = view.getUint32(offset);
            const type = new TextDecoder().decode(
                bytes.slice(offset + 4, offset + 8),
            );
            // Independent bitwise CRC implementation, including each chunk's type.
            let crc = 0xffffffff;
            for (const byte of bytes.subarray(
                offset + 4,
                offset + 8 + length,
            )) {
                crc ^= byte;
                for (let bit = 0; bit < 8; bit++)
                    crc = crc & 1 ? (crc >>> 1) ^ 0xedb88320 : crc >>> 1;
            }
            expect(view.getUint32(offset + 8 + length)).toBe(
                (crc ^ 0xffffffff) >>> 0,
            );
            if (type === "IDAT")
                compressed.push(
                    ...bytes.subarray(offset + 8, offset + 8 + length),
                );
            last = type;
            offset += length + 12;
        }
        expect(last).toBe("IEND");
        const filtered = unzlibSync(new Uint8Array(compressed));
        const decoded = new Uint8Array(pixels.length);
        for (let row = 0; row < height; row++) {
            expect(filtered[row * 13]).toBe(1);
            for (let i = 0; i < 12; i++)
                decoded[row * 12 + i] =
                    filtered[row * 13 + i + 1] +
                    (i < 4 ? 0 : decoded[row * 12 + i - 4]);
        }
        expect([...decoded]).toEqual([...pixels]);
    });
    it("rejects incomplete, overlong, misaligned and reused images", () => {
        expect(() => new PngWriter(0, 1)).toThrow();
        const writer = new PngWriter(2, 1);
        expect(() => writer.finish()).toThrow("Incomplete");
        expect(() => writer.write(new Uint8ClampedArray(9))).toThrow("strip");
        writer.write(new Uint8ClampedArray(8));
        expect(() => writer.write(new Uint8ClampedArray(8))).toThrow("strip");
        writer.finish();
        expect(() => writer.finish()).toThrow("Incomplete");
    });
});
