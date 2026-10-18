# Graph Canvas Rendering Design

Approved scope: preserve existing gpui node styling and gestures while replacing
custom graph rendering. The service snapshot remains the topology authority.
All unrelated working changes must remain untouched.

## Geometry pipeline

- Camera: world/canvas conversion, inverse conversion, rectangles, drag deltas,
  pointer-centered zoom, viewport fitting, and visible world bounds.
- Layout: one world-space description for card, icon, text, meter, labels and
  port anchors. Project all metrics uniformly; no independent minimum font sizes.
- Wires: project cubic control points before adaptive subdivision. Stroke the
  resulting centerline with GPUI's tessellator and hit-test that same centerline.
  A triangle fan cannot correctly fill the old concave ribbon.
- Frame: prepaint resolves Fit from real bounds once. Painting, hit testing and
  the overview share this frame, including its camera and viewport clip.
- Overview: aspect-preserving projection of graph plus visible world bounds,
  using the camera inverse rather than a second set of camera formulas.

## Zoom limits and persistence

Normal manual zoom stays within 25% to 200%. Fit may go below 25% to show a
large graph; subsequent relative zoom remains smooth and anchor-preserving in
that range rather than snapping up to the manual minimum. The finite positive
minimum is `f32::EPSILON`, and saved layouts accept the same lower bound.
Legacy saved zoom above 200% can decrease smoothly without a reverse jump when
zooming in; existing saved layouts remain compatible up to 400%.

## Acceptance

Cover anchor invariance, fractional zoom, inverse transforms, drag deltas, fit
gutters, proportional layouts, multi-input anchors, narrow strokes, reversed and
vertical curves, degenerate endpoints, shared paint/hit geometry, route selection,
first-frame Fit, overview alignment, and viewport clipping. Run app unit and
headless tests, formatting, Clippy, and native rendering captures when supported.
