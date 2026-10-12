# Moiren Figma UI Redesign Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Build a complete token-driven Moiren desktop UI in the target Figma file, with reusable components, four peer workspaces, separate Settings, and validated 1440×900 and 1024×720 layouts.

**Architecture:** Build the Figma file in dependency order: local variables and styles, icon and control components, workstation components, application shell, workspace screens, and cross-workspace prototypes. Every reusable visual property is bound to a semantic token where Figma supports it. Every individual component or page block has an immediate screenshot gate: inspect it, fix it if necessary, and do not begin the next component until the current screenshot passes.

**Tech Stack:** Figma Design, Figma Plugin API via `use_figma`, Figma variables and component sets, Material 3 library discovery for semantic reference only, local SVG assets, Slint-oriented annotations, `get_metadata`, `get_screenshot`, `download_assets`, and `upload_assets`.

---

## Locked Scope and Safety Rules

- Target file: `dnmEP4We8cGHc0GK33mOeg`.
- The target starts as an empty file.
- The only permitted read from the previous Moiren file `axX8ZopKhTi9XfcfoLotiP` is node `20:613`, which contains the logo reference.
- Do not inspect pages, metadata, components, variables, or screenshots outside that logo node in the previous file.
- Do not modify Slint or Rust source during this Figma phase.
- Do not flatten complete screens into screenshots. Final UI layers must remain editable.
- Settings is opened from the gear immediately left of window controls; it is not a fifth peer tab.
- Graph, Mixer, Matrix, and Devices are the four peer workspaces.
- Scene selection stays in the Project or Session menu and is not permanently displayed in the toolbar.
- A screenshot gate follows every individual component or coherent indivisible component set. If a fix is needed, take one post-fix screenshot; that passing screenshot is the component’s final evidence.
- A screenshot gate also follows each meaningful page block: toolbar, status bar, library, canvas, inspector, strip bank, matrix grid, device table, and settings section.
- Never continue past a failed structural or visual gate.
- `use_figma` calls are sequential. Never mutate the same Figma file with concurrent `use_figma` calls.
- Include `resource:figma-use,resource:figma-generate-library,resource:figma-generate-design` in mutating `use_figma` calls.

## Files and External State

**Create in repository:**
- `docs/superpowers/plans/2026-10-12-moiren-figma-ui-redesign.md` — this implementation plan.

