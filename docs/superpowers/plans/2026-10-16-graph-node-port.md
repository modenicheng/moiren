# Graph node port

1. Port the five Figma node families using local icons and shared geometry.
2. Add a searchable, grouped node library, session node list, and native drag/drop.
3. Resolve new node positions only after the service confirms creation.
4. Restrict Inspector to selected-node properties, bindings, and parameters.
5. Add independent channel strips, send levels and explicit pre/post taps, M/S and cycling channel modes.
6. Dock buses/outputs on the right, add drag ordering and resizable widths, persist project and tray presentation.
7. Test DSP, allocation contracts, persistence and native/headless interactions; update FRONTEND.md.

Preserve existing uncommitted work; do not commit. Prepare strip processors off the audio thread and preserve the no-allocation render contract. Inspector must not copy the v0.1.2 layout. Run Cargo serially with -j 2 and tests with --test-threads=2.
