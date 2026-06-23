# Design TODO

Design assets are intentionally deferred. The TUI ships with clean, labeled
placeholders so the daemon looks tidy today and a designer can drop final assets
in later without touching engine code. Each item below names the exact swap-in
point.

## ASCII logo

- **Where:** `const LOGO` in `crates/construct-cli/src/tui/dashboard.rs`.
- **Current state:** a tiny three-line placeholder rendered in the bottom-left box
  of the `construct watch` dashboard.

  ```
    ╔═╗
    ║C║  THE
    ╚═╝  CONSTRUCT
  ```

- **Needed:** a final ASCII/text wordmark. Constraints: it must fit the
  bottom-left box, which is ~8 rows tall and roughly 22% of the terminal width
  (see the bottom-row layout in `draw_bottom`). Keep it monospace-safe and
  readable down to ~30 columns.
- **Swap-in:** replace the `LOGO` string literal. It's rendered via a single
  `Paragraph::new(LOGO)` styled with `Theme::accent()`, so no layout changes are
  needed if the new art fits the box.

## Digital rain styling

- **Where:** `crates/construct-cli/src/tui/rain.rs` (animation), rendered in the
  bottom-row "rain" box of the dashboard.
- **Current state:** a working, CPU-cheap "Matrix" digital-rain panel. It's driven
  by the dashboard's existing ~5fps render tick (200ms poll), does only
  `O(columns)` integer work per frame, and uses a seeded xorshift PRNG (no `rand`
  dependency), so it's deterministic and adds no background CPU load.
- **Needed (taste, not function):** final glyph set and color ramp. The glyphs are
  `const GLYPHS` in `rain.rs`; the head/trail colors are inline in the dashboard's
  rain renderer (bright head → green → dim tail).
- **Swap-in:** edit `GLYPHS` for the character set and the rain color match arms in
  `dashboard.rs` for the palette. **Keep the cost model:** no threads, no per-cell
  `SystemTime`, no busy loop — this runs in an always-on process.

## Brand colors / theme

- **Where:** `crates/construct-cli/src/theme.rs`.
- **Current state:** an "earthy blues & browns" placeholder palette
  (`DEEP_BLUE`, `DUSK_BLUE`, `CLAY`, `SAND`, `PARCHMENT`) exposed through
  `Theme::header()`, `Theme::accent()`, and `Theme::body()`.
- **Needed:** the final brand palette and any additional named styles the
  dashboard should use consistently.
- **Swap-in:** update the `Color::Rgb(...)` constants and the style helpers in
  `theme.rs`. All dashboard widgets pull styling through `Theme::*`, so changing
  the palette here re-themes the whole TUI in one place.

## Notes for whoever picks this up

- The daemon runs perfectly headless (`construct watch --headless`); the TUI is a
  read-only observability surface, so design work here is purely cosmetic and
  can't break the core loop.
- Nothing above blocks a release. These are polish items, not functionality gaps.
