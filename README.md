# ascii-aquarium

[ascii-sharks](https://github.com/stackedlol/ascii-sharks) (stackedlol) and
[ascii-koi](https://github.com/Ahmet-Dedeler/ascii-koi) (Ahmet Dedeler, MIT),
ported from a browser canvas to Rust and drawn in a fullscreen GPUI window, as
a scene in the Omarchy shader screensaver's rotation.

ascii-sharks carries no licence, so this port stays private.

## Run

```sh
./install.sh                        # builds ~/.local/bin/ascii-aquarium
ascii-aquarium sharks               # any key, click or mouse move exits
ascii-aquarium koi --theme light    # upstream's paper-white pond
ascii-aquarium sharks --cast thresher,hammerhead,mako,nurse
```

`--cast` takes upstream's `?sharks=` / `?koi=` lists (first is biggest);
`--max-fps` caps the frame rate (the simulation always steps at 60 Hz).

As the screensaver, the shader screensaver's launcher runs it with
`--app-id org.omarchy.screensaver --log <run log> <collection>/sharks.ascii`
when `sharks.ascii` or `koi.ascii` comes up in `screensaver-shaders pick`.
That mode hides the pointer, stops when focus leaves or the session locks,
stops the other monitors' instances, and logs `showing` / `stopped` lines the
rotation reads.

## How it draws

The originals draw a grid of ASCII cells on a canvas; so does this. Each frame
the simulation (fish boids, predator steering, 60 Hz fixed step) is
rasterised onto cells: five samples per cell against each body and fin shape,
fully covered cells as a shaded block with a dense glyph, partly covered ones
as a light edge glyph. Fish are `-`, wake ripples `·`, kills `*`. The window
paints each row as one shaped line with a colour run per change and every
glyph forced to the cell width. The grid follows upstream: cells
`round(width / 124)` px wide for sharks, `round(width / 150)` for koi, 1.45
times as tall.

## Layout

- `src/domain`: the simulation and renderers, no I/O (`pond`, `shark`,
  `koi`, `*_skin`, `water`, `raster`, `scene`), and the dismissal policy
  shared with commit-screensaver (`input`, `focus`).
- `src/app.rs`: `Session`, a scene on the host's clock, and the focus watch.
- `src/adapters`: the GPUI window, Hyprland presence and pointer, sibling
  shutdown, the run log.

Gates: `cargo clippy --all-targets -- -D warnings`, `cargo test`, and the
5000-token per-file commit budget.
