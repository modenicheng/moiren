# Graph node assets

SVG exports from Moiren UI v0.1.2, checked on 2026-10-16.

- More menu: `31:52`, rotated 90 degrees to match the node instance.
- Input and output ports: 8 px dots (`in port`, `out port`).

The node kind glyphs (microphone, output, bus) are no longer taken from
this export: they came in three different boxes (18.3 × 24.2, 28 × 28,
28 × 28 px) and read at different sizes next to the Tabler glyphs the
other kinds use. Every kind icon now comes from `assets/icons/` (Tabler,
24-unit box, 2-unit padding, stroke 2) — the same files the node library
paints — so all cards share one visual size.
