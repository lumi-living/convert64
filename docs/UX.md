# Convert64 — UX foundations

**Status:** v0.2 (2026-10-11). Design starting point for E8/E12. v0.2: the C64 result lives permanently at the top of the right dock; the main canvas is a working view that defaults to the source and flips to the result for inspection. Decisions here are conventions to follow until a usability test says otherwise; anything marked *open* is a question for the first build.

Sources: a survey of Photoshop, Affinity Photo 2, PhotoCraft, GIMP 3, Krita 5, Aseprite, Pixelmator Pro and Photopea; of C64/retro converters (retropixels+, png2c64, ArcPaint64, C64CC, ALBERT, Multipaint, Disco Volante, convertron3000, retropic) and dithering tools (Ditherista, Dither Boy, Dither Machine, Photoshop/GIMP/Aseprite indexed-colour dialogs); and egui 0.36 capabilities. Key references are listed at the end.

---

## 1. Who it is for and what they do

| User | Comes with | Wants to leave with | Session shape |
|---|---|---|---|
| **Game developer** (first user: *No Glory*) | painted or AI-generated concept art, a Makefile | a legal `.kla`/charset/sprite file that builds into the game, reproducibly | 10–30 min per asset, revisits the same project many times |
| **Scene graphician** | a reference image and strong opinions about dithering | a conversion good enough to finish by hand in Pixcen/Multipaint/ArcPaint64 | 1–2 h, iterative, keyboard-heavy |
| **Demo coder** | any picture | bytes in assembler-ready form, fast | 5 min, CLI-first, GUI for checking |

The core loop is the same for all three: **open → see the C64 result immediately → steer (layers, recipes, pins) → check the colour-limited cells → export → come back later and re-run**. The UI exists to make "steer" precise and "check" instant. It is not a paint program.

## 2. Design principles

