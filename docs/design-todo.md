# Design TODO

Design assets are intentionally deferred. The TUI ships with clean, labeled
placeholders so the daemon looks tidy today and a designer can drop final assets
in later without touching engine code. Each item below names the exact swap-in
point.

## Logo

- **Where:** `logo_lines()` in `crates/construct-cli/src/tui/dashboard.rs` (the
  top-right box of the `construct watch` dashboard) and the 🌐 in the title bar.
- **Current state:** the 🌐 (Websites on Computers) mark + "THE CONSTRUCT" wordmark
  + publisher credit + tagline. The brand image lives at `docs/globe.png` (used in
  the README; a terminal can't render a PNG, so the TUI uses the emoji).
- **Needed (taste, not function):** a final ASCII/text wordmark if desired. It must
  fit the top-right box (~3 inner rows, ~38% of the terminal width). Keep it
  monospace-safe and readable down to ~30 columns.
- **Swap-in:** edit `logo_lines()` (returns styled `Line`s). No layout changes are
  needed if the new art fits the box.

## Footer flourish (spinner)

- **Where:** the `SPINNER` braille frames + `draw_footer` in `dashboard.rs`.
- **Current state:** a cheap animated "uplink" spinner advanced once per render
  tick (no threads, no timers). Replaced the earlier digital-rain panel.
- **Swap-in:** edit `SPINNER`/`draw_footer`. **Keep the cost model:** no threads,
  no per-frame `SystemTime`, no busy loop — this runs in an always-on process.

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
