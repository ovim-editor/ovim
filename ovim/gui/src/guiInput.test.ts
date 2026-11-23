import { describe, expect, it } from "vitest";
import { chordKey, guiKeyInput } from "./guiInput";

const event = (
    key: string,
    code: string,
    mods: Partial<{
        shiftKey: boolean;
        ctrlKey: boolean;
        altKey: boolean;
        metaKey: boolean;
    }> = {},
) => ({
    key,
    code,
    shiftKey: false,
    ctrlKey: false,
    altKey: false,
    metaKey: false,
    ...mods,
});

describe("chordKey on non-Latin layouts", () => {
    it("maps Ctrl+letter from the physical key", () => {
        expect(chordKey(event("ц", "KeyW", { ctrlKey: true }))).toBe("w");
        expect(chordKey(event("ы", "KeyS", { ctrlKey: true }))).toBe("s");
        expect(chordKey(event("я", "KeyZ", { metaKey: true }))).toBe("z");
        expect(chordKey(event("ς", "KeyW", { ctrlKey: true }))).toBe("w");
        expect(chordKey(event("ק", "KeyE", { ctrlKey: true }))).toBe("e");
    });

    it("keeps shift as uppercase for letters", () => {
        expect(
            chordKey(event("Я", "KeyZ", { ctrlKey: true, shiftKey: true })),
        ).toBe("Z");
    });

    it("maps punctuation keys Vim chords rely on", () => {
        expect(chordKey(event("х", "BracketLeft", { ctrlKey: true }))).toBe(
            "[",
        );
        expect(chordKey(event("ъ", "BracketRight", { ctrlKey: true }))).toBe(
            "]",
        );
        expect(chordKey(event("ж", "Semicolon", { ctrlKey: true }))).toBe(";");
        expect(chordKey(event("б", "Comma", { ctrlKey: true }))).toBe(",");
    });

    it("maps digit-row keys that report a letter", () => {
        expect(chordKey(event("é", "Digit2", { ctrlKey: true }))).toBe("2");
        expect(chordKey(event("é", "Digit1", { metaKey: true }))).toBe("1");
    });

    it("leaves plain typing untouched", () => {
        expect(chordKey(event("ц", "KeyW"))).toBe("ц");
        expect(chordKey(event("Ц", "KeyW", { shiftKey: true }))).toBe("Ц");
    });

    it("does not remap when Alt is held (AltGr reports ctrl+alt)", () => {
        expect(
            chordKey(event("ł", "KeyL", { ctrlKey: true, altKey: true })),
        ).toBe("ł");
    });

    it("leaves ASCII keys and named keys alone (Dvorak, AZERTY, Escape)", () => {
        expect(chordKey(event("w", "Comma", { ctrlKey: true }))).toBe("w");
        expect(chordKey(event("Escape", "Escape", { ctrlKey: true }))).toBe(
            "Escape",
        );
        expect(chordKey(event("Tab", "Tab", { ctrlKey: true }))).toBe("Tab");
    });

    it("falls back to the reported key when the code is unknown", () => {
        expect(chordKey(event("ц", "Unidentified", { ctrlKey: true }))).toBe(
            "ц",
        );
        expect(
            chordKey({
                key: "ц",
                shiftKey: false,
                ctrlKey: true,
                altKey: false,
                metaKey: false,
            }),
        ).toBe("ц");
    });
});

describe("guiKeyInput with non-Latin layouts", () => {
    it("sends Ctrl+W as w so Vim chords work", () => {
        expect(guiKeyInput(event("ц", "KeyW", { ctrlKey: true }))).toEqual({
            key: "w",
            shift: false,
            control: true,
            alt: false,
            meta: false,
        });
    });

    it("sends plain Cyrillic text unchanged", () => {
        expect(guiKeyInput(event("ц", "KeyW"))?.key).toBe("ц");
    });
});
