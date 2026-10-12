# Moiren Figma UI Redesign Design

**Date:** 2026-10-12  
**Status:** Approved for implementation planning  
**Target Figma file:** `dnmEP4We8cGHc0GK33mOeg` (`Moiren UI v0.1.2`)  
**Reference asset:** Only node `20:613` from file `axX8ZopKhTi9XfcfoLotiP` may be read, and only for the Moiren logo.

## 1. Objective

Create a complete, implementation-oriented desktop UI design for Moiren in Figma. The design must be professional, modern, attractive, easy to use, and suitable for long real-time audio sessions.

The deliverable covers a future-facing routing workstation while distinguishing current runtime capabilities from prototype-only functionality. It must not expand into a DAW: there is no timeline, recording arrangement, waveform editor, MIDI editor, automation lane, or plug-in browser.

Success means the Figma file contains:

1. A Tailwind-style variable and token system.
2. Reusable components with complete interaction states.
3. Professional routing and monitoring controls.
4. Graph, Mixer, Matrix, and Devices as four peer workspaces.
5. Settings as an application surface outside the peer workspace navigation.
6. Complete 1440×900 and 1024×720 desktop compositions.
7. Prototype flows and annotations that distinguish runtime-ready and prototype behavior.
8. Exported visual checks for every completed section before the next section begins.

## 2. Product Model

Moiren is a native Windows real-time audio routing workstation. Its central object is a Session that binds sources, processing, routing, destinations, controls, and engine state.

Graph, Mixer, and Matrix are three projections of the same routing model:

- **Graph** explains topology and signal flow.
- **Mixer** exposes gain structure, metering, and monitoring controls.
- **Matrix** exposes source-to-destination relationships at scale.

Devices manages available audio resources and capabilities. It does not edit topology directly. Settings manages application-level preferences and diagnostics. It does not duplicate device selection, routing, or session parameters.

The UI must distinguish:

- the configuration prepared for the next Start;
- the Session currently active in the engine;
- a requested control value;
- an accepted request;
- a value applied by the engine;
- unavailable or missing data;
- prototype-only future behavior.

Unknown real-time data is displayed as `—` or an explicit state. It is never represented by a fabricated zero.

## 3. Global Information Architecture

### 3.1 Single-row application toolbar

The toolbar is 44 px high and remains a single visual layer. It follows the existing hand-authored application-frame direction rather than introducing a navigation rail or a second full-width toolbar.

#### Left: application and project identity

- Moiren mark.
- Optional `Moiren` word label when space allows.
- Current Project or Session selector.
- Save state and save action.

Progressive compression hides the word label and nonessential project iconography before truncating the project name.

Scene selection is not permanently displayed. It belongs in the Project or Session menu. A future user preference may pin it to the toolbar if scene switching becomes frequent.

#### Center: peer workspace navigation

Only these destinations are peers:

1. Graph
2. Mixer
3. Matrix
4. Devices

They use text labels and a restrained violet underline. They are not enclosed in heavy button containers.

#### Right: global transport and window actions

- Minimal engine state indicator.
- Start or Stop action.
- Settings icon.
- Minimize, maximize or restore, and close controls.

Settings sits immediately left of the window controls. It is not a peer workspace.

### 3.2 Engine status bar

A 28 px status bar remains visible at the bottom. It owns detailed engine diagnostics:

- engine phase;
- sample format;
- block size and latency;
- clock source;
- CPU;
- XRUN count;
- application version;
- error entry when needed.

The toolbar communicates transport availability and action. The status bar communicates diagnostics. The same detailed phase text is not duplicated in both places.

### 3.3 Module-local controls

Graph, Mixer, Matrix, and Devices place view-specific controls inside their content surfaces. They do not add a second full-width application toolbar.

## 4. Visual Direction: Signal Atelier

The design is a dark professional workstation with restrained brand character.

- Graphite and near-black surfaces provide the working field.
- Violet is the brand and primary interaction color.
- Cyan identifies live input signal.
- Blue and blue-violet identify output signal and active destinations.
- Amber identifies waiting, pending, candidate, or next-Start state.
- Red is reserved for failures, clipping, XRUNs, and dangerous actions.

