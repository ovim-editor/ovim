# Cat Sprite Sketches

Exploring beyond pure ASCII for the walking cat animation.

## Available Terminal Primitives

- **Block elements** (2x2 per cell): `▀ ▄ █ ░ ▒ ▓ ▖ ▗ ▘ ▝ ▚ ▞ ▐ ▌`
- **Braille** (2x4 per cell): `⠁⠂⠃...⣿` — highest resolution
- **Box drawing**: `╭ ╮ ╰ ╯ │ ─ ╲ ╱`
- **Misc**: `◠ ◡ ● ◦ ⟡ ∧ ⁀ ‿ ◜ ◝ ◟ ◞ · ˙ ° ˆ`

---

## Approach 1: Block Element Pixel Art (side profile)

Uses half-blocks (▀▄█) for a chunky pixel-art cat.
Each character = 2 vertical pixels, so a 4-row sprite = 8px tall.

```
Walk 1:                Walk 2:
    ▄▀▀▄                  ▄▀▀▄
▀▄▄█▀  █▄              ▀▄▄█▀  █▄
   █ ▀▀                    █ ▀▀
   ▌▐                      ▐▌
```

Pros: Chunky retro feel, good contrast
Cons: Harder to read at small sizes

---

## Approach 2: Braille High-Res Sprites (side profile)

Each braille char is a 2x4 dot grid. A 6-wide × 3-tall sprite area
gives us 12x12 effective pixels — enough for a recognizable cat.

Example silhouette (conceptual, dots approximate):

```
Walk 1:          Walk 2:
 ⢀⡴⠶⣄           ⢀⡴⠶⣄
⠈⣧⣤⡾⠃⠤         ⠈⣧⣤⡾⠃⠤
  ⠇⠸             ⠸ ⠇
```

Pros: Smoothest look, highest detail
Cons: May not render well on all terminals/fonts

---

## Approach 3: Mixed Unicode (side profile, readable)

Combine regular chars, box drawing, and select unicode for a
clean side-profile cat that's readable and charming.

```
Walk 1:             Walk 2:
  ∧ ∧                ∧ ∧
~(° ◡ °)            ~(° ◡ °)
  | |╱               ╲| |
  |╱                  ╲|
```

Or with more personality:

```
Walk 1:             Walk 2:
  ╱╲ ╱╲              ╱╲ ╱╲
~( ◦.◦ )            ~( ◦.◦ )
  ╱╱                  ╲╲
 ╱╱                    ╲╲
```

Pros: Readable, distinctive, works on most terminals
Cons: Still somewhat flat

---

## Approach 4: Block + Unicode Hybrid (chunky side cat)

```
Walk 1:              Walk 2:
   ▄█▄                 ▄█▄
 ╱(●.●)             ╱(●.●)
▔  ╱╱   ▏          ▔  ╲╲   ▏
  ╱ ╱                 ╲ ╲
```

Sitting:
```
   ▄█▄
  (●.●)
  ╱██╲
  ╱  ╲
```

---

## Approach 5: Stylized Minimal (Unicode accents on ASCII base)

Keep the ASCII skeleton but replace key elements with unicode
for smoother look:

```
Walk 1:           Walk 2:          Sit:
  ╱╲_╱╲            ╱╲_╱╲           ╱╲_╱╲
 ( ◦.◦ )          ( ◦.◦ )         ( ◦.◦ )
  ╲ ▽ ╱            ╲ ▽ ╱           ╱▏▕╲
  ╱▏ ▕             ▏▕ ╲╱           ▕  ▏
 ╱▕                ╲▏
```

Pros: Closest to current aesthetic, easy transition
Cons: Least dramatic improvement

---

## Approach 6: Braille Body + ASCII Face

Best of both worlds: use braille for the body silhouette
(smooth curves) but keep ASCII/unicode for the face (expressive).

```
Walk 1:            Walk 2:           Sit:
  ╱╲_╱╲             ╱╲_╱╲            ╱╲_╱╲
 ( ◦.◦ )           ( ◦.◦ )          ( –.– )
 ⣿⡗ ⢼⣿            ⣿⡗ ⢼⣿           ⢸⣿⣿⡇
 ⠇  ⠸              ⠸  ⠇            ⠈⠉⠉⠁
```

The braille body gives smooth contour for the torso/legs,
while the face stays readable with standard chars.

Pros: Smooth body, expressive face, best visual quality
Cons: Font rendering dependency for braille

---

## Recommendation

**Approach 3 (Mixed Unicode)** or **Approach 5 (Stylized Minimal)** are
the safest — they work everywhere and still look great.

**Approach 6 (Braille Body + ASCII Face)** is the most visually
impressive but depends on good braille font support.

Consider: we could detect terminal capability and fall back gracefully.

---

## Walking Animation Specifics

Regardless of approach, the side-profile walking needs:

1. **Tail sway** — alternates position each frame (adds life)
2. **Leg pairs** — front/back legs alternate (trot gait):
   - Frame 1: front-left + back-right forward
   - Frame 2: front-right + back-left forward
3. **Slight body bob** — optional, subtle vertical shift
4. **Head stays level** — anchor point for recognition

A 2-frame walk cycle is minimum. 4 frames would be smoother:
- Frame 1: Right legs forward (contact)
- Frame 2: Right legs passing (mid-stride)
- Frame 3: Left legs forward (contact)
- Frame 4: Left legs passing (mid-stride)
