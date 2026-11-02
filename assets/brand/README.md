# Mothball brand files

| File | Use |
|---|---|
| `mothball-icon.svg` | App icon, 64 px and up (dark tile) |
| `mothball-icon-small.svg` | App icon at 16–48 px: no antennae, thicker body |
| `mothball-mark.svg` | The moth alone, for dark backgrounds |
| `mothball-mark-dark.svg` | The moth alone, for light backgrounds |
| `mothball-wordmark.svg` / `.png` | Moth + name, for dark backgrounds (GitHub dark, website) |
| `mothball-wordmark-dark.svg` / `.png` | Moth + name, for light backgrounds |

Colours: white `#ECE9F4`, purple `#9466FF`, tile `#14101D`.

The wordmark SVGs use live text (Bahnschrift, falling back to Segoe UI / Inter);
use the PNGs where the font may not be installed.

The Windows icons in `src-tauri/icons/` are rendered from the two icon SVGs:
16–48 px from the small variant, 64 px and up from the full one.