The diagonal shuttle geometry from the logo may inform cable direction, selected-edge treatment, and subtle transition motifs. It must not become decorative visual noise.

The interface avoids glassmorphism, large decorative gradients, excessive glow, and oversized pill-shaped controls. Hierarchy comes from luminance, spacing, one-pixel borders, and controlled elevation.

## 5. Foundations and Tokens

The Figma file uses variables and styles. Components consume semantic tokens rather than raw palette values.

### 5.1 Palette scales

Create Tailwind-style scales for:

- `neutral-50` through `neutral-950`;
- `violet-50` through `violet-950`;
- `cyan-50` through `cyan-950`;
- `blue-50` through `blue-950`;
- `amber-50` through `amber-950`;
- `red-50` through `red-950`.

Material 3 may inform role completeness and interaction-state semantics, but Moiren must use its own density and visual appearance.

### 5.2 Semantic colors

At minimum, define:

- `bg-canvas`, `bg-toolbar`, `bg-panel`, `bg-raised`, `bg-overlay`;
- `border-subtle`, `border-default`, `border-strong`, `border-focus`;
- `text-primary`, `text-secondary`, `text-muted`, `text-disabled`, `text-on-accent`;
- `accent-primary`, `accent-primary-hover`, `accent-primary-pressed`, `accent-primary-muted`;
- `signal-input`, `signal-output`, `signal-active`, `signal-inactive`;
- `status-info`, `status-success`, `status-warning`, `status-danger`;
- `state-pending`, `state-unavailable`, `state-prototype`.

### 5.3 Spacing and sizing

Use a 4 px base grid. Define `space-1`, `2`, `3`, `4`, `5`, `6`, `8`, `10`, and `12`.

Control heights are 28, 32, and 36 px. The application toolbar is 44 px and the status bar is 28 px.

Create two density modes:

- `Comfortable` for 1440×900;
- `Compact` for 1024×720.

They are variable modes of one system, not duplicated component libraries.

### 5.4 Shape, type, and motion

- Default radius: 4 px.
- Inputs and panels: 6 px.
- Menus, dialogs, and overlays: 8 px.
- One-pixel borders are the default structural device.
- UI typeface: Inter or Segoe UI Variable.
- Technical typeface: JetBrains Mono or Cascadia Mono.
- Core sizes: 10, 11, 12, 13, 15, and 20 px.
- Icons use a 16 px base and approximately 1.5 px stroke.
- Focus is a visible 2 px violet ring.
- Motion is short and functional; it confirms state and spatial relationships rather than decorating the screen.

All normal body text must reach WCAG 2.1 AA contrast. State cannot be communicated by color alone.

## 6. Component Architecture

Every interactive component includes applicable `default`, `hover`, `pressed`, `focus`, `selected`, `disabled`, and `error` states.

### 6.1 General components

- Primary, Secondary, Ghost, and Danger Button.
- Standard, Toggle, Settings, and Window Icon Button.
- Underline Tab for peer workspace navigation.
- Select, Text Field, Search Field, Checkbox, Switch, and Segmented Control.
- Slider, Numeric Field, and Knob with keyboard adjustment and reset behavior.
- Tooltip, Menu, Context Menu, Popover, Dialog, and Toast.
- Status Badge, Inline Alert, Empty State, Loading State, and Error State.

### 6.2 Workstation structure

- App Toolbar.
- Engine Status Bar.
- Project Selector.
- Transport Control.
- Dock Panel and Panel Header.
- Resize Handle.
- Inspector Section.
- Compact side drawer and bottom drawer.

### 6.3 Graph components

- Graph Node with Source, Processing, Routing, and Output categories.
- Typed Port.
- Cable with active, candidate, pending, unavailable, selected, and muted states.
- Selection Marquee.
- Canvas Toolbar.
- Minimap.
- Node Library item and category.

### 6.4 Mixer components