**Create outside repository during execution:**
- `C:\Windows\Temp\moiren-figma-redesign-state.json` — durable state ledger containing page, collection, variable, style, component, and frame IDs.
- `C:\Windows\Temp\moiren-figma-assets\` — downloaded SVG assets from the explicitly allowed logo node only.

**Modify in Figma:**
- `00 Foundations`
- `01 Components`
- `02 Shell`
- `03 Graph`
- `04 Mixer`
- `05 Matrix`
- `06 Devices`
- `07 Settings`
- `08 Prototype`

## State Ledger Shape

Use this exact top-level structure and update it after each successful creation call:

```json
{
  "runId": "moiren-ui-redesign-2026-10-12",
  "fileKey": "dnmEP4We8cGHc0GK33mOeg",
  "step": "discovery",
  "pages": {},
  "collections": {},
  "variables": {},
  "styles": {},
  "components": {},
  "frames": {},
  "completed": [],
  "pendingValidation": []
}
```

The Figma API return value is authoritative for every ID. Never guess an ID. Every mutation must return all created or mutated IDs.

---

### Task 1: Inspect the Blank Target and Initialize the Ledger

**Files:**
- Create: `C:\Windows\Temp\moiren-figma-redesign-state.json`
- Modify: target Figma file, read-only inspection only

- [ ] **Step 1: Inspect pages, local variables, styles, components, and available fonts**

Run a read-only `use_figma` call with `skillNames: "resource:figma-use"`:

```javascript
const collections = await figma.variables.getLocalVariableCollectionsAsync();
const variables = await figma.variables.getLocalVariablesAsync();
const textStyles = await figma.getLocalTextStylesAsync();
const effectStyles = await figma.getLocalEffectStylesAsync();
const pages = figma.root.children.map(page => ({
  id: page.id,
  name: page.name,
  children: page.children.map(node => ({
    id: node.id,
    name: node.name,
    type: node.type,
    x: node.x,
    y: node.y,
    width: node.width,
    height: node.height
  }))
}));
const fonts = await figma.listAvailableFontsAsync();
return {
  pages,
  collections: collections.map(c => ({ id: c.id, name: c.name, modes: c.modes, variableIds: c.variableIds })),
  variables: variables.map(v => ({ id: v.id, name: v.name, type: v.resolvedType, collectionId: v.variableCollectionId })),
  textStyles: textStyles.map(s => ({ id: s.id, name: s.name })),
  effectStyles: effectStyles.map(s => ({ id: s.id, name: s.name })),
  availableFonts: fonts
    .filter(f => ["Inter", "Segoe UI Variable", "Cascadia Mono", "JetBrains Mono"].includes(f.fontName.family))
    .map(f => f.fontName)
};
```

Expected: one blank page named `Page 1`, no local design-system variables or components, and at least one usable UI family plus one usable mono family.

- [ ] **Step 2: Select exact fonts from returned evidence**

Use this order without guessing unavailable styles:

1. UI: `Segoe UI Variable` if Regular, SemiBold or Semi Bold, and Bold are available.
2. UI fallback: `Inter` with Regular, Medium, Semi Bold, and Bold.
3. Mono: `Cascadia Mono` if Regular and SemiBold or Semi Bold exist.
4. Mono fallback: `JetBrains Mono` with Regular and Medium.

Record exact Figma family and style strings in the ledger.

- [ ] **Step 3: Write the initial state ledger**

Write the JSON structure above with the discovery result summarized and `completed: ["discovery"]`.

- [ ] **Step 4: Verify the ledger is parseable**

Run:

```bash
python -I -c "import json; json.load(open(r'C:\Windows\Temp\moiren-figma-redesign-state.json', encoding='utf-8')); print('ledger-ok')"
```

Expected: `ledger-ok`.

---

### Task 2: Create the Page Skeleton and Import the Allowed Logo

**Files:**
- Modify: target Figma file
- Modify: `C:\Windows\Temp\moiren-figma-redesign-state.json`
- Create: `C:\Windows\Temp\moiren-figma-assets\*`

- [ ] **Step 1: Create or rename the nine deterministic pages**

Use one `use_figma` call. Rename `Page 1` to `00 Foundations`, then create the remaining pages. Do not switch pages during this call.

```javascript
const names = [
  "00 Foundations", "01 Components", "02 Shell", "03 Graph", "04 Mixer",
  "05 Matrix", "06 Devices", "07 Settings", "08 Prototype"
];
const createdNodeIds = [];
let first = figma.root.children[0];
first.name = names[0];
createdNodeIds.push(first.id);
for (const name of names.slice(1)) {
  let page = figma.root.children.find(p => p.name === name);
  if (!page) {
    page = figma.createPage();
    page.name = name;
  }
  createdNodeIds.push(page.id);
}
return {
  mutatedNodeIds: createdNodeIds,
  pages: figma.root.children.map(p => ({ id: p.id, name: p.name }))
};
```

Expected: exactly the nine named pages in the specified order.

- [ ] **Step 2: Update page IDs in the ledger**

Persist the returned page IDs immediately.

- [ ] **Step 3: Download assets from only the allowed old-file node**

Call `download_assets` with:

```json
{
  "fileKey": "axX8ZopKhTi9XfcfoLotiP",
  "nodeId": "20:613",
  "defaultFormat": "svg"
}
```

Download only URLs returned for that node into `C:\Windows\Temp\moiren-figma-assets\`. Do not make another read call against the previous file.

- [ ] **Step 4: Place editable logo assets on `00 Foundations`**

Upload/import the symbol, wordmark, and monochrome mark that are present in the allowed node. Put them inside an auto-layout frame named `Brand / Moiren Logo`, with a neutral canvas background and labels `Symbol`, `Wordmark`, and `Monochrome`.

Expected returned evidence: page ID, showcase frame ID, imported vector IDs, and bounds.

- [ ] **Step 5: Screenshot the logo block immediately**

Call `get_screenshot` for `Brand / Moiren Logo` at `maxDimension: 1600`.

Pass criteria:

- Logo geometry is not distorted.
- Symbol has adequate padding.
- Dark and monochrome versions are legible.
- No other legacy design content is present.

If any item fails, fix only the logo showcase and take one post-fix screenshot before Task 3.

---

### Task 3: Build Primitive and Semantic Variables

**Files:**
- Modify: target Figma file
- Modify: state ledger

- [ ] **Step 1: Create the primitive color collection**

Create collection `Moiren / Primitives` with mode `Value`. Create these COLOR variables with `scopes = []` and WEB code syntax `var(--moiren-{name-with-slashes-replaced-by-hyphens})`:

```text
neutral/50  #F6F8FA    neutral/100 #E8ECF1    neutral/200 #D4DAE2
neutral/300 #B4BCC8    neutral/400 #8C96A6    neutral/500 #667080
neutral/600 #3A424E    neutral/700 #2A303A    neutral/800 #1B2028
neutral/900 #11151B    neutral/950 #090B0F
violet/50   #F8F5FF    violet/100  #EFE7FD    violet/200  #DECCFB
violet/300  #C6A9F7    violet/400  #A985F0    violet/500  #8B64E1
violet/600  #7549C6    violet/700  #6232A6    violet/800  #4A2281
violet/900  #35165F    violet/950  #251047
cyan/50     #ECFEFF    cyan/100    #CFFAFE    cyan/200    #A5F3FC
cyan/300    #67E8F9    cyan/400    #22D3EE    cyan/500    #06B6D4
cyan/600    #0891B2    cyan/700    #0E7490    cyan/800    #155E75
cyan/900    #164E63    cyan/950    #083344
blue/50     #EFF6FF    blue/100    #DBEAFE    blue/200    #BFDBFE
blue/300    #93C5FD    blue/400    #60A5FA    blue/500    #3B82F6
blue/600    #2563EB    blue/700    #1D4ED8    blue/800    #1E40AF
blue/900    #1E3A8A    blue/950    #172554
amber/50    #FFFBEB    amber/100   #FEF3C7    amber/200   #FDE68A
amber/300   #FCD34D    amber/400   #FBBF24    amber/500   #F59E0B
amber/600   #D97706    amber/700   #B45309    amber/800   #92400E
amber/900   #78350F    amber/950   #451A03
red/50      #FEF2F2    red/100     #FEE2E2    red/200     #FECACA
red/300     #FCA5A5    red/400     #F87171    red/500     #EF4444
red/600     #DC2626    red/700     #B91C1C    red/800     #991B1B
red/900     #7F1D1D    red/950     #450A0A
white       #FFFFFF    black       #000000
```

Return collection ID and every variable ID. Persist them.

- [ ] **Step 2: Create semantic color aliases**

Create collection `Moiren / Color` with one mode named `Dark`. Alias every semantic token to a primitive; do not duplicate raw values.

```text
bg/canvas            → neutral/950
bg/toolbar           → neutral/900
bg/panel             → neutral/900
bg/panel-subtle      → neutral/800
bg/raised            → neutral/800
bg/overlay           → neutral/900
bg/input             → neutral/950
border/subtle        → neutral/800
border/default       → neutral/700
border/strong        → neutral/600
border/focus         → violet/400
text/primary         → neutral/50
text/secondary       → neutral/300
text/muted           → neutral/400
text/disabled        → neutral/600
text/on-accent       → white
accent/primary       → violet/600
accent/hover         → violet/500
accent/pressed       → violet/700
accent/muted         → violet/900
signal/input         → cyan/400
signal/output        → blue/400
signal/active        → violet/400
signal/inactive      → neutral/600
status/info          → blue/400
status/success       → cyan/400
status/warning       → amber/400
status/danger        → red/400
state/pending        → amber/400
state/unavailable    → neutral/500
state/prototype      → violet/300
meter/safe           → cyan/400
meter/high           → amber/400
meter/clip           → red/500
```

Set targeted scopes:

- `bg/*`: `FRAME_FILL`, `SHAPE_FILL`.
- `border/*`: `STROKE_COLOR`.
- `text/*`: `TEXT_FILL`.
- `accent/*`, `signal/*`, `status/*`, `state/*`, `meter/*`: `FRAME_FILL`, `SHAPE_FILL`, `STROKE_COLOR`, except any explicit text-only alias.

- [ ] **Step 3: Create dimension and density variables**

Create these collections:

- `Moiren / Spacing`, mode `Comfortable`, add `Compact` if plan supports a second mode.
- `Moiren / Radius`, mode `Value`.
- `Moiren / Size`, modes `Comfortable` and `Compact`.

Create exact values:

```text
spacing/1 4/4     spacing/2 8/6     spacing/3 12/8
spacing/4 16/12   spacing/5 20/16   spacing/6 24/20
spacing/8 32/24   spacing/10 40/32  spacing/12 48/40
radius/none 0     radius/sm 4       radius/md 6
radius/lg 8       radius/full 999
size/control-sm 28/28    size/control-md 32/30    size/control-lg 36/32
size/toolbar 44/44       size/statusbar 28/28     size/icon 16/16
size/panel-header 36/32  size/hit-target 32/32
stroke/thin 1            stroke/focus 2
```

Values before and after `/` are Comfortable and Compact. Radius and stroke use one mode.

- [ ] **Step 4: Validate variables structurally**

Run a read-only `use_figma` audit returning collection names, modes, variable counts, missing code syntax, missing targeted scopes, and broken aliases.

Expected:

- Six collections or fewer if compatible size variables are combined deliberately.
- All primitive scopes empty.
- All semantic and dimension variables have targeted scopes.
- All variables have WEB code syntax.
- Broken alias count is zero.

- [ ] **Step 5: Build a color and spacing specimen on `00 Foundations`**

Create editable swatch rows for the six color scales, semantic surface examples, spacing rulers, radius examples, and Comfortable/Compact control-height comparison. Bind all specimen properties to the variables.

- [ ] **Step 6: Screenshot the Foundations specimen immediately**

Pass criteria:

- Each scale progresses visibly and consistently.
- Semantic backgrounds are distinguishable without excessive contrast.
- Violet remains the primary accent.
- Cyan, blue, amber, and red are clearly separable.
- Compact and Comfortable specimens are visibly different but share the same system.
- No labels are clipped.

Fix before continuing.

---

### Task 4: Create Typography, Effects, and Icon Components

**Files:**
- Modify: `00 Foundations`, `01 Components`, ledger

- [ ] **Step 1: Create text styles using the verified font strings**

Create:

```text
UI/Title          20 px, 26 line, Semibold
UI/Heading        15 px, 20 line, Semibold
UI/Body           13 px, 18 line, Regular
UI/Label          12 px, 16 line, Medium
UI/Label Small    11 px, 14 line, Medium
UI/Caption        10 px, 14 line, Regular
Mono/Metric       11 px, 14 line, Regular
Mono/Metric Small 10 px, 12 line, Regular
Mono/Value        12 px, 16 line, Medium
```

Use `Inter` or `Segoe UI Variable` exactly as discovered. Use `Cascadia Mono` or `JetBrains Mono` exactly as discovered. Apply semantic text color variables in specimen instances, not directly in the styles.

- [ ] **Step 2: Create effect styles**

Create only:

- `Elevation/Menu`: two restrained dark shadows for menus and popovers.
- `Elevation/Dialog`: stronger dark shadow for modal surfaces.
- `Elevation/Drag`: tight shadow for a dragged node or strip.

No background blur.

- [ ] **Step 3: Create typography and elevation specimens**

Place them in `00 Foundations`, bound to semantic colors.

- [ ] **Step 4: Screenshot typography/elevation specimen**

Pass criteria: fonts are correct, small labels remain legible, mono metrics align, no descenders or accents are clipped, and shadows are visible without looking luminous.

- [ ] **Step 5: Import local SVG icons as reusable components**

Use exact SVG sources from `crates/moiren-app/ui/icons/` and the allowed logo download. Required icon components:

```text
Icon/Graph, Icon/Mixer, Icon/Matrix, Icon/Devices, Icon/Settings,
Icon/Folder, Icon/Save, Icon/Chevron Down, Icon/Play, Icon/Stop,
Icon/Minimize, Icon/Maximize, Icon/Close, Icon/Search, Icon/Refresh,
Icon/Plus, Icon/Minus, Icon/Fit, Icon/Hand, Icon/Pointer,
Icon/More, Icon/Warning, Icon/Error, Icon/Check, Icon/Info,
Icon/Mute, Icon/Dim, Icon/Mono, Icon/Swap, Icon/Left, Icon/Right,
Icon/Input, Icon/Output, Icon/Application, Icon/Clock
```

Every icon component uses a 16×16 frame, editable vectors, semantic icon color binding, and a clear description. Missing local icons may be created as minimal SVG paths only when the visual role is unambiguous.

- [ ] **Step 6: Screenshot the icon grid immediately**

Pass criteria: consistent optical size, stroke weight, alignment, and no distorted paths. Fix before building controls.

---

### Task 5: Build and Validate Button Components One by One

**Files:**
- Modify: `01 Components`, ledger

- [ ] **Step 1: Create `Button / Filled`**

Axes:

- Style: `Primary`, `Danger`.
- Size: `Small`, `Medium`, `Large`.
- State: `Default`, `Hover`, `Pressed`, `Focus`, `Disabled`.

Exactly 30 variants. Expose `Label` TEXT, `Show Icon` BOOLEAN, and `Icon` INSTANCE_SWAP. Bind fill, text, border, focus ring, padding, gap, radius, and height to variables.

- [ ] **Step 2: Screenshot `Button / Filled` immediately**

Check variant grid, state differentiation, focus visibility, disabled contrast, icon alignment, text clipping, and 28/32/36 px heights. Fix and rescreenshot if needed.

- [ ] **Step 3: Create `Button / Quiet`**

Axes:

- Style: `Secondary`, `Ghost`.
- Size: `Small`, `Medium`, `Large`.
- State: `Default`, `Hover`, `Pressed`, `Focus`, `Disabled`.

Exactly 30 variants with the same public properties.

- [ ] **Step 4: Screenshot `Button / Quiet` immediately**

Check that Secondary and Ghost are distinct but restrained. Fix before continuing.

- [ ] **Step 5: Create `Icon Button / Standard`**

Axes:

- Intent: `Neutral`, `Accent`, `Danger`.
- Size: `Small`, `Medium`.
- State: `Default`, `Hover`, `Pressed`, `Focus`, `Disabled`.

Exactly 30 variants. Expose `Icon` INSTANCE_SWAP and tooltip documentation.

- [ ] **Step 6: Screenshot `Icon Button / Standard` immediately**

Check hit-target size, icon centering, focus ring, and danger hover.

- [ ] **Step 7: Create `Icon Button / Toggle`**

Axes: Selected `False/True`; State `Default/Hover/Pressed/Focus/Disabled`. Exactly 10 variants.

- [ ] **Step 8: Screenshot `Icon Button / Toggle` immediately**

Check that selection is not communicated by color alone; use fill plus border or icon-weight treatment.

- [ ] **Step 9: Create `Window Button`**

Axes: Intent `Normal/Close`; State `Default/Hover/Pressed/Focus`. Use icon swap for Minimize, Maximize, and Close rather than an action variant axis.

- [ ] **Step 10: Screenshot `Window Button` immediately**

Check Windows-style close danger hover and equal hit targets.

---

### Task 6: Build Navigation, Status, and Badge Components One by One

- [ ] **Step 1: Create `Workspace Tab`**

Axes: State `Default/Hover/Selected/Focus/Disabled`; expose Label and Icon visibility. Selected state uses text plus 2 px underline.

- [ ] **Step 2: Screenshot `Workspace Tab` immediately**

- [ ] **Step 3: Create `Status Badge`**

Axes: Intent `Info/Active/Pending/Warning/Danger/Unavailable/Prototype`; Density `Comfortable/Compact`. Use icon, label, and color.

- [ ] **Step 4: Screenshot `Status Badge` immediately**

- [ ] **Step 5: Create `Engine Status Indicator`**

Axes: Phase `Idle/Starting/Running/Stopping/Failed/Exiting`; Density `Full/Dot`. Expose phase label.

- [ ] **Step 6: Screenshot `Engine Status Indicator` immediately**

- [ ] **Step 7: Create `Project Selector`**

States: Default/Hover/Open/Focus/Disabled; support long-name truncation and save-state icon.

- [ ] **Step 8: Screenshot `Project Selector` immediately**, including a long project-name specimen.

- [ ] **Step 9: Create `Transport Control`**

States: Ready/Running/Pending/Disabled/Failed; action switches Start/Stop without changing width.

- [ ] **Step 10: Screenshot `Transport Control` immediately**

---

### Task 7: Build Form and Selection Controls One by One

Create each control, then immediately screenshot its dedicated showcase frame before starting the next:

- [ ] `Text Field`: Size Small/Medium; State Default/Hover/Focus/Error/Disabled; label, helper, value, placeholder.
- [ ] `Search Field`: State Default/Hover/Focus/Has Value/Disabled; search and clear icons.
- [ ] `Select`: State Default/Hover/Open/Focus/Error/Disabled; label and selected value.
- [ ] `Checkbox`: Checked False/True/Mixed; State Default/Hover/Focus/Disabled.
- [ ] `Switch`: Checked False/True; State Default/Hover/Focus/Disabled.
- [ ] `Segmented Control`: two-to-four item container using a `Building Blocks/Segment` component; State Default/Hover/Selected/Focus/Disabled.

Each screenshot must check state differentiation, 32 px compact hit targets, label clipping, and focus visibility.

---

### Task 8: Build Numeric and Audio Parameter Controls One by One

Create and validate sequentially:

- [ ] `Numeric Field`: Unit `dB/%/ms/Hz/frames`; State Default/Hover/Focus/Error/Disabled; mono value and stepper affordance. Screenshot immediately.
- [ ] `Slider / Linear`: Orientation Horizontal/Vertical; State Default/Hover/Focus/Disabled; Default marker and danger range. Screenshot immediately.
- [ ] `Knob`: Size Small/Medium; State Default/Hover/Focus/Disabled; value arc, unity marker, mono value. Screenshot immediately.
- [ ] `Fader`: Size Channel/Master; State Default/Hover/Focus/Disabled; dB scale, unity marker, cap, optional active value. Screenshot immediately.
- [ ] `Parameter Feedback`: State Pending/Accepted/Applied/Rejected; expose requested value, revision, and effective frame. Screenshot immediately.

---

### Task 9: Build Feedback and Overlay Components One by One

Create and screenshot sequentially:

- [ ] `Tooltip` — one- and two-line specimens.
- [ ] `Menu Item` and `Context Menu` — default, hover, selected, disabled, destructive, submenu.
- [ ] `Popover` — title, content slot, optional footer.
- [ ] `Dialog` — info, confirmation, danger specimens.
- [ ] `Toast` — info, success, warning, error.
- [ ] `Inline Alert` — info, warning, error.
- [ ] `Empty State` — title, explanation, optional action.
- [ ] `Data State` — Unavailable, Waiting, Paused, No Publisher, Closed.

Visual gate for each: no excessive rounding, no glass blur, correct elevation, readable text, and semantic icon plus color.

---

### Task 10: Build Shell and Panel Components One by One

Create and screenshot sequentially:

- [ ] `Panel Header` — title, optional count, actions, collapse.
- [ ] `Dock Panel` — default, focused, empty, error.
- [ ] `Resize Handle` — idle, hover, dragging.
- [ ] `Side Drawer` — left and right; overlay and docked specimens.
- [ ] `Bottom Drawer` — collapsed, open, resizing.
- [ ] `App Toolbar` — 1440 and 1024 component specimens with the locked single-row hierarchy.
- [ ] `Engine Status Bar` — 1440 and 1024 specimens.

Toolbar screenshot pass criteria:

- Only Graph, Mixer, Matrix, Devices are peer tabs.
- Settings is immediately left of window controls.
- Scene is absent from the persistent toolbar.
- No second top-level row.
- Start/Stop remains visible at 1024 width.
- Progressive project-name compression works.

---

### Task 11: Build Meter and Monitoring Components One by One

Create and screenshot sequentially:

- [ ] `Meter / Mono Peak RMS` — normal, high, clip, unavailable.
- [ ] `Meter / Stereo Peak RMS` — L/R, peak hold, clip latch, unavailable.
- [ ] `LUFS Value` — LUFS-S and LUFS-I, waiting, unavailable.
- [ ] `Phase Correlation` — negative, neutral, positive, unavailable.
- [ ] `Sparkline / Diagnostic` — normal, warning, no data.
- [ ] `Monitor Control Group` — Mute, Dim, Mono, Left, Right, Swap.
- [ ] `Gain Stage Header` — Source Trim, Node Gain, Bus Gain, Output Gain.

Meters must never show zero as the unavailable specimen. Use explicit data-state overlays.

---

### Task 12: Build Graph Components One by One

Create and screenshot sequentially:

- [ ] `Graph Port` — Input/Output; Audio/Control; Idle/Hover/Connected/Selected/Unavailable.
- [ ] `Graph Cable` — Active/Candidate/Pending/Selected/Muted/Unavailable; Stereo/Mono.
- [ ] `Graph Node / Source` — physical input, application, tone specimens.
- [ ] `Graph Node / Processing` — Gain and Pan specimens.
- [ ] `Graph Node / Routing` — Split and Merge prototype specimens.
- [ ] `Graph Node / Output` — device and bus specimens.
- [ ] `Node Library Item` — category, search result, dragging, unavailable.
- [ ] `Canvas Toolbar` — Select, Hand, Fit, Zoom Out/In, Minimap toggle.
- [ ] `Minimap` — viewport rectangle and node overview.
- [ ] `Inspector Section` — collapsed and expanded, parameter and route specimens.

All graph screenshots must verify that active and candidate routes differ by line style as well as color.

---

### Task 13: Build Mixer Components One by One

Create and screenshot sequentially:

- [ ] `Channel Header` — physical, application, tone, bus, unavailable.
- [ ] `Insert Slot / Reserved` — empty and prototype.
- [ ] `Channel Strip` — idle, active, selected, pending, unavailable.
- [ ] `Master Monitor Strip` — active, dimmed, muted, clip.
- [ ] `Channel Bank Header` — filter, density, meter mode.

Channel Strip must follow the locked order: Header → Input Meter → Trim → Reserved Insert → Pan → Fader → Output Meter → Monitor Controls.

---

### Task 14: Build Matrix Components One by One

Create and screenshot sequentially:

- [ ] `Matrix Source Header` — device, application, bus, collapsed group.
- [ ] `Matrix Destination Header` — output, bus, monitor, collapsed group.
- [ ] `Route Cell` — Off/On/Selected/Muted/Pending/Unavailable.
- [ ] `Level Cell` — gain display, edit focus, clipping warning.
- [ ] `Matrix Inspector Summary` — source, destination, mapping, level, state.

Route Cell must remain legible at compact density and communicate state without color alone.

---

### Task 15: Build Device and Diagnostic Components One by One

Create and screenshot sequentially:

- [ ] `Device Category Item` — Inputs/Outputs/Applications/Clock; default/selected/focus.
- [ ] `Device Row` — available, active, default, next start, unavailable, selected.
- [ ] `Capability Tag` — format, sample rate, channels, clock.
- [ ] `Device Detail Header` — identity, stable ID, badges, actions.
- [ ] `Diagnostic Row` — format, buffer, latency, clock source, drift, XRUN, publisher.
- [ ] `Catalog Error Banner` — last-success timestamp and refresh action.

Long device names and stable IDs must be tested in screenshots.

---

### Task 16: Compose and Validate the Two Shell Frames

**Files:**
- Modify: `02 Shell`, ledger

- [ ] **Step 1: Compose `Shell / 1440×900`**

Use instances of App Toolbar and Engine Status Bar. Create an empty content region using `bg/canvas`. Toolbar contains:

- logo and Moiren;
- Project Selector;
- Graph, Mixer, Matrix, Devices tabs;
- engine dot and Transport Control;
- Settings Icon Button;
- Minimize, Maximize, Close.

- [ ] **Step 2: Screenshot the 1440 toolbar block immediately**

Fix before continuing.

- [ ] **Step 3: Screenshot the 1440 status bar immediately**

Fix before continuing.

- [ ] **Step 4: Screenshot the complete 1440 shell**

Check visual balance, center-tab alignment, empty content hierarchy, and no duplicated detailed phase text.

- [ ] **Step 5: Compose `Shell / 1024×720`**

Keep four text tabs, Start/Stop, Settings, and window controls. Hide the Moiren word label if needed, truncate project name, and move detailed engine metrics to the status bar.

- [ ] **Step 6: Screenshot the 1024 toolbar block immediately**

- [ ] **Step 7: Screenshot the 1024 status bar immediately**

- [ ] **Step 8: Screenshot the complete 1024 shell**

Do not proceed until both shell sizes pass.

---

### Task 17: Compose and Validate Graph Screens

**Files:**
- Modify: `03 Graph`, ledger

- [ ] Build `Graph / 1440×900` using the shell, 240 px Library, flexible Canvas, and 288 px Inspector.
- [ ] Screenshot Library immediately; inspect search, categories, unavailable item, and scroll affordance.
- [ ] Place Source → Gain → Pan → Sink with active cables, plus one candidate prototype route.
- [ ] Screenshot the node-and-cable cluster immediately; inspect port alignment, cable crossings, label legibility, and active/candidate distinction.
- [ ] Add Canvas Toolbar and Minimap; screenshot that local control cluster immediately.
- [ ] Populate Inspector with selected cable mapping, signal state, meter, Gain/Pan, and Parameter Feedback; screenshot Inspector immediately.
- [ ] Screenshot the complete 1440 Graph.
- [ ] Build `Graph / 1024×720` with Canvas primary and closed Library/Inspector drawers.
- [ ] Create a separate compact state with Library open; screenshot immediately.
- [ ] Create a separate compact state with Inspector open; screenshot immediately.
- [ ] Screenshot the complete compact default frame.

Pass criteria: the canvas dominates, no second toolbar appears, drawers are mutually exclusive, and runtime-ready versus prototype actions are labeled.

---

### Task 18: Compose and Validate Mixer Screens

**Files:**
- Modify: `04 Mixer`, ledger

- [ ] Build the 1440 Channel Bank header; screenshot immediately.
- [ ] Place one physical-input strip, one application strip, one stereo bus strip, and a fixed Master Monitor strip.
- [ ] Screenshot the strip bank immediately; check baseline alignment, meter consistency, control order, and strip widths.
- [ ] Add selected and pending examples without fabricating live values.
- [ ] Screenshot the complete `Mixer / 1440×900`.
- [ ] Build `Mixer / 1024×720` with fixed 176 px Master and horizontally scrolling channel area.
- [ ] Screenshot the compact strip bank immediately.
- [ ] Screenshot the complete compact Mixer.

Pass criteria: no DAW timeline or recording controls, one-channel state remains truthful, and meters show explicit unavailable states when needed.

---

### Task 19: Compose and Validate Matrix Screens

**Files:**
- Modify: `05 Matrix`, ledger

- [ ] Build the frozen Source column and Destination row for 1440; screenshot headers immediately.
- [ ] Populate a representative grid with physical, application, bus, output, and monitor groups.
- [ ] Screenshot the grid immediately; inspect cell density and state recognition.
- [ ] Add the right Inspector; screenshot immediately.
- [ ] Screenshot complete `Matrix / 1440×900`.
- [ ] Build `Matrix / 1024×720` with bottom Inspector drawer and two-axis scrolling cues.
- [ ] Screenshot compact grid immediately.
- [ ] Screenshot complete compact Matrix.

---

### Task 20: Compose and Validate Devices Screens

**Files:**
- Modify: `06 Devices`, ledger

- [ ] Build the 176 px category navigation; screenshot immediately.
- [ ] Build the device table with Active, Default, Next Start, and Unavailable rows; screenshot immediately.
- [ ] Build the 320 px detail panel with capabilities and diagnostics; screenshot immediately.
- [ ] Add a catalog-refresh error specimen that retains last-success data; screenshot immediately.
- [ ] Screenshot complete `Devices / 1440×900`.
- [ ] Build `Devices / 1024×720` with category Segmented Control and closed Detail drawer.
- [ ] Create a compact Detail-open state and screenshot immediately.
- [ ] Screenshot complete compact Devices.

---

### Task 21: Compose and Validate Settings Screens

**Files:**
- Modify: `07 Settings`, ledger

- [ ] Build the Settings full-height surface within the shell; screenshot the Settings header and close action immediately.
- [ ] Build Audio Engine section; screenshot immediately.
- [ ] Build Appearance section; screenshot immediately.
- [ ] Build Shortcuts section; screenshot immediately.
- [ ] Build Diagnostics section; screenshot immediately.
- [ ] Build the danger zone confirmation entry; screenshot immediately.
- [ ] Screenshot complete `Settings / 1440×900`.
- [ ] Build and screenshot complete `Settings / 1024×720`.

Pass criteria: no routing or device selection duplication, category hierarchy is clear, and closing Settings returns to the prior workspace conceptually.

---

### Task 22: Build Core Prototype Flows

**Files:**
- Modify: `08 Prototype`, ledger

- [ ] Create linked prototype frames for Graph → Mixer → Matrix selection synchronization.
- [ ] Create Active versus Next Start device flow.
- [ ] Create Start → Starting → Running and Stop → Stopping → Idle transport flow.
- [ ] Create parameter Pending → Accepted → Applied and Rejected flow.
- [ ] Create Settings open and close flow preserving previous workspace.
- [ ] Create compact drawer open and close flows.
- [ ] Screenshot each flow strip immediately after it is connected.

Prototype-only controls must display a visible `Prototype` badge. Runtime-ready flows display `Runtime-ready` annotations.

---

### Task 23: Add Slint Implementation Annotations

**Files:**
- Modify: all Figma component and screen pages

- [ ] Add descriptions to every Component or ComponentSet with:
  - proposed Slint component name;
  - principal properties;
  - callbacks;
  - optional slots;
  - `Runtime-ready` or `Prototype`.
- [ ] Add compact documentation cards beside major screen blocks showing ownership and data source.
- [ ] Add a component-to-Slint index frame on `01 Components`.
- [ ] Screenshot the index frame immediately and fix any clipped text.

Do not put code into the visual screen frames. Annotations belong beside showcases or in component descriptions.

---

### Task 24: Final Structural Audit

- [ ] Run read-only audits on each page, one page per `use_figma` call, sequentially.
- [ ] Verify page names, top-level frame names, sizes, instance counts, component-set variant counts, and bound-variable counts.
- [ ] Verify every screen contains editable text and component instances rather than a complete-screen raster.
- [ ] Verify every local variable has the expected scope and WEB syntax.
- [ ] Verify all semantic aliases resolve.
- [ ] Verify no placeholder shimmer remains.
- [ ] Verify no component or component set has a generic name such as `Component 1`.
- [ ] Verify all 1440 frames are exactly 1440×900 and all compact frames are exactly 1024×720.
- [ ] Verify only Graph, Mixer, Matrix, Devices appear in the peer toolbar tabs.
- [ ] Verify Settings is positioned beside window controls in both shell sizes.

Any failure blocks Task 25.

---

### Task 25: Final Visual QA at Component and Page Levels

- [ ] Review the latest passing screenshot for every component created in Tasks 5–15. If a component lacks a passing screenshot, capture and inspect it now before page QA.
- [ ] Review final 1440 and 1024 screenshots for Shell, Graph, Mixer, Matrix, Devices, and Settings.
- [ ] Test long project names, long device names, unavailable devices, no publisher, failed engine, pending route, clipped meter, and selected matrix cell.
- [ ] Check 100% zoom readability, WCAG AA body contrast, focus-ring visibility, and state differentiation without color alone.
- [ ] Check no unsupported blur or excessive effect is used.
- [ ] Check no page introduces a second full-width toolbar.
- [ ] Check the diagonal logo motif is restrained to brand and signal-flow contexts.
- [ ] Apply targeted fixes only, then capture one post-fix screenshot per affected component or page.

Expected: all component and page screenshot gates pass with no known visual defect.

---

### Task 26: Final Handoff Summary

- [ ] Update the ledger with `step: "complete"`, empty `pendingValidation`, and all page/component IDs.
- [ ] Provide the target Figma URL and summarize created pages, variables, styles, components, desktop sizes, prototype flows, and runtime/prototype annotations.
- [ ] Explicitly report any capability intentionally left prototype-only.
- [ ] Do not claim Slint implementation is complete; this phase delivers the Figma system and implementation map only.

## Completion Criteria

The Figma redesign is complete only when:

1. All nine pages exist and are organized.
2. Foundations use Tailwind-style primitive scales and semantic aliases.
3. All component families listed in Tasks 5–15 exist and each has a passing screenshot taken immediately after creation or targeted repair.
4. Shell, Graph, Mixer, Matrix, Devices, and Settings exist at both 1440×900 and 1024×720.
5. Each meaningful page block was visually checked before later blocks were built.
6. Settings is outside the four peer workspaces and sits beside window controls.
7. Unknown live data is never represented by fake zero values.
8. Final structural and visual audits pass.
9. The design remains editable and uses reusable components and variable bindings.
10. No previous Moiren design content was inspected beyond the explicitly permitted logo node.
