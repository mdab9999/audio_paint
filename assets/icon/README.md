# Audio Paint icon

| File | Use |
|---|---|
| `audio_paint.svg` | Master artwork (48 px and larger) |
| `audio_paint_small.svg` | Simplified artwork for 16-32 px: four bold bars, no brush |
| `audio_paint.ico` | Windows icon with 16, 24, 32, 48, 64, 128 and 256 px images |
| `png/audio_paint_<size>.png` | PNG renders at 16, 24, 32, 48, 64, 128, 256, 512 and 1024 px |

Sizes up to 32 px are rendered from the small artwork, larger sizes from the master.
The PNGs were rendered from the SVGs with [resvg](https://github.com/linebender/resvg)
(`resvg -w <size> -h <size> <svg> <png>`), and the `.ico` was assembled from those PNGs.