- Channel Strip.
- Master or Monitor Strip.
- Stereo Peak plus RMS Meter.
- Clip Hold and reset.
- Optional LUFS-S and LUFS-I value card.
- Stereo phase-correlation indicator.
- Fader, Pan Knob, Trim, and unity marker.
- Monitor Source, Mute, Dim, Mono, Left, Right, and Swap controls.
- Reserved Insert Slot without a plug-in browser.

### 6.5 Matrix components

- Source and Destination group header.
- Route Cell with Off, On, Selected, Muted, Pending, and Unavailable states.
- Level Cell.
- Frozen row and column headers.
- Matrix Inspector summary.

### 6.6 Device and diagnostic components

- Device Row.
- Capability Tag.
- Active, Default, Next Start, and Unavailable badges.
- Endpoint identity and stable-ID field.
- Format, buffer, latency, clock, drift, XRUN, and publisher diagnostic rows.
- Short-window Sparkline only where real data exists.
- Explicit Unavailable, Waiting, Paused, No Publisher, and Closed states.

### 6.7 Runtime annotations

Each reusable component and page section must carry implementation metadata:

- proposed Slint component name;
- main properties;
- callbacks;
- optional content slots;
- `Runtime-ready` or `Prototype` status.

Core states must not require blur, complex filters, or effects that are impractical in Slint.

## 7. Workspace Designs

### 7.1 Graph

The large layout uses:

- 240 px Node Library;
- flexible Graph Canvas;
- 288 px Inspector.

The Canvas owns local Select, Hand, Fit, Zoom, and Minimap controls. Nodes may be selected and moved. Future prototype interactions include drag-in node creation, port-to-port connections, multi-selection, and context menus.

Selecting a node exposes its parameters. Selecting a cable exposes source, destination, channel mapping, state, and signal level.

Active routing is solid. Candidate or next-Start routing is dashed and explicitly labeled. Unavailable routing is subdued and includes a reason.

In Compact mode, Library and Inspector become mutually exclusive drawers. The Canvas remains the primary surface.

### 7.2 Mixer

Mixer is a routing and monitoring view, not a DAW console.

The content header contains only channel or bus filtering, density control, and meter mode. A Channel Strip follows this order:

1. header;
2. input meter;
3. source trim;
4. reserved insert area;
5. pan;
6. fader;
7. output meter;
8. monitoring controls.

The Master or Monitor Strip remains fixed on the right. Channel strips scroll horizontally. A Session with one stereo path presents one truthful complete strip rather than fabricated channels.

Comfortable strip width is 144 px. Master or Monitor width is 176 px. Compact mode preserves usable hit targets, scrolls horizontally, and hides low-priority labels before hiding numeric values or status.

### 7.3 Matrix

Matrix has a frozen Source column, a frozen Destination row, and a scrollable Route Cell grid. Cells may expose route gain without placing excessive metadata in the grid.

Sources and destinations can be grouped by device, application, or bus. The Inspector explains the selected intersection.

Comfortable Source Header width is 180 px and Destination Header width is 104 px. Compact mode moves the Inspector to a bottom drawer and preserves two-axis scrolling with frozen headers.

### 7.4 Devices

Devices is a peer workspace because endpoint and application-resource management is a primary product task.

Comfortable mode uses:

- 176 px category navigation;
- a flexible device table;
- 320 px detail panel.

Categories are Inputs, Outputs, Applications, and Clock. Device Detail shows capabilities, default state, active state, candidate state, availability, identity, and diagnostics.

`Active`, `Default`, and `Next start` are separate states. Missing endpoints retain their stable identity and explain why they are unavailable.

Compact mode replaces category navigation with a top Segmented Control and opens Detail as a side drawer.

### 7.5 Settings

Settings opens from the gear immediately left of the window controls. It is a separate full-height application surface, not a peer tab.

Sections are:

1. Audio Engine;
2. Appearance;
3. Shortcuts;
4. Diagnostics.

Closing Settings restores the previous workspace, selection, viewport, panel state, and scroll state.

Settings contains application preferences only. Device selection, routing, and session parameters remain in their owning workspaces.

## 8. Cross-workspace Behavior

Graph, Mixer, and Matrix operate on one routing model. Changes synchronize selection and highlights across representations where a direct mapping exists.

