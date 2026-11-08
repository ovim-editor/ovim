import type { GuiKeyInput } from "./types";

const CODE_PUNCTUATION: Record<string, [string, string]> = {
    Minus: ["-", "_"],
    Equal: ["=", "+"],
    BracketLeft: ["[", "{"],
    BracketRight: ["]", "}"],
    Backslash: ["\\", "|"],
    Semicolon: [";", ":"],
    Quote: ["'", '"'],
    Backquote: ["`", "~"],
    Comma: [",", "<"],
    Period: [".", ">"],
    Slash: ["/", "?"],
};

const SHIFTED_DIGITS = ")!@#$%^&*(";

const latinKeyFromCode = (code: string, shift: boolean): string | undefined => {
    const letter = /^Key([A-Z])$/.exec(code);
    if (letter) return shift ? letter[1] : letter[1].toLowerCase();
    const digit = /^Digit([0-9])$/.exec(code);
    if (digit) return shift ? SHIFTED_DIGITS[Number(digit[1])] : digit[1];
    const punctuation = CODE_PUNCTUATION[code];
    if (punctuation) return punctuation[shift ? 1 : 0];
    return undefined;
};

type KeyEventLike = Pick<
    KeyboardEvent,
    "key" | "shiftKey" | "ctrlKey" | "altKey" | "metaKey"
> &
    Partial<Pick<KeyboardEvent, "code">>;

/**
 * The key a chord should be dispatched as. On non-Latin layouts (Russian,
 * Greek, Hebrew...) Ctrl+W arrives as event.key "ц", so when Ctrl or Meta is
 * held and the key is a single non-ASCII letter we fall back to the physical
 * key (event.code) as a US layout would report it. Plain typing and
 * Alt/AltGr-produced characters are left alone: AltGr reports ctrl+alt on some
 * setups, so Alt disables the remap.
 */
export const chordKey = (event: KeyEventLike): string => {
    if (
        !(event.ctrlKey || event.metaKey) ||
        event.altKey ||
        !event.code ||
        Array.from(event.key).length !== 1 ||
        !/^\p{L}$/u.test(event.key) ||
        /^[\x00-\x7f]$/.test(event.key)
    )
        return event.key;
    return latinKeyFromCode(event.code, event.shiftKey) ?? event.key;
};

export const guiKeyInput = (event: KeyEventLike): GuiKeyInput | undefined => {
    if (["Shift", "Control", "Alt", "Meta"].includes(event.key))
        return undefined;
    const optionProducedText =
        event.altKey &&
        Array.from(event.key).length === 1 &&
        !/^[\x00-\x7f]$/.test(event.key);
    return {
        key: chordKey(event),
        shift: event.shiftKey,
        control: event.ctrlKey,
        alt: event.altKey && !optionProducedText,
        meta: event.metaKey,
    };
};
