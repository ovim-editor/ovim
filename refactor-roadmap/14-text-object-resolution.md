# 14: Unify `TextObjectType` Resolution

**Goal:** Replace three identical 8-arm match blocks with a single `TextObjectType::resolve()` method.

**Fixes:** Triplication that requires touching three files whenever a new text object type is added.

**Risk:** Low. Pure extract-method refactor — no behavior change.

## The Problem

The same match-on-`TextObjectType` → dispatch-to-`TextObjects::*` pattern appears in three places:

| Location | File | Lines | Purpose |
|----------|------|-------|---------|
| `Change::find_text_object()` | `change.rs` | 932–979 | Resolve for dot-repeat |
| `Buffer::delete_text_object()` | `buffer/text_ops.rs` | 549–603 | Resolve for deletion |
| `RepeatAction::ChangeCaseTextObject` | `repeat_action.rs` | 201–247 | Resolve for case transform |

Each handles all 8 variants identically:

```rust
match object_type {
    Word { inner, big } => match (*inner, *big) {
        (true, true) => TextObjects::inner_big_word(buffer),
        (true, false) => TextObjects::inner_word(buffer),
        (false, true) => TextObjects::around_big_word(buffer),
        (false, false) => TextObjects::around_word(buffer),
    },
    Quote { char, inner } => TextObjects::quoted_string(buffer, *char, !*inner),
    Paired { open, close, inner } => TextObjects::paired_delimiters(buffer, *open, *close, !*inner),
    Paragraph { inner } => if *inner { inner_paragraph } else { around_paragraph },
    Sentence { inner } => if *inner { inner_sentence } else { around_sentence },
    Tag { inner } => TextObjects::tag(buffer, !*inner),
    Indent { inner, tab_width } => if *inner { inner_indent } else { around_indent },
    Function { inner } => if *inner { inner_function } else { around_function },
}
```

## The Fix

Add a method to `TextObjectType`:

```rust
impl TextObjectType {
    /// Resolve this text object type to a range at the current cursor position.
    pub fn resolve(&self, buffer: &Buffer) -> Option<TextObjectRange> {
        match self {
            // ... single copy of the dispatch
        }
    }
}
```

Then each call site becomes one line:

```rust
// change.rs (if find_text_object survives roadmap 13 — it won't)
object_type.resolve(buffer)

// text_ops.rs
if let Some(range) = object_type.resolve(self) { ... }

// repeat_action.rs
if let Some(range) = object_type.resolve(buffer) { ... }
```

## Where should `TextObjectType` live?

Currently in `change.rs`, but after roadmap 13 removes the dead variants, its primary consumers are:

- `repeat_action.rs` (RepeatAction::DeleteTextObject, RepeatAction::ChangeCaseTextObject)
- `buffer/text_ops.rs` (Buffer::delete_text_object)
- `textobjects.rs` (TextObjects methods it dispatches to)

The natural home is either `textobjects.rs` (next to the methods it dispatches to) or its own `text_object_type.rs` module. Either is fine — the important thing is that the dispatch lives next to the definition.

## Ordering

Do roadmap 13 first. It removes `Change::find_text_object()`, which is one of the three copies. After that, only two remain, and the extraction is cleaner.

## Files

- `ovim-core/src/change.rs` — move `TextObjectType` out (or keep and add `resolve()`)
- `ovim-core/src/buffer/text_ops.rs` — replace inline dispatch with `resolve()` call
- `ovim-core/src/repeat_action.rs` — replace inline dispatch with `resolve()` call
- `ovim-core/src/textobjects.rs` — possible new home for `TextObjectType`