Devices owns resource availability and capabilities. It must not silently edit Graph topology.

The current Session, global transport, active or candidate distinction, and engine phase persist across all workspaces.

Control feedback follows:

1. local edit becomes `Pending`;
2. the service accepts or rejects the request;
3. the engine reports the applied revision and effective frame;
4. the UI enters stable applied state.

Catalog refresh errors preserve the last successful data. Local failures remain local. Only engine-wide failure escalates to the global transport and status bar.

## 9. Responsive Rules

### 9.1 Comfortable: 1440×900

- Toolbar: 44 px.
- Status bar: 28 px.
- Content height: 828 px.
- Three-panel workspaces remain visible together.
- Resizable panels use documented minimums, maximums, and double-click reset.

### 9.2 Compact: 1024×720

- Toolbar and status bar retain 44 px and 28 px heights.
- Graph, Mixer, Matrix, and Devices remain visible as text navigation.
- Start or Stop, Settings, and window controls remain visible.
- Project identity compresses progressively.
- Detailed engine information moves to the status bar.
- Library, Inspector, and Device Detail use mutually exclusive drawers.
- Mixer scrolls instead of shrinking controls below usable size.
- Matrix retains frozen headers.
- Density changes from Comfortable to Compact by reducing control height and spacing one step and hiding low-priority labels.

## 10. Keyboard and Interaction Rules

- `Ctrl+1`: Graph.
- `Ctrl+2`: Mixer.
- `Ctrl+3`: Matrix.
- `Ctrl+4`: Devices.
- `Ctrl+,`: Settings.
- `Esc`: close the topmost menu, drawer, dialog, or Settings surface.

Regular edits support Undo and Redo. Dangerous actions require confirmation. Hover, focus, selection, active engine state, pending state, and unavailable state have distinct visuals and semantics.

## 11. Error and Empty-state Rules

- A catalog refresh failure preserves and timestamps the last successful catalog.
- A missing publisher does not render a zero meter.
- Workspace errors do not erase valid neighboring content.
- Engine failure escalates globally.
- Prototype controls are labeled `Prototype` and are never presented as runtime-connected.

## 12. Figma File Structure

Create these pages in order:

1. `00 Foundations`
2. `01 Components`
3. `02 Shell`
4. `03 Graph`
5. `04 Mixer`
6. `05 Matrix`
7. `06 Devices`
8. `07 Settings`
9. `08 Prototype`

The target file is initially blank. Do not import or inspect any previous UI design except the explicitly allowed logo node.

## 13. Incremental Visual Validation

Work proceeds one page at a time. After each meaningful block:

1. create or update the variables, components, or layouts;
2. export a screenshot of the affected block;
3. inspect hierarchy, alignment, contrast, density, state semantics, and Slint feasibility;
4. correct defects before proceeding;
5. export the complete page at the end of the phase.

Representative checks include:

- 100% zoom readability;
- 1440×900 and 1024×720 full-frame composition;
- selected, pending, unavailable, and error states;
- long device and project names;
- missing meters and missing devices;
- one-channel and many-channel Mixer layouts;
- small and large Matrix dimensions;
- keyboard focus visibility;
- panel-resize limits;
- absence of unsupported blur or excessive effects.

## 14. Implementation Order

1. Foundations and semantic variables.
2. General components and workstation controls.
3. App shell in both sizes.
4. Graph in both sizes.
5. Mixer in both sizes.
6. Matrix in both sizes.
7. Devices in both sizes.
8. Settings in both sizes.
9. Cross-workspace prototype flows.
10. Final annotation and Slint implementation map.

A phase cannot be considered complete until its screenshots have been visually inspected and corrected.

## 15. Explicit Non-goals

- Reusing or taking inspiration from previous Moiren UI designs other than the logo.
- Designing a timeline, arranger, recorder, waveform editor, MIDI editor, automation system, or plug-in marketplace.
- Treating Settings as a peer workspace.
- Adding a second full-width toolbar.
- Filling the interface with fabricated runtime values.
- Requiring browser technology, Electron, Chromium, or WebView.
- Changing the Slint code during the Figma design phase.
