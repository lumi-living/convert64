# Usability test plan — Convert64 0.1

**Status:** plan, not yet run (#9). It is the first user test that [UX.md §14](../UX.md#14-open-questions-for-the-first-build-e8) item 6 asks for. The expert review of 2026-10-13 ([2026-10-13-usability-review.md](2026-10-13-usability-review.md)) came first: its critical findings C1–C4 are fixed (#248, #249, #285, #255), so this test measures the design and not those bugs.

## Goals

1. Can people who make C64 graphics for a living or a hobby get from a picture to a legal file that loads in VICE without help?
2. How long does the first export take?
3. Does anyone find the colour-limited cells view (the *Colour-limited cells* overlay, formerly "clashes", #243) without a hint from the facilitator?
4. Secondary, from UX.md §14: where people leave the dock divider (item 1); whether they ask for a sub-pixel grid (item 2); whether they stay in Source or Result mode to work (item 5, log the time in each).

## Participants

Five people, matching the UX.md §1 users:

| # | Profile | Recruit for |
|---|---|---|
| P1, P2 | **Game developer** | has shipped or is making a C64 game; uses a Makefile or build script; has converted art for it before |
| P3, P4 | **Scene graphician** | has released C64 pictures; works in Pixcen, Multipaint, ArcPaint64 or similar; has opinions about dithering |
| P5 | **Demo coder** | writes 6502 for demos; needs pictures as data; comfortable on the command line |

- Nobody who has worked on Convert64 or seen its issues in detail.
- At least one participant on each of macOS and Windows or Linux if the desktop packages are ready; otherwise everyone uses the browser build.
- Remote over a video call with screen sharing, or in person. 60 minutes each.

## Setup

| Item | Value |
|---|---|
| Build | **Browser:** `[https://lumi-living.github.io/convert64/]` (the Pages build, #9) in desktop Chrome or Firefox. **Or desktop:** `[release URL, v0.1.0]` (unsigned: show the right-click › Open step on macOS before the session starts, it is not part of the test). Write down the exact commit or version. |
| Files | Send before the session, in one folder: `title.png` (`fixtures/title/source.png`), `title-portrait.convert64` (below), and nothing else. |
| `title-portrait.convert64` | Made by the facilitator beforehand: open `title.png`, *Layer › New Layer from File…* `portrait.png` (`fixtures/portrait/source.png`), a radial gradient mask so the face sits in the snow scene, save. These are the E8 exit steps ([docs/screenshots/210/README.md](../screenshots/210/README.md)), so the soldier's face is a known target. Then embed the sources so the file opens alone: `convert64 apply title-portrait.convert64 --embed --in-place` ([cli.md](../reference/cli.md)). Open it once in the test build to check. |
| VICE | `x64sc` 3.x installed on the participant's machine, or on the facilitator's (the participant sends the file). Default settings, PAL. |
| Window | At least 1280 × 800. Clear the browser's site data (or use a fresh profile) so no autosaved session is offered. |
| Recording | Screen and voice, with consent (below). A stopwatch or the recording's timestamps for the times. |
| Facilitator | One person runs the session and does not help; a second takes notes if possible. |

## Consent

Read aloud and get a yes before recording; note it in the results.

> Thanks for helping. We're testing Convert64, not you: if something is hard, that's what we need to find. I'll ask you to do three things and to think aloud while you do them. I won't help unless you're stuck for a long time; you can stop or skip a task at any point, and you can stop the whole session whenever you like.
>
> With your permission I'll record the screen and your voice. The recording is only for the Convert64 team to take notes from; it won't be published, and I'll delete it once the notes are written, within [N] weeks. The notes won't use your name: you'll be P1 to P5. Pictures you make in the session stay yours; we'd only use them in the notes if you say so.
>
> Is that OK? Do you have questions before we start?

## Script

Times are a guide; total about 60 minutes.

### 0. Warm-up (5 min)

- "What do you use to make or convert C64 graphics today? Walk me through the last picture you made."
- "What annoys you most about converting pictures for the C64?"

Do not show Convert64 yet.

### Task 1 — Convert `title` with a smooth sky (15 min)

> "Here is a painting, `title.png`. Convert it to a C64 multicolor picture you would be happy with. The sky in particular should look smooth: no stripes, worms or blotches. Tell me when you're done."

- **Start** the clock when the app is on screen with nothing open.
- **Done** when the participant says the sky is acceptable. Ask them to rate the sky 1–5 ("1 = I'd redo it by hand, 5 = I'd ship it").
- **Success:** the participant is satisfied with the sky (rating ≥ 3) and the picture is still the whole `title` scene. **Partial:** satisfied only after a hint. **Fail:** gives up, or 15 minutes pass.
- Expected paths (do not suggest them): change the method or strength on the Recipe tab; duplicate the layer or paint a Region for the sky with its own recipe; adjust levels or gamma.
- Note which dither methods they try and whether they use the method picker's thumbnails or search.

### Task 2 — Pin the soldier's face colours (10 min)

> "Open `title-portrait.convert64`. The soldier's face should keep its skin colours whatever else you change. Make sure it does."

- **Done** when the face's cells have pins (check the Pins & locks overlay or the status bar readout after the session).
- **Success:** pins on the face's cells and the participant can say what they did. **Partial:** pins set after a hint, or pins on the wrong cells. **Fail:** gives up, or 10 minutes pass.
- Then: "Change something else in the picture, anything." Watch whether they check that the face kept its colours.
- Expected path: Pin tool (`P`), a click or a drag over the face's cells in Source or Result mode; or the cell's right-click menu.

### Task 3 — Export a Koala that loads in VICE (10 min)

> "Now get this picture (either one, your choice) into VICE as a Koala file, the way you would for your own work."

- **Done** when VICE shows the picture.
- **Success:** a `.kla` or `.prg` that VICE shows correctly. **Partial:** an export that works after a hint, or a different format that loads. **Fail:** gives up, or 10 minutes pass.
- Note: whether they look for an Export button, a menu, a shortcut; whether they choose `.kla` or `.prg`; how they load it in VICE (a Koala file needs a viewer, the PRG autostarts).

### 4. Debrief (10 min)

- "What was the hardest part? The easiest?"
- "Did you notice anything that told you how good or bad the conversion was?" (Probe for the *Colour-limited cells* count and overlay **only now**, and only if they did not open it.)
- "Would you use this in your own work? What would stop you?"
- "Anything you expected and didn't find?"
- Game developers and the coder: "How would you put this in your build?" (the CLI and the project file).

## Measures

| Measure | How | Notes |
|---|---|---|
| **Time to the first legal export** | From the start of Task 1 to the first file written by an export, in any task, of any format | Every export is legal by construction (UX.md §2, principle 3), so this is the time to the first export. Record the format too. If no export happens, record "none". |
| **Found the colour-limited cells view without a hint** | Yes / no, and when: the participant opens the overlay (`Shift C`, *View › Colour-Limited Cells*, the status bar link or the result panel's count) before the debrief question | The status bar's own "Show colour-limited cells (Shift C)" link counts as part of the UI, not a hint. A hint is anything the facilitator says or points at. |
| Task success | Success / partial / fail per task, with the time | Definitions in each task above |
| Hints | Count and wording of every hint per task | Give a hint only after about 2 minutes stuck, starting with "What would you try next?" |
| Sky rating | 1–5 at the end of Task 1 | Task 1 only |
| Mode time | Rough minutes in Source and in Result mode | UX.md §14 item 5 |
| Dock divider | Where it ends (screenshot at the end) | UX.md §14 item 1 |
| Sub-pixel | Whether they nudge by less than a pixel (`Alt` arrows or the X/Y fields), and whether they ask for a grid | UX.md §14 item 2 |
| Notes | Quotes, errors, workarounds, things they looked for and didn't find | Each with a timestamp |

## Results template

Copy one block per participant into `docs/research/usability-test-0.1-results.md`, then fill the summary.

```markdown
### P[n] — [game developer / graphician / coder], [date], [browser + OS / desktop + OS], build [commit]

- Consent to record: [yes / no] · Recording deleted: [date]
- Current tools: [...]

| Task | Result | Time | Hints | Notes |
|---|---|---|---|---|
| 1. Smooth sky | [success / partial / fail] | [mm:ss] | [n: wording] | sky rating [1–5]; methods tried [...] |
| 2. Pin the face | [success / partial / fail] | [mm:ss] | [n: wording] | [...] |
| 3. Koala in VICE | [success / partial / fail] | [mm:ss] | [n: wording] | format [kla / prg]; loaded by [...] |

- Time to the first legal export: [mm:ss from the start of Task 1, format] / none
- Found the colour-limited cells view without a hint: [yes at mm:ss, via (Shift C / menu / status bar / panel count)] / no
- Mode time: Source [n] min, Result [n] min · Dock divider: [ratio] · Sub-pixel nudge used: [yes / no]; asked for a grid: [yes / no]
- Quotes: "[...]"
- Problems seen (severity: critical / major / minor): [...]
```

### Summary (after all five)

| | P1 | P2 | P3 | P4 | P5 |
|---|---|---|---|---|---|
| Task 1 | | | | | |
| Task 2 | | | | | |
| Task 3 | | | | | |
| First export (mm:ss) | | | | | |
| Found colour-limited cells unaided | | | | | |

- Median time to the first legal export: [mm:ss] (range [..]).
- Found the colour-limited cells view without a hint: [n] of 5.
- Problems seen by two or more participants, with a proposed fix and an issue number each.
- Answers to UX.md §14 items 1, 2 and 5, and any change to UX.md they call for (spec first, in the same PR as the fix).
