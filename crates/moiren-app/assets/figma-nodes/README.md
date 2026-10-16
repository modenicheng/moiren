# Graph node assets

SVG exports from Moiren UI v0.1.2, checked on 2026-10-16.

- Input node: `556:11071`, inner microphone group `I538:10974;42:66`.
- DSP node: `556:11072`, fader icon `654:6729`.
- Output node: `648:12412`, output icon `648:12442`.
- Bus node: `650:6644`, bus icon `654:6740`.
- More menu: `31:52`, rotated 90 degrees to match the node instance;
  input and output ports are 8 px.

The microphone export is 18.3333 by 24.1667 px within its original 28 px
slot. The other node icons are 28 px and the menu icon is 16 px. Preserve
these proportions when scaling the graph camera.