1. **The C64 result is always visible.** A 320×200 result at 1× is small on any modern display, so it does not fight for the main canvas: it lives at the top of the right dock, always on, in hardware colours, never themed, and updates with every change. The main canvas is where the user *works* (on the 16-bit source) and *inspects* (the result, zoomed, compared).
2. **Every change is live; nothing is modal.** No "Convert" button, no apply dialogs. Dragging a slider re-solves (fast path while dragging, full search on release). The one exception is a *pause* toggle for slow modes.
3. **Legality is guaranteed; colour limits are visible, quantified and navigable.** The colour-limited cell count in the status bar at all times, in neutral text because the result is legal by construction; a heatmap overlay on demand; jump-to-worst-cell. Red is kept for what blocks export (a failed export, a file that cannot be read), so users never discover illegality at export and never mistake a diagnostic for an error (#243).
4. **Steering survives re-solving.** Pins, locks and region overrides are part of the project, not edits of the output. Re-run from scratch and they still apply.
5. **Borrow every convention that has one.** Photoshop-family layout, shortcuts and layer-row anatomy. Invent only where no tool has gone: per-layer recipes, cell steering, colour-limit UX.
6. **Keyboard-first, mouse-complete.** Everything has a shortcut; nothing *needs* one.
7. **Same words everywhere.** One glossary (§9) used in UI, CLI flags, MCP tool names and docs.

## 3. Layout

Consensus across the surveyed editors: menu bar → options/tool bar → canvas in the centre → a right dock with a navigator on top, properties in the middle, layers below → status bar. Convert64 follows it, with the C64 result taking the navigator's place and no left tool palette (a converter has four tools, which fit in the toolbar).

```
┌────────────────────────────────────────────────────────────────┐
│ Menu bar  File  Edit  View  Layer  Recipe  Export  Help        │ 22 px
├────────────────────────────────────────────────────────────────┤
│ 📌 🔒 🖌 ✥ │ ⊕ ⊖ ⛶ │ ⇄ ◫          Working view: Source (Q → Result · hold \ to flip) │ 28 px
├──────────────────────────────────────────┬─────────────────────┤
│                                          │ Result  cells: 3 ◫ ⛶ │ 26 px header
│                                          │ Multicolor▾ Pepto▾ ■▾│ 22 px picture row
│                                          │ ┌─────────────────┐ │ 320×200 at 1×
│          WORKING VIEW                    │ │   C64 result    │ │ + border,
│                                          │ │  (hardware px)  │ │ always on
│   Source (16-bit, layers, masks)         │ └─────────────────┘ │
│   or Result (zoomed, overlays, compare)  ├─────────────────────┤
│                                          │ Recipe Layers Export│ right dock
│                                          │  (selected tab)     │ 320 px
│                                          │                     │ (260–420)
├──────────────────────────────────────────┴─────────────────────┤
│ Status  Multicolor 160×200 · Source · cell 12,7 · px 48,56 · colours ■■■■ · error 0.282 · 400%   Colour-limited cells: 3 · solve 42 ms │ 22 px
└────────────────────────────────────────────────────────────────┘
```

Heights and the dock width are the Figma `convert64/*` layout variables (`theme::layout`); the Figma `App Shell / Light` and `/ Dark` frames are the reference.

- **Right dock: the result, then tabs.** **Result** (its header, the picture row with the picture-wide settings, and the C64 result at 1×; fixed height, never scrolls away), then the **Recipe**, **Layers** and **Export** tabs (Figma `Dock Tabs`; the Recipe tab also lists the layers, so the common case needs no tab switch). Default width 320 px = one C64 result at 1×; resizable 260–420, and the result scales down (never up) if the dock is narrower than 320. `Cmd \` hides all chrome (Figma's Show/Hide UI) and `Shift Cmd \` only the dock; when the dock is hidden the result is gone too, which is the point — full-screen working. Hiding the UI shows a toast, "UI hidden — press Cmd \ or Esc to show it", and `Esc` always shows it again. `Tab` is left to focus traversal (§12); until #240 it hid the chrome (Photoshop's convention), which trapped new users with no way back. (Until #203 this section stacked Recipe and Layers with a draggable divider and collapsed them to tabs below ~700 px; the Figma frames chose tabs at every width.)
- **Working view** gets all remaining space and has two modes, **Source** and **Result**, switched with `Q` or the toolbar's Source/Result button. Source is the default on open. The status bar always names the current mode.
- **The result panel is also a navigator:** when the working view is in Result mode and zoomed, the panel shows the visible rectangle; clicking or dragging on the panel pans the working view there (Photoshop Navigator behaviour). In Source mode a click on the panel switches the working view to Result and centres on that spot.
- **No docking framework in v1.** Fixed `egui::Panel`s with persisted sizes; egui_tiles only if users ask to rearrange.

## 4. The working view and the result panel

### 4.1 Source mode (default)

The 16-bit composite of all layers at any zoom, with the C64 cell grid projected onto it (supersample-aware, so a cell is a 32×32 or 16×32 source-pixel region at 4×). This is where the user nudges, scales, paints masks and regions, and sets pins — cells are addressed by C64 coordinates in both modes, so a pin set here is the same pin seen in Result mode.

| Aspect | Decision |
|---|---|
| Zoom | fractional allowed (it is a source image): 25 %–800 %, `Cmd/Ctrl +/−`, `Cmd/Ctrl 0` fit (leaving 5 % of the view free around the picture, as Figma does, so the transform handles stay clear of the edge), `Cmd/Ctrl 1` 100 %, wheel around the cursor |
| Aspect | square source pixels; the projected cell grid shows the 2:1 stretch of multicolor modes as 4×8-equivalent rectangles |
| Overlays | **Cell grid** `Cmd/Ctrl '`, **Pins & locks**, **Regions**, **Colour-limited cells** `Shift+C` (the heatmap projected onto source cells), **Overlay the result** (the result at 30 % over the source, *View › Overlay the Result*; the same toggle as Result mode's *Overlay the source*; usability review m5) |
| Selection | the selected layer's bounds are outlined; its transform box, handles and anchor are drawn in Source mode only. In Result mode (or while `\` shows the result) the Transform tool draws nothing and its handles do not catch the pointer: a click still selects the layer under it, as Nudge does (usability review m3) |
| While dragging | a transform drag (nudge, scale, rotate, held arrow keys) moves the layer's cached picture, at full detail, until release (a translucent layer, a blend mode or a mask over other layers: as a slider drag); a slider drag shows the picture at the solve's resolution (1×, square-edged pixels) until release. On release the full-detail view returns. The full-detail view takes about 60 ms per frame to make in the browser; this takes a few (#264) |

### 4.2 Result mode

The solved C64 image, for inspection and comparison.

| Aspect | Decision | Why |
|---|---|---|
| Zoom | Integer only: 1×, 2×, 3×, 4×, 6×, 8×; same keys as above; `Cmd/Ctrl 0` fits to the largest integer zoom, and the view opens at it: the picture with at least half the border (16 px) on each side when the border is drawn, with the Source margin when it is not | nearest-neighbour texture; fractional zoom blurs pixels |
| Aspect | 2:1 pixels in multicolor modes rendered as 2 device px per C64 px at 1×; optional "PAL 0.936" correction toggle, off by default | what the solver optimised, then what a TV would show |
| Pan | `Space`+drag, middle-drag, scroll, or the result panel as navigator | universal |
| Border | 32 px-equivalent border in the chosen border colour, toggle `View › Border` | judging edge colours against the frame is part of the craft |
| Overlays | **Grid**, **Colour-limited cells** `Shift+C` (heatmap: per-cell error, colour-limited cells outlined; off by default), **Pins & locks**, **Attribute map** (debug: which recipe governs each pixel), **Overlay the source** (the source at 30 % over the result, *View › Overlay the Source*) | painter calls over the same rect; overlays in semantic theme colours |
| Compare | **Hold `\`** shows the source in place (and in Source mode, hold `\` shows the result — the key always flips to the other one); **Split** draggable divider; **A/B flicker** at 2 Hz; **Side by side** source \| result with linked cursor; `,` cycles | three habits from three editor families; cheap to offer all |
| Cursor readout | status bar: cursor cell x,y; the cell's colour set; its error; its pin/lock state | the "why is this cell like that" question answered without a dialog |

### 4.3 The result panel (right dock, top)

- Always visible while the dock is visible; 320×200 at 1× (2:1 pixels already applied), with the border, inside a frame that reads in both themes. It never scales up; it scales down only if the dock is narrower than 320.
- Header row (26 px): **Result**, the method picker's `Preview: Atkinson` while it previews (§6a), and on the right the colour-limited cell count, `Colour-limited cells: 367`, then Compare and Fit. The right-hand part is laid out first and the preview label takes only the space left between it and **Result**, cut short with `…` (its tooltip has the full name), so it never draws over the count (found in the release-prep run: with the picker open and the pointer on a card, `Preview: …` was drawn on top of the count). The count is in normal text, not red: a colour-limited cell is one where the solver wanted more colours than the mode allows and used the closest it may, so the result is still legal and the count is a diagnostic, not a warning (usability review M1, #243). Its tooltip says so. The count is a button that turns the Colour-limited cells overlay on and switches the working view to Result at the worst cell.
- Picture row (22 px, under the header; usability review M5, #288): the picture-wide settings, which apply to every layer and so live with the result, not in a layer's Recipe. Three compact dropdowns, left to right, each with its name in the tooltip:
  - **Mode** (*Multicolor · Hi-res*; charset modes when they solve). This is the only Mode control: the Recipe tab no longer has one.
  - **Palette** (*Pepto PAL · Pepto NTSC · Colodore*, then *VICE palette file (.vpl)…*), showing the palette's name.
  - **Background**: a 12 px swatch of the background and its name, `Auto` while the solver picks it. The swatch is the solver's pick under Auto (ringed in the text colour) or the pinned colour (ringed in the accent colour), as in the grid. The dropdown holds *Auto (the solver picks)* and the 16 colours as a 2 × 8 grid of 16 px swatches, ringed the same way; a click on a swatch pins it. It stays open until a click outside, so a colour can be tried and another picked.
- Overlays on the panel: dots on the colour-limited cells (not the full heatmap), only while that overlay is on (`Shift C`, *View › Colour-Limited Cells*; off by default, so the main output is clean), and the navigator rectangle in Result mode.
- **Short windows** (usability review m8, §14 Q1): in a window under 800 px tall the result collapses to a strip, the picture at half size (160 × 100, 2:1 pixels kept) with 6 px of border above and below, 112 px in all instead of 200, so the tabs below get 88 px more. The header and picture row stay. The strip is still the navigator; at 800 px and taller the full 1× panel is back. Nothing to set: it follows the window.
- Mode switching re-solves immediately; the panel desaturates slightly while a full search runs (>200 ms) and shows `Solving…` in place of the timing.
- The working view and the panel are the same texture at two sizes; there is no second render.

The result is **never themed**: hardware palette on a border, overlays in semantic colours chosen per theme for contrast against any of the 16 palette colours (test: readable over Black, White, Yellow and Blue).

## 5. Layers panel

Row anatomy, left to right (editor consensus): **drag handle** · **eye** · **thumbnail** (source) · mask chain and thumbnail when there is a mask · **name** over a one-line **subtitle**. The row is two text lines tall. Nothing sits right of the name column, so nothing is clipped at the dock's narrowest (usability review m7): the old recipe badge and opacity column are the subtitle now. A **lock** per layer arrives with layer locking (not built; cells have Lock, §7).

- Subtitle shows the recipe in short form: `FS 80% · OKLab · Force`, then `· 50% opacity` when the layer is not fully opaque. Its tooltip gives the method's CLI id (`floyd-steinberg 80% · OKLab · Force`). Secondary text (§10), in body text on the selected row's highlight. This is the one departure from editors, and it is the point of the product: you see at a glance how each layer will be dithered.
- Names: a picture added as a layer is named by its file stem, as an opened picture is (`source`, never `source.png`); a name already in the stack gets the next free number: `source 2`, `source 3` (usability review m7).
- Blend mode and opacity controls sit above the list (Photoshop/Krita/Photopea). Mask thumbnail appears beside the layer thumbnail with a chain icon when a mask exists.
- Bottom icon strip: **New from file**, **Duplicate**, **Add mask**, **Group** (v2), **Delete**, each with a tooltip naming it and its key (`New layer from file (Shift Cmd N)`, `Delete layer (Delete)`); a disabled one says why. Drag to reorder (`egui_dnd`), drop a file onto the panel or the preview to add a layer, double-click to rename, `Cmd/Ctrl J` duplicate, `Delete`/`Backspace` delete with undo, but only while the Layers panel or the canvas has the keys (after a press on either, or `Esc`), never from the Recipe form; every delete shows a toast naming the layer with an **Undo** button: "Deleted layer 'source' · Undo" (#245).
- Selecting a layer selects what the Recipe panel edits and what nudging moves. Multi-select applies recipe changes to all (v2).
- Empty state (no layers): one sentence and one button — *"Drop an image here or open one. Convert64 shows the C64 result as soon as it loads."* **Open image…**

### 5a. Splitting an image into layers

Users split an image because one area needs different treatment. Which tool depends on *what* differs:

| What differs for the area | Use | Why |
|---|---|---|
| Only the recipe — dither, metric, allowed colours, Force/Adaptive | **Region** (`R`, §7) | no new layer; a region is a mask plus a recipe painted over the composite |
| Also adjustments, transform/nudge, blend mode or opacity | **Layer via mask** | the area needs its own layer state |
| A different source image | **New from file** + mask | — |

**Layer via mask** is the counterpart of Photoshop's *Layer via copy / via cut* that fits ADR-003: no pixels are copied. The new layer references the same source pixels as the original and gets the current selection as its mask; with the *cut* variant the original receives the inverted mask. A selection is made with the mask tools (§7a: lasso `W`, key `K`, gradient `G`, snap to cells) while no mask is being edited — the same tools, the same red overlay. `Cmd J` with a selection active = layer via mask; `Shift Cmd J` = via cut; without a selection `Cmd J` is plain Duplicate. The new layer is named "<original> (mask)" and selected, with its mask thumbnail showing the selection. Because the compositor blends feathered alpha and the attribute map takes the top-most layer, overlapping feathered edges between the two layers behave as one would expect, and a Region can still be painted on top for a recipe-only tweak inside the new layer.

Bottom icon strip gains **Layer via mask** between Duplicate and Add mask; it is disabled until a selection exists, with a tooltip saying so.

## 6. Recipe & adjustments panel (the form)

This is the only real form in the app, so it follows form conventions strictly: visible labels, slider + numeric field on one row, per-group **Reset**, no modal apply.

The tab edits the selected layer only; the picture-wide Mode, Palette and Background are in the result's picture row (§4.3). It starts with the **layer header**: the layer's 32 px thumbnail, its name in the heading style and a subtitle, `Layer 1 of 2 · 1024 × 768 px` (the source's size), so it is clear which layer the groups below belong to (usability review M5).

Collapsible groups follow, ordered by use, the setting this product is about first (usability review M5, #288): **Dither recipe → Colour fitting → Adjustments → Sharpen → Transform → Mask → Advanced**. Sharpen stays its own group right after Adjustments (#237). Transform, Mask and Advanced start collapsed, the others open. A group opened or closed stays so: the choice is the user's, kept per group in the app's own storage (eframe's storage: a file natively, `localStorage` in the browser), not in the project, and it holds for every layer and for region recipes. The numbers below are the groups' reference numbers, not their order on screen.

1. **Transform** (ADR-011; usability review M6, #289) — in **screen pixels**, the 320 × 200 grid every mode is composited on: one screen pixel is one hi-res pixel, and a multicolor pixel is 2 screen pixels wide (the compositor applies multicolor's 2:1, never the user). Rows, top to bottom:
   - **X, Y** — the top-left corner of the unrotated box, `DragValue` in px, 0.01 px step.
   - **W × H** — the layer's size on the screen in px (`320.00 × 199.06`), with a chain toggle between them (uniform by default: the chain keeps H/W as it is), 0.01 px step. Under them, weak, the secondary readout: the scale as a % of the source, `31.25 % × 31.25 % of the source`, and in multicolor the width in multicolor pixels, `· 160.0 multicolor px wide`.
   - **Rotate** in degrees, 0.01° step, −180…180.
   - **Anchor** — a 3 × 3 control of 16 px cells in one frame (pivot for scale and rotate, default centre): the anchor's cell filled in the accent colour, the others a dot, hover lighter; each cell's tooltip names it ("Pivot: top left").
   - **Presets** — four visible segmented buttons, *Fit · Fill · 1:1 · 2:1*, the one the transform matches shown selected. Each keeps rotation, anchor and flips:
     - *Fit*: uniform, the whole picture on the screen: scale `min(320 / w, 200 / h)` on both axes, centred.
     - *Fill*: uniform, the screen covered, the overflow cropped evenly: scale `max(320 / w, 200 / h)`, centred.
     - *1:1*: one source pixel per screen pixel, scale (1, 1): a hi-res pixel each, or half a multicolor pixel (for a 320-wide multicolor screenshot whose pixels are doubled). Centred, then moved left and up onto whole C64 pixels of the mode (multicolor: an even x), so pixel art stays crisp.
     - *2:1*: one source pixel per multicolor pixel, scale (2, 1): each source pixel 2 screen px wide and 1 tall, for pixel art drawn at 160 × 200. The same transform in hi-res, where it stretches such a picture to the screen. Placed as 1:1.
   - **Flip** — two icon toggles, *Flip horizontal* and *Flip vertical* (the Material `flip` glyph, the second turned a quarter), selected when on.
   - **Snap** *Off · C64 pixel · Cell* (default **Off** — everything is sub-pixel because the source is 16-bit and fractional placement is what steers the solver; snapping is for pixel-art sources and logos that must sit on the grid); *Downscale: Auto · Exact* (#235) and *Resample: Mitchell · Lanczos* (ADR-014 §3).
   Arrow keys nudge 1 px, `Shift+arrow` 10 px, `Alt+arrow` 0.25 px (the sub-pixel nudge is the feature; it gets the modifier that no editor uses). Position, scale and rotation are continuous unless Snap is on.
   A picture opened in the app (its first layer) opens with **Fill**: uniform, the screen covered with nothing distorted, the group shows Fill selected. A layer added on top opens fitted inside the screen and never enlarged (uniform, top left). `convert64 convert picture.png` keeps png2c64's default, the picture stretched to 320 × 200 (the reference's front end, ADR-010); a project stores its transform, so opening one, old or new (#251's migration included), changes nothing.
2. **Adjustments** — Brightness, Contrast, Saturation, Hue as sliders −100…100 with numeric entry; **Black point** 0–50 % (png2c64's `--black-point`, ADR-014); Levels (black/gamma/white) and Curves (own widget over a histogram); **Range** (*Match the palette's range*, applied last), its own row after Curves, with *Method: Box · Hull (experimental)* and the hull's Amount under it. Then **Sharpen** as its own collapsible group right after Adjustments, with its own changed-value dot and Reset (#237) — Amount −100…300 % (negative softens, for noisy photos), Radius as a segment *png2c64 · Gaussian* (png2c64's 3×3 kernel is the default) with the Gaussian's σ (0.3–3.0 C64 px) in a field beside the segment when Gaussian is chosen, Threshold 0–10 % of lightness (tooltip: "Skip changes smaller than this, so noise isn't sharpened") — on lightness only, at the C64 grid after the layer is scaled, so what you see on the Source view is exactly what the solver gets. Reset per widget via double-click on the label (PhotoCraft-style scrubby label) and a group Reset button.
3. **Mask** — see §7a. Feather radius (target px, slider 0–32), Invert, Levels (black/white/gamma), Snap to cells (*Off / Expand / Contract / Nearest*), a **Sources** list (painted strokes, gradients, keys — each with its combine mode and a delete), and **Edit mask** (enters the mask tools, §7a). A layer without a mask shows one button: **Add mask** (from PNG alpha if the layer has any, otherwise full white).
4. **Dither recipe** — **Method** (the picker in §6a), Strength 0–100 % slider, Pattern phase (*Screen · Layer*: locked to the screen or to the layer, each segment's tooltip says which; the longer *Screen-locked · Layer-locked* wrapped in a dock under about 280 px, usability review m6), Orientation (*Auto · Vertical · 2:1*). Every segmented row fits on one line in the narrowest dock (260 px). Methods are grouped by family, following Ditherista's lesson that a flat list of 60 names is unusable. Every method is shown as a picture, because a dither is judged by eye, not by name.
5. **Colour fitting** — Metric (*OKLab · CIELAB · Pepto YUV · RGB*), Fitting (*Adaptive / Force*, segmented, with one-line help: "Adaptive lets the solver reassign cell colours for the lowest error. Force keeps this layer's colours even if neighbours pay."), Luma-pair bias toggle, **Allowed colours**: the 16 palette swatches as toggles (all on by default; a disabled swatch is drawn hatched, not hidden).

Controls: `Slider` for bounded values with the value field editable on click; `DragValue` for integers and unbounded numbers. **Shift-drag** on either moves in fine steps, a tenth of the plain drag (Figma's and Photoshop's convention; usability review m9: Brightness moved about 1.5 units a pixel); releasing Shift mid-drag goes back to the plain rate, and a slider then follows the pointer again. The sliders' tooltips say so (number fields have egui's own Shift behaviour); a changed-from-default value shows a small dot before its label (Lightroom/Affinity convention) so the user can see what they touched. Changes commit to undo on drag end, never per frame.

## 6a. Method picker

The Method row is a button: a 32 px thumbnail of the current method, its name and ▾ (Figma `Method Field`), with ▲▼ step buttons beside it (png2amiga's dither stepper). Clicking it, or pressing `Enter` while it has focus, opens the picker (Figma `Method Picker`). The picker is a popover anchored to the dock's left edge, opening over the working view so the result panel stays visible. It is 4 columns wide (about 680 px) and at most the window height minus 120 px, scrolling inside. The same picker serves layer recipes and region recipes.

**Contents, top to bottom**

1. A search field, focused on open. It filters by name, CLI id and aliases (`fs`, `checker`, `png2c64 line2`). Typing anywhere in the picker types into it.
2. **Preview on:** *Gradient · Picture* (segmented, remembered).
   - *Gradient* is a diagonal ramp through the current palette's greys plus one hue. The ramp carries ±2/255 seeded jitter, because without it error-diffusion kernels lock into regular patterns that look ordered (png2amiga's lesson).
   - *Picture* is a 32 × 32 C64-pixel crop of the selected layer, taken at the last cell the cursor was over (or the picture's centre), so methods are compared on the user's own image.
3. **Recent** — the last four methods used in this project.
4. Families in fixed order, each under a small uppercase heading: **None · Ordered · Ordered noise · Error diffusion · Palette-aware · Effects**. All 90 methods (`docs/reference/dither-methods.md`): png2c64's 30, Convert64's own 28 and png2amiga's 32, each family's in that file's order.

**Card (Figma `Method Card`)**
- A 64 px square thumbnail: 32 × 32 C64 pixels at 2×, nearest-neighbour, with 2:1 pixels in multicolor.
- The name to its right, wrapping to two lines.
- A *slow* badge for methods whose full solve exceeds 200 ms (DBS, Yliluoma 1).
- States:
  - Default.
  - Hover: a lighter fill.
  - Selected: an accent outline and a tinted fill (the current method).
  - Focused: a focus ring.
  - Rendering: a neutral placeholder, never a spinner.
- The tooltip gives the one-line description and the CLI id from `docs/reference/dither-methods.md`. For png2amiga's 32 methods (#275) it adds where the method comes from (*From png2amiga, on png2c64's pipeline; the png2amiga solver runs png2amiga's own.*) and png2amiga's own name where it differs (`--dither bayer3 (png2amiga: bayer3x3)`); `gilbert` and `dbs` say that on C64 png2amiga runs them as a nearest pick.

**Thumbnails are real solves.** Each one is rendered by the solver with the recipe's current mode, palette, strength, phase, metric and allowed colours, so it shows what this layer would get.
- They render in the background, visible cards first, and can be cancelled. In the browser build they are rendered in small batches, one batch per frame.
- They are cached by method, settings and source tile. Changing strength re-renders them; reopening the picker does not.

**Live preview** (the font-menu convention of Figma and Photoshop):
- Resting on a card, or arrowing to it, for 150 ms previews that method on the whole result with an Interactive solve. The result header shows `Preview: Atkinson`.
- Moving off the card reverts the preview.
- A click or `Enter` commits the method as one undo step and closes the picker.
- `Esc` or a click outside reverts and closes.

**Keyboard**
- The arrows move across the grid.
- `Home` / `End` jump to the first and last card.
- With the closed Method button focused, `↑` / `↓` step through methods in picker order with live preview (Photoshop's blend-mode stepping).

**Accessibility:** every card is a focusable widget whose label reads "Atkinson, error diffusion, selected".

## 6b. Export panel

The Export tab is the export panel (usability review M4, #287), after Figma's Export section in the properties panel: a list of rows, **+** to add one, and one **Export** button that writes them all. There is no export dialog. The rows are the project's **export set**: saved in the project file (`exports`, [project-file.md](reference/project-file.md#export-set)), changed with the `export_set` command (one undo step per change), and written the same way by `Shift Cmd E`, `convert64 export` without `--asm` and the MCP server's `export_run` (ADR-005), so the Makefile case has a visible definition.

```
┌ Export ─────────────────────────────────────┐
│ [Koala (.kla)         ▾]                  × │  row: format · options · remove
│ [title         ]  title.kla · 10 kB         │       file name · preview with size
│ [KickAssembler (.asm) ▾] [Include raw set ▾]× │
│ [title         ]  title.asm + 4 files · 21 kB│
│ [PNG (.png)           ▾] [2× ▾]           × │
│ [title@2x      ]  title@2x.png · 4.1 kB     │
│ + Add format                                │
│ ┌─────────────────────────────────────────┐ │
│ │            Export 3 formats             │ │  primary, accent fill
│ └─────────────────────────────────────────┘ │
│ To a folder you pick. Export Again ⇧⌘E      │  weak
└─────────────────────────────────────────────┘
```

**A row** is two lines:
1. The **format** dropdown, the format's **options**, and **×** to remove the row.
   - The dropdown lists the formats in three groups, each under a small heading:
     - *C64 files*: Koala `.kla` (Art Studio `.art` in hi-res), PRG (self-displaying), Raw bitmap set (`.bin` blocks and a JSON manifest).
     - *Source code*: KickAssembler `.asm`, ACME `.a`, 64tass `.tas`, ca65 `.s`, C header `.h`.
     - *Preview*: PNG.
   - Each entry names its extension. Its tooltip says what the file is for ("A self-displaying program: LOAD and RUN it").
   - Options: PNG has a zoom, 1×–8× (default 2×). Source code has *Inline bytes* (one file, the default) · *Include raw set* · *Include picture file*. The C64 files have none.
2. The **file name** and what the row writes.
   - The name field's placeholder is the project's name (`title`). An empty field means that name, so renaming the project renames the files. The name is committed on `Enter` or when the field loses focus. A name with a folder in it or a leading dot is refused with a toast, and the field keeps the old name.
   - Beside it, in weak text, the first file and its size: `title.kla · 10 kB`. A row that writes several files says so, `title.asm + 4 files · 21 kB`, and its tooltip lists each file with its size. Sizes are of the result on screen, in §9's human units. While the first full solve runs the size reads `…`.
   - Two rows that would write different bytes to one file name (two PNGs at different zooms, both named `title`) mark the second row in the `error` colour: `Same file as row 1: give one another name`. Export is disabled until that is fixed. Rows that share an included file (two assemblers including the raw set) write it once.

**+ Add format** adds a row with the next format the set lacks, in this order: the picture file, PRG, PNG, KickAssembler, then the rest. With no rows the panel shows one sentence and the button: *"Choose what to export. The set is saved with the project."* **+ Add format**.

**Export** is the panel's one primary button (accent fill, like Figma's), full width under the rows. It reads `Export 1 format` / `Export 3 formats` (`Export` with no rows), and is disabled with no rows, while the first full solve runs, or while two rows clash. It writes every file:
- Natively, into a folder the user picks.
- In the browser, through #241's direct save path: into a folder picked with `showDirectoryPicker` where the browser has it (Chromium), otherwise one download (`<a download>`) per file. There is no overlay.

The toast follows once the files are written (§9). Under the button, in weak text: `To a folder you pick. Export Again ⇧⌘E`, and after an export `Last: build/ · Export Again ⇧⌘E` (the folder's name).

**Export Again** (`Shift Cmd E`, *Export › Export Again*) writes the set again, into the same folder, without asking. In the browser that is the same directory handle while the page is open; after a reload, or where the browser has no folder picker, it asks or downloads as Export does. With no folder yet it asks, as Export does. With an empty set it shows the Export tab instead. `Cmd E` (*Export › Export…*) shows the Export tab: the dock and the UI are shown if they were hidden.

## 7. Cell tools (steering)

Three tools in the toolbar, all operate on the preview, all additive to the project, all undoable. **Nudge** (`V`, Move in Photoshop and Figma) is the default, on first run and for every picture opened, so a stray click selects a layer and changes nothing; Pin, Lock and Region act only once picked, and a click outside the picture (on the border or the pasteboard) never pins, locks or paints (#242). Every drag, with these tools, the mask tools (§7a) and the Hand, counts from where the button went down, not from where the pointer passed egui's 6 pt drag threshold: a nudge moves by the whole drag, and a rectangle, stroke or gradient starts at the press (#263):

| Tool | Key | Click | Drag | Feedback |
|---|---|---|---|---|
| **Pin** colour set | `P` | pin the cell's current colour set; `Alt`-click to pin one colour only (from the status-bar swatch or the palette strip) | pin a rectangle of cells | pinned cells get a small pin glyph at top-right; status bar: "Pinned 7 cells" |
| **Lock** cell | `L` | freeze the cell's output exactly as solved | lock a rectangle | lock glyph; locked cells are skipped by re-solve and drawn with a faint hatch when the Colour-limited cells overlay is on |
| **Region** override | `R` | — | paint a soft mask that carries its own recipe (dither/metric/allowed colours); the first stroke of a new or empty region always adds (below) | the regions show tinted, each outlined in the `region` colour (the selected one thicker) from the moment it exists; its recipe editor opens in the Recipe panel as a pseudo-layer under a breadcrumb (below): Dither recipe, then Colour fitting, in the layer form's order and open or closed as there (§6) |
| **Nudge** (move) | `V` | select layer under cursor | move the layer in sub-pixel steps | X/Y readout in status bar; the solver re-runs on drop |
| **Transform** (scale / rotate) | `S`, or `Cmd T` natively (§8) | select layer; shows its (rotated) bounding box with 8 handles and the anchor, in Source mode only (§4.1) | corner = uniform scale (hold `Shift` for free aspect), edge = one axis, just outside a corner = rotate (hold `Shift` for 15° steps), inside = move, `Alt` = about the centre, drag the anchor to move the pivot; scroll wheel over the box = ±1 %; hold `Cmd` to invert the current Snap for this drag | live X/Y, W × H in screen px and the angle in the status bar, all sub-pixel; Interactive solve during drag, Full on release; `Enter` commits, `Esc` reverts the whole drag (one undo step) |

**Tool options are kept per tool** (usability review M7, #290). The Region brush, the mask Brush and the Lasso (§7a) each keep their own Add/Subtract, and the two brushes their own Size, Hardness and Flow; `X`, `[` and `]` change the current tool's only. Picking a tool never carries another tool's options over: the Region brush used to share the mask brush's, so after an `X` there its first stroke was a Subtract on an empty region and showed nothing.

**The Region flow** (usability review M7, #290):
- *Options bar:* `Region · Brush`, then Add · Subtract, Size, Hardness, Flow: the Region brush's own options.
- *The first stroke of a new region always adds*, whatever Add/Subtract says, so it always shows. That is the stroke that makes Region 1 when there is none, and the first one on an empty region (one made with **Add region**). The bar goes back to Add with it, so it shows what the stroke did.
- *Outline:* each region is outlined in the `region` colour, along the edge where its mask crosses 50 %, over its tint; the selected region at 2 px, the others at 1 px. While a stroke is painted, its swath shows in the `region` colour before the release. A region with nothing painted yet has no edge to draw, so the options bar says *Region 1 is empty: paint on the picture to add to it* instead.
- *Breadcrumb:* the Recipe tab opens with **Region 1 ‹ source**: the region in the heading style, then `‹` and the selected layer's name as a link. A click on the layer's name goes back to that layer's recipe and to the Nudge tool (`V`), the way out the review missed (its tooltip: *Back to source's recipe (V)*). With no region yet it reads **Regions ‹ source**. The region list, **Add region** and the recipe follow.

Right-click on any cell: a small menu — *Pin colours · Lock · Clear pin/lock · Jump to worst neighbour · Copy cell colours*. `Cmd/Ctrl Shift ]` jumps to the next-worst cell, `[` to the previous.

## 7a. Mask tools (ADR-008)

Masks are edited inside Convert64 because the edge is tuned against the cell grid: move it, change the feather, watch the Colour-limited cells overlay. A mask is one raster in layer space plus non-destructive modifiers; every tool below is a command with undo. The brush is the only freehand tool in the app and it paints weight, never colour (ADR-003).

Entering mask mode (`M`, or **Edit mask** in the Recipe panel, or clicking the mask thumbnail) switches the toolbar to the mask tools and shows the mask as a **red quick-mask overlay** on the working view (Photoshop convention; red is distinct from every overlay colour in §10 and from the C64 palette's two reds at 50 % opacity). `Alt`-click the mask thumbnail shows the mask alone as greyscale. `Esc` or `M` leaves mask mode.

| Tool | Key | Click | Drag | Options bar |
|---|---|---|---|---|
| **Brush** | `B` | dab | paint `Add`; hold `Alt` for `Subtract` | Size (`[` / `]`), Hardness, Flow; `X` swaps Add/Subtract |
| **Lasso / polygon** | `W` | add a vertex; double-click or `Enter` to close | freehand lasso | `Add` / `Subtract`, Fill value |
| **Gradient** | `G` | — | drag from→to; `Shift` constrains to 45° | Linear / Radial, Softness, combine `Max · Min · Multiply` |
| **Key** | `K` | sample the layer pixel under the cursor | drag to widen the range | Luma / Chroma / Colour, Range, Softness, combine |
| **Snap to cells** | toolbar toggle | — | — | Off / Expand / Contract / Nearest; previews the snapped edge in the overlay |
| **Feather** | slider in options bar | — | — | 0–32 target px; applies after snap |
| **Invert / Clear / Fill / Load PNG / From alpha** | options-bar buttons | — | — | — |

Feedback: while a brush stroke is in progress only the quick-mask overlay updates; the solver re-runs on mouse-up for the cells under the stroke's bounding box and the status bar says "Re-solved 14 cells". A colour-limited cell that sits under a feathered edge gets a one-line hint: *"Edge straddles cell 12,7 — try Snap to cells or a smaller feather."*

Mask thumbnails in the Layers panel (§5) show the raster with modifiers applied; the chain icon between layer and mask thumbnails is informational in v1 (masks always move with the layer; unlinking is v2).

## 8. Keyboard shortcuts

`Cmd` on macOS, `Ctrl` elsewhere; `Option`↔`Alt`. Menu items and tooltips show the accelerator via `shortcuts::format`: egui's modifier names or symbols with the key as its symbol, never its name (`Ctrl+=`, `Ctrl+−`, `Ctrl+'`, `⌘=`; not `Ctrl+Equals`; usability review m5).

**Menus** (usability review m5). An on/off item has a check mark when on and the same space blank when off, so a toggle never looks like a hovered item: *View › Source / Result*, *Compare ›*, the overlays, *Border*, *Theme ›*. An item that shows or hides says which it will do: *Hide Dock* / *Show Dock*, *Layer › Hide Layer* / *Show Layer*. **Edit** has Undo, Redo, then **Copy Recipe** and **Paste Recipe**; the **Recipe** menu has Copy Recipe, Paste Recipe and **Reset Recipe**. Copy takes the selected layer's whole recipe (dither and colour fitting, the solver settings included) into the app's own clipboard; Paste sets all of it on the selected layer as one `recipe_set` (one undo step) and is disabled until something was copied; Reset sets the defaults the same way. *View › Overlay the Source* (Result mode) / *Overlay the Result* (Source mode) is the ghost overlay of §4. Single-key shortcuts are suppressed only while text is being typed (a text field, or a number field after a click on it); a slider or button with focus leaves them working, though arrows then belong to that control. `Enter` and the end of a drag hand the keyboard back from a slider or number field, a press anywhere else ends typing, and `Esc` always returns the keyboard to the canvas (Figma behaviour; #244).

**Dialogs.** `Esc` closes any dialog: the quit question (as **Cancel**), the method picker (reverting its preview), a menu or popover. While a dialog is open, no shortcut acts on the app behind it: `Q`, `Shift C`, `Cmd E` and the rest wait until it closes. The usability review saw `Q` and `Shift C` switch the view and the overlay behind the old export dialog (M4, #287).

| Action | Shortcut | Convention source |
|---|---|---|
| Open image / project | `Cmd O` | all |
| Save project / Save as | `Cmd S` / `Shift Cmd S` | all |
| Export: show the Export panel (§6b) | `Cmd E` | Pixelmator, GIMP (Photoshop uses `Shift Cmd Alt W`; too obscure) |
| Export Again: write the export set into the last folder | `Shift Cmd E` | our own; the Makefile case |
| Undo / Redo | `Cmd Z` / `Shift Cmd Z` (also `Cmd Y`) | all; `Cmd Y` for GIMP users |
| Zoom in / out / fit / 100 % | `Cmd =` / `Cmd −` / `Cmd 0` / `Cmd 1` | Photoshop family; bare `1` also = 100 % for Krita/Aseprite users |
| Working view: Source ⇄ Result | `Q` | ours (unused by the Photoshop family for anything we need) |
| Jump to result at cursor | click the result panel | Photoshop Navigator |
| Pan | `Space` + drag, `H` | all |
| New layer from file / Duplicate / Delete | `Shift Cmd N` (browser: `Shift Cmd L`; both work natively) / `Cmd J` / `Delete` | Photoshop family; the browser's ours |
| Layer via mask / via cut (with a selection) | `Cmd J` / `Shift Cmd J` | Photoshop *Layer via copy / cut* |
| Toggle layer visibility | `Cmd ,` ; click eye | Photoshop/Pixelmator |
| Nudge 1 / 10 / 0.25 px | `←↑↓→` / `Shift+` / `Alt+` | editors / ours |
| Tools: Nudge, Pin, Lock, Region | `V`, `P`, `L`, `R` | single-key tools (Photoshop style) |
| Transform (scale / rotate) layer | `Cmd T` (native only) / `S` | Photoshop free transform / ours |
| Snap: Off → C64 pixel → Cell | `Shift Cmd ;` | Photoshop snap toggle |
| Rotate selected layer ±0.1° / ±1° | `Cmd Alt [` `]` / with `Shift` | ours |
| Scale selected layer ±1 % / ±10 % | `Cmd` + `[` `]` / with `Shift` (±10 % only with the Transform tool; otherwise `Shift Cmd [` `]` walk the worst cells) | ours |
| Mask mode on/off; mask tools Brush, Lasso, Gradient, Key | `M`; `B`, `W`, `G`, `K` | Photoshop (`B`, `G`), ours (`M`, `W`, `K`) |
| Brush size down / up; swap Add/Subtract (the current tool's own, §7) | `[` / `]`; `X` | Photoshop |
| Show mask alone | `Alt`-click mask thumbnail | Photoshop |
| Grid / Colour-limited cells / Border | `Cmd '` / `Shift C` / `Cmd B` | Affinity/Pixelmator for grid; ours |
| Hold-to-compare (flips to the other image) | `\` (hold) | Photoshop |
| Split / A-B / side-by-side | `,` cycles | Affinity's "split view cycle" |
| Next / previous worst cell | `Shift Cmd ]` / `[` | ours |
| Hide dock / show or hide UI (all chrome); `Esc` shows the UI | `Shift Cmd \` / `Cmd \` | Figma (`Tab` is focus traversal, §12) |
| Theme | menu only | — |
| Command palette | `Cmd K` (v2) | PhotoCraft, every modern tool |

Deliberately avoided: `Ctrl+Y` as the only redo (GIMP-only), `1`–`6` zoom levels (Aseprite-only), Pixelmator's Save-As/Duplicate swap.

**Keys the browser keeps** (usability review M3, #286). A browser does not give a page some keys: Chrome opens a tab on `Cmd T`, a window on `Cmd N` and an incognito window on `Shift Cmd N`. It closes the tab on `Cmd W` and the window on `Shift Cmd W`, and reopens a closed tab on `Shift Cmd T`. On macOS `Cmd Q` quits it. Outside an installed PWA or fullscreen keyboard lock the page never sees these keys. The browser build therefore binds none of them, and its menus and tooltips show the binding that works there:

| Action | Native | Browser | Why this one |
|---|---|---|---|
| New layer from file | `Shift Cmd N` (also `Shift Cmd L`) | `Shift Cmd L` | L for layer, and unbound before. `Alt Cmd N` was the other candidate, but on Windows `Ctrl Alt` is AltGr, which types a letter on many layouts (`ń` on Polish) |
| Transform tool | `Cmd T` or `S` | `S` | `S` already picks it on both |
| Quit | `Cmd Q` | — | a page cannot quit; closing the tab is the browser's |

`Cmd N` and `Cmd W` were never bound. In the browser a slip onto `Cmd W` (or a reload) meets the unsaved-changes guard first (§11, #241), and the autosave offers the session again on the next visit.

## 9. Words

One glossary, used identically in the app, `convert64 --help`, MCP tool names and the docs.

| Term | Means | Not |
|---|---|---|
| **Layer** | a source image with its own transform, mask, adjustments and recipe | "image", "picture" |
| **Selection** | a temporary mask made with the mask tools, used to create a region or a layer via mask | "marquee" |
| **Recipe** | the per-layer dithering + colour-fitting settings | "settings", "profile" |
| **Solve** / re-solve | run the constraint solver; produces the result | "convert", "render", "process" (CLI keeps `convert` as the verb for files) |
| **Result** | the solved C64 image shown in the preview | "output", "preview" (preview is the *view* of the result) |
| **Cell** | a 4×8 or 8×8 colour cell | "block", "tile", "char" (tile and char are reserved for charset mode) |
| **Colour-limited cell** | a cell where the solver wanted more colours than the mode allows and used the closest it may; the result is still legal. The core's report, the CLI and the MCP server keep the field name `clashes` | "clash" in the app, "error", "conflict" |
| **Pin** | require colours in a cell | "force" (Force is the fitting mode) |
| **Lock** | freeze a cell's result | "freeze" |
| **Region** | a painted area with its own recipe | "selection", "mask" (mask is per-layer alpha) |
| **Mask** | a per-layer weight that hides part of the layer and fades its dither; one raster plus feather/snap/levels modifiers | "alpha", "transparency" (the PNG's own alpha is separate) |
| **Feather** | Gaussian softening of a mask edge, measured in target pixels | "blur", "soft edge" |
| **Snap to cells** | quantising a mask edge to whole C64 cells | "align to grid" |
| **Adaptive / Force** | fitting modes (ArcPaint64's terms, kept so scene users recognise them) | "soft / hard" |
| **Mode** | the C64 graphics mode (Multicolor, Hi-res, Charset…) | "format" (format is the export file type) |

Copy rules (ux-writing defaults): sentence case; verb-first buttons (**Open image…**, **Export Koala**, **Reset group**); no "please", no exclamation marks; errors name cause and remedy and keep the user's input; numerals; units on every number (`px`, `%`, `ms`); file sizes in human units, SI, two or three digits (`512 B`, `9.8 kB`, `10 kB`, `1.8 MB`; #241).

Sample strings:

- Status, healthy: `Multicolor 160×200 · Source · cell 17,9 · px 68,72 · colours ■■■■ · error 0.031 · 200% · Colour-limited cells: 0 · solve 38 ms`. Each part is set off by a `·`; the numbers are in the monospace font, padded to their widest (`cell  7, 9`, `px  68, 72`), so the line does not jump as the pointer moves; the cell's colours are labelled `colours` and each swatch's tooltip names its colour (usability review m4).
- Status, with colour-limited cells: `… · Colour-limited cells: 12 · …` in normal text; tooltip: `Cells where the solver wanted more colours than the mode allows, so it used the closest ones it may. The picture is still legal and exports as it is. Shift C shows them.` The hint beside it, once: **Show colour-limited cells (Shift C)**.
- Export success (toast, 3 s, once the files are written): `Exported title.kla (10 kB), title.prg (10 kB)`
- Save success: natively `Saved /path/title.convert64 (1.3 MB)`; in the browser `Saved title.convert64 (1.3 MB)` once the file is written (Save dialog) or handed to the browser as a download, never before (#241).
- Export blocked (in the `error` colour, the only red text): `Can't export: 12 cells still exceed 4 colours. Let the solver resolve them (Layer › Fitting › Adaptive).`
- Slow solve: `Solving charset budget… 2 of 16 backgrounds` with a **Pause** button, never a spinner alone.
- Unsaved changes: the window title natively, the tab title in the browser, is `title.png* — Convert64` from the first change after the last Save until the next Save, Photoshop's asterisk convention; a restored session starts with it (#241). Export does not clear it: exporting does not save the project.
- Restore bar (under the toolbar, non-modal): `Restore your last session (title.convert64, 3 layers, 2 min ago)?` **Restore** · Discard
- Quit with unsaved changes (native only; the browser shows its own "Leave site?"): `Save changes to title.png before quitting?` / `Your changes are lost if you don't save them.` **Save** · Don't save · Cancel
- Autosave failed (status bar, warning colour, dismissible): `Autosave failed: storage is full. Save to keep your work.`
- Empty state: see §5.
- Tooltip, Force: `Keep this layer's colours even if neighbours pay`
- Tooltip, Adaptive: `Let the solver reassign cell colours for the lowest error`

## 10. Visual language

- **Theme:** egui `Visuals::light()` / `dark()`, system-following, override in View (ADR-007). Dark is what most users of this category expect (Photoshop, Affinity, Krita, PhotoCraft Pro default dark); light must be equally finished.
- **Semantic colours** (one `Theme` struct, two instances): `clash` (red family; the Colour-limited cells overlay only, never text: red text is kept for errors, which block export, #243), `pin` (amber), `lock` (blue-grey), `region` (teal), `selection` (accent), `grid` (40 % of fg), each verified for ≥3:1 against the dock surface and visible over the four test palette colours (Black, White, Yellow, Blue).
- **Type:** egui default proportional at 13 px for panels, the status bar in egui's `Small` style as the Figma `Status Bar` draws it (#203), its numbers in the monospace font at that size (m4), monospace for any address/size. Headings 14 px semibold. No display type.
- **Density:** 8 px spacing unit; rows 24 px in the layer list; 28 px touch targets minimum on controls (egui default), toolbar buttons 24×24 in the 28 px toolbar (Figma `Toolbar Button`; #203).
- **Icons:** Material Symbols, glyph map in ADR-007, 16 px in rows and toolbar, 20 px for the four tools. Icon + text on every toolbar button except tools (icon + tooltip + key).
- **Motion:** none beyond egui's 100 ms panel animations; the A/B flicker is the only "animation" and it is a feature.

## 11. States and feedback

| State | Treatment |
|---|---|
| No project | empty state in the preview area (§5); dock shows disabled groups, not hidden ones |
| Loading an image | the preview shows the source immediately, then the first solve (<100 ms) replaces it |
| Solving (fast path) | nothing visible; results arrive within a frame or two |
| Solving (full search, >200 ms) | status bar progress text + Pause; preview keeps the last result, slightly desaturated |
| Colour-limited cells | count in the status bar and the result header in normal text with a tooltip (a diagnostic: the result is legal); the overlay suggested once via a dismissible hint, never auto-enabled; no dots on the result panel until it is on (#243) |
| Export | the Export panel (§6b): a folder dialog natively, the browser's folder picker where it has one (Chromium), else a download per file, with no overlay (#241, #287); toast once the files are written; blocked export explains and links to the fix |
| Dialog open | the quit question is the one modal; `Esc` closes it, and no shortcut reaches the app behind it (§8) |
| Save | natively to the project's file (Save As asks); in the browser as Export, the project with every source embedded; the toast and the cleared `*` come only once the file is written |
| Unsaved changes | `*` after the name in the window or tab title (§9); in the browser a `beforeunload` guard asks before a reload or a closed tab drops them; natively quitting asks Save · Don't save · Cancel, and only then |
| Autosave | nothing visible: 1 s after each committed change (a drag counts on release), to OPFS or IndexedDB in the browser, a recovery folder natively ([reference/autosave.md](reference/autosave.md)); a failure is one line in the status bar's warning colour and work goes on |
| Restore | on start, a bar under the toolbar offers the last session if it holds changes the last Save does not: **Restore** opens it as unsaved work, Discard deletes it; ignoring it changes nothing |
| Undo | every steering action, recipe change and layer edit; slider drags coalesced; `Edit › History` list (v2) |
| Error in a file | inline in the status bar + a details popover with the byte offset; never a modal unless data would be lost |

## 12. Accessibility

- Every control reachable by `Tab`; custom-painted areas (preview, curves, palette swatches) are focusable and report a `WidgetInfo`.
- Arrow keys work inside the preview for cell navigation when a tool is active; `Enter` applies the tool to the focused cell.
- Contrast ≥4.5:1 for text, ≥3:1 for controls and overlays, in both themes.
- Native AccessKit on desktop; the browser build has no screen-reader support (egui limitation) — stated in the README, not hidden.
- No information by colour alone: colour-limited cells also get an outline and a count; pins/locks have glyphs; disabled swatches are hatched.

## 13. What we deliberately do not build

Brushes, pencil, fill, selection marquees, text, skew and perspective (transform is translate / scale / rotate / flip, ADR-011; painting excluded by ADR-003). The export step hands off to pixel editors; a later "Open in…" button is the right integration, not an in-app pencil.

## 14. Open questions for the first build (E8)

1. Right dock split ratio Recipe : Layers under the fixed Result panel — start 55 : 45, make the divider draggable, see where users leave it. On short windows (<800 px), does the Result panel need a collapse-to-half-height option? *Answered (usability review m8): it collapses by itself to a 160 × 100 strip below 800 px (§4.3).*
2. Does the sub-pixel nudge need a visible 0.25 px grid, or is the status-bar readout enough?
3. Colour-limited cells heatmap: per-cell error magnitude (continuous) or limited/not (binary) by default? (Usability review M1: continuous.)
4. Should "Region" be a pseudo-layer in the Layers list (discoverable) or stay a tool-only object (simpler)?
5. Does anyone stay in Result mode to work, or is Source-by-default right? Log mode time in the test.
6. First usability test: 5 users (2 game devs, 2 graphicians, 1 coder), tasks: convert the `title` fixture with a smooth sky; pin the soldier's face colours; export a Koala that loads in VICE. Measure time-to-first-legal-export and whether anyone finds the Colour-limited cells overlay without a hint.

## References

Editor conventions: Adobe Photoshop shortcuts (helpx.adobe.com/photoshop/using/keyboard-shortcuts-web.html); Affinity Photo 2 interface and shortcuts (affinity.help/photo2); PhotoCraft README and `docs/ui-parity.md` (github.com/storytold/photocraft); GIMP 3 docs (docs.gimp.org/3.0); Krita 5 docs (docs.krita.org); Aseprite docs (github.com/aseprite/docs); Pixelmator Pro user guide (support.pixelmator.com); Photopea learn pages (photopea.com/learn). Converters: retropixels+ write-up (theoasisbbs.com), png2c64.app, ArcPaint64 (csdb.dk/release/?id=260323), C64CC (elder0010.com/projects/c64cc), ALBERT (albertpixels.com), Disco Volante (generationamiga.com, 2026-07), convertron3000 and retropic (GitHub), Ditherista (github.com/robertkist/ditherista), Dither Boy (digitalproduction.com), Photoshop Indexed Color (learn.adafruit.com), GIMP convert-to-indexed docs. egui: docs.rs/egui 0.36 (Panel, Slider, KeyboardShortcut, TextureOptions, ThemePreference), eframe_template, rfd wasm support table, egui accessibility.md.
