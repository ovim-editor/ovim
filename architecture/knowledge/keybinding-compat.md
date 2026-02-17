# Keybinding Compatibility Contract

This document defines ovim keybinding compatibility against Neovim behavior.

## Goal

- Match core Neovim editing/motion behavior by default.
- Keep intentional product-specific overrides explicit and documented.

## Intentional Divergences

These are deliberate and not considered bugs:

1. `<C-i>`:
   - Reserved for ovim-specific behavior (not jump-forward).
2. `-` in Normal mode:
   - Toggles file tree (oil-style), instead of Neovim `-` motion.
3. AI region mode Ctrl bindings:
   - `<C-e>`, `<C-y>`, `<C-n>`, `<C-Space>`, `<C-c>` are contextually repurposed when an AI region is selected.

## Compatibility Scope

ovim should track Neovim semantics for:

- Core motions and operators
- Text objects
- Mark/jump behavior
- Keymap APIs and Ex map commands

unless explicitly listed in "Intentional Divergences".

## Review Rule

When a keybinding mismatch is reported:

1. Check this contract.
2. If not listed as intentional, treat as compatibility bug.
3. Add/adjust tests before closing.
