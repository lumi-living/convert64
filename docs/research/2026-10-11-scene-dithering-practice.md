# How C64 graphicians dither: what the sources actually support

Scope note: web search surfaced little long-form tutorial material. Most evidence is CSDb release comments, converter/tool docs and two interviews. Several requested sources (Reddit, YouTube, Lemon64, Pixcen/Multipaint manuals, Timanthes docs, Project One docs) returned nothing usable or were blocked. Gaps are flagged. Labels: **[Documented]** = stated in a tool/reference page; **[Artist]** = one named person's statement; **[Not found]**.

## 1. Named pattern types and when they are chosen

**What the sources show**
- **Checkerboard / "chessboard" dither.** Joe calls the moire chessboard dither "long known" and says it gives extra colours when viewed from a distance, possibly independent of PAL blending [Artist: Joe on Lobo's *Real OG*, https://csdb.dk/release/?id=93183]. Joe also links the "136 colours" idea (16 colours mixed pairwise, 16+15+...+1 = 136) to Koala Painter and Blazing Paddles, and ties it to "the chessboard thing about PAL-blending" [Artist: https://csdb.dk/release/?id=93206].
- **Line alternation vs. dither.** Same thread: Joe says "alignment of lines producing new colors of equal luminance simply looked awkward", so he dithered "regardless the eventual moire effect it caused on a real setup" [Artist: https://csdb.dk/release/?id=93206]. One person's opinion, but it is the clearest scene statement of the line-vs-checker trade-off I found.
- **Diagonal line dithering.** Tobias (-trb-) says he took inspiration from recent pictures using diagonal line dithering, did skew/deskew in Photoshop and the dithering in Retropaint, and states "no publicly available software" did this dithering; deskewing 160 columns by hand takes minutes. He used greyscale sources to avoid colour clashes. Jetboy replied it is "virtually the same as" checkerboard on the original [Artist: https://csdb.dk/release/?id=233261]. Treat diagonal-line dithering as a recent, niche technique.
- **Pattern vocabulary in tools.** Colodore-style tools list "Alternating Columns", "Checkered", "Checkered 2x" and horizontal 25/50/75% patterns [Documented: https://pagetable.com/c64ref/colors/]. Multipaint has numbered dither brushes: Slaxx mentions "Dither 7/9" with grey for clouds [Artist: https://csdb.dk/release/?id=237328]. Convertron3000 offers Bayer ("my favourite for C64 pictures"), a modified "line dithering", and "dot dithering" [Documented: https://github.com/fieserWolF/convertron3000].
- **Loose / irregular dither.** Facet says he learned a "loosen" dithering style from HOF/Movement and that it saved about half the time; a faulty mouse that left pixels unfinished gave a "random" effect he liked [Artist: https://csdb.dk/release/?id=206229].
- **Minimal dither.** Jak T Rip: "no simple dither stuff. I tend to dislike dithering in most cases" and prefers careful colour setting [Artist: https://csdb.dk/release/?id=29152].
- **Dither with specific colours sparingly.** Electric avoided dithering with black "as little as possible" in *Fleshy* [Artist: https://csdb.dk/release/?id=209413].

**Not found:** Any source naming "stipple", "crosshatch", "25/75", "noise/grain" or "clustered" as scene terms. The 25/50/75 wording appears only in tool documentation. No source gave a rule of when each pattern is chosen beyond the items above.

**Orientation (multicolor 2:1 vs hi-res).** No source states a preference with reasons. What is documented:
- Multicolor pixels are 2x1 (160x200) [Documented: https://marincomics.com/c64-graphics.html; https://www.cosmigo.com/promotion/docs/onlinehelp/gfxHardware-c64.htm]. Steve Day (STE'86) draws in Photoshop at 2:1 aspect [Artist: https://www.atlantis-prophecy.org/recollection/?load=interviews&id_interview=162].
- Colodore-type tooling warns that column-alternating and checker patterns can produce visible vertical low-frequency stripes over large areas ("GEOS effect") [Documented: https://pagetable.com/c64ref/colors/]. That is a tool author's observation about wide areas, not a hand-dithering rule.
- PAL line blending works across scanlines, so it depends on horizontal-line (alternating row) patterns (section 2).

## 2. Same-luminance pairs and PAL blending

**Luma levels** [Documented, Pepto's colour work, https://www.pepto.de/projects/colorvic/]: most-common revision luma groups are Black; Blue+Brown; Dk.Grey+Red; Purple+Orange; Md.Grey+Lt.Blue; Green+Lt.Red; Lt.Grey+Cyan; Yellow+Lt.Green; White (that page lists these as luma values 0, 8, 10, 12, 15, 16, 20, 24, 32). pagetable states the first-revision VIC-II has 5 luma levels and all later ones have 9 [Documented: https://pagetable.com/c64ref/colors/]. Note the two pages differ on how many distinct levels/ties the earlier chip had; I did not reconcile this beyond quoting each.

**PAL blending mechanism** [Documented]: the display's delay line averages the chroma of adjacent scanlines but not their brightness, so alternating-row mixes of equal-luma colours are real signal-level blends, not just optical mixing [https://pagetable.com/c64ref/colors/; https://en.wikipedia.org/wiki/Composite_artifact_colors]. Wikipedia says this yields seven additional colours and cites *Mayhem in Monsterland* and *Parallaxian* as games using it.

**Pairs actually seen in practice** (thin):
- Blue + brown alternated by line gives a dark grey; swapping line order gives two slightly different mixes; "only with PAL, not NTSC" [Artist: Tobias, quoted by Mermaid, *PALmix Island*, https://csdb.dk/release/?id=230750]. Tobias says using same-luminance colours "will generate perfectly mixed (additional) colors" and a green/purple interference effect gives six colours total from two; best viewed on real C64 and CRT.
- DeeKay: blue/brown gives dark grayish red or dark grey on a Philips monitor but always dark grey on a 1084 [Artist, https://csdb.dk/release/?id=46312].
- Wile Coyote used the same approach in *The Famous Melee Island* and claims a clearer result than Tobias [Artist, via https://csdb.dk/release/?id=230750].

**Rules / caveats** (all from comments, not manuals):
- Result depends on display: ptoing says it is the PAL signal, not the monitor, and expects it everywhere; Ed and DeeKay report differences between even/odd lines and between monitors; Ed notes dependence on starting y-position and that VICE does not reproduce it [Artist, https://csdb.dk/release/?id=46312]. Contradictory anecdotes, so treat monitor-specific claims as unverified.
- NTSC does not blend this way [Artist: Tobias, https://csdb.dk/release/?id=230750].
- Larger luma gaps make the pattern more visible; equal-luma pairs blend seamlessly [Documented: https://pagetable.com/c64ref/colors/].

## 3. Cell-boundary tactics

Thin; mostly documentation of limits rather than artist tactics.
- Hi-res: 2 colours per 8x8 cell. Multicolor: 4 colours per cell, one of which (background) is global [Documented: https://marincomics.com/c64-graphics.html; https://www.cosmigo.com/promotion/docs/onlinehelp/gfxHardware-c64.htm].
- STE'86: "so much less frustrating to draw something, THEN sort out the attribute clash" [Artist, https://www.atlantis-prophecy.org/recollection/?load=interviews&id_interview=162]. Draw-first-fix-later is the one workflow statement I found.
- Facet: the C64's grid and palette restrictions speed him up because "limitations often make decisions" [Artist, paraphrase of comment at https://csdb.dk/release/?id=206229].
- Editors flag clash visually (Multipaint red miscolourings; Pixcen "constraint feedback, color-source tools, bitpair remapping, protected colors, color priorities") [Documented: https://marincomics.com/c64-graphics.html; https://theoasisbbs.com/pixcen-deluxe-c64-pixel-editor-the-tool-your-vic-ii-has-been-waiting-for/]. The Pixcen description comes from a news article, not the manual.
- Pro Motion: double-width pixel view, colour constraint checking, background copied to the transparent slot for MC char projects [Documented: https://www.cosmigo.com/promotion/docs/onlinehelp/gfxHardware-c64.htm].
- A Mufflon/NUFLI note mentions stripe artifacts at tile boundaries in converter output [Documented: https://www.c64-wiki.com/wiki/NUFLI].

**Not found:** Any artist describing "dither bleeding" across cells, deliberate colour sharing between neighbouring cells, or a strategy for background-colour choice. I'm not going to invent these.

## 4. Mode-specific practice

- **FLI:** per-scanline colour choice inside a cell gives more variety; first three tiles per row are unusable (FLI bug) [Documented: https://www.cosmigo.com/promotion/docs/onlinehelp/gfxHardware-c64.htm].
- **IFLI/MCI interlace:** two pictures alternate each frame; two mixed colours give "theoretically 128" colours; the alternating frames can look like jerky side-to-side movement [Documented: https://studiostyle.sk/dmagic/gallery/gfxmodes.htm]. Electric: "IFLI finally made it possible to actually draw good looking pictures"; he now uses Draz Lace (two Koala pictures laced), valuing memory and raster time over the reduced colour range [Artist, https://www.atlantis-prophecy.org/recollection/?load=interviews&id_interview=45]. Leon's MCI *Dream Land* drew praise for carefully shaped dithering (Sander, Hoild) [Artist comments, https://csdb.dk/release/?id=22875]. Which patterns flicker vs. don't: **not found** in any source I could open. Compare Helm's *Nightanday*, praised for "non-flickering goodness" (dalezy) [https://csdb.dk/release/?id=29152].
- **NUFLI:** advertised as flicker-free; many IFLI pictures "stopped flickering" once Crest Slide Story appeared (c64-wiki, unsourced claim) [https://www.c64-wiki.com/wiki/NUFLI]. Slaxx: "NUFLI is a thankless format" - either fight the editor or keep fixing clashes after conversion [Artist, https://csdb.dk/release/?id=237328].
- **Hi-res:** c64-wiki-style gallery text says hi-res is rarely used with more than 2 colours and multicolor "almost nobody" paints nicely in, but this is one site's opinion [https://studiostyle.sk/dmagic/gallery/gfxmodes.htm]. STE'86 prefers multicolor and sees no reason to use hi-res for art [Artist]. Tobias's *PALmix Island* is a hires piece built on PAL line mixing [https://csdb.dk/release/?id=230750].
- **Charset mode under a 256-char budget:** Electric's *Fleshy* uses multicolour charmode with sprites in the borders, four charset colours, and two midtones so mixing can happen via dithering as well as raster lines; he says it "produces tones that look kinda non-C64" and works best on a real CRT [Artist, https://csdb.dk/release/?id=209413]. How artists budget and reuse dither tiles: **not found**.
- **Sprite overlay:** NUFLI layers a sprite plane under/over hi-res (wiki page above). Dithering practice for sprites: **not found**.

## 5. Artists on converters

- Sander objects when converted layers are presented as hand-pixelled ("insulting to others") [Artist, https://csdb.dk/release/?id=237328]. chatGPZ: the 136-colour pattern dither "reminds me too much of the output produced by converters" and he prefers traditional dithering [Artist, https://csdb.dk/release/?id=93206].
- Oxidy (*Badning*, FLI): "every single pixel set by hand (at least once)", dithered and moved pixels for many hours; Jailbird said it "looks hand-pixelled"; Sander once accused him of tracing, retracted in 2010 [Artist, https://www.csdb.dk/release/?id=8085].
- Converter authors admit limits: heavy dither "can look impressive on a modern monitor but turn messy once split into C64 cells", reducing dither strength often cleans results [Documented: https://www.generationamiga.com/2026/07/12/a-new-windows-tool-makes-c64-image-conversion-surprisingly-easy/]; the tool "won't replace careful pixel editing". c64img's auto-fix is "pretty naive" [Documented: https://pypi.org/project/c64img/]. a1bert's converter advises leaving out red and enabling no more than three colours in hires, and raising luma weight for non-realistic pictures [Documented: https://a1bert.kapsi.fi/Dev/vicgfx/].
- Positive example: CapFuture1975 won a compo with Retropixels output with "no post-processing"; bepp called it impressive and questioned whether it looked converted [https://csdb.dk/release/?id=229560]. Shows converters can place at the top of a compo, but also drew objections.
- Tools named: Retropixels (Michel de Bree, https://csdb.dk/release/?id=229560), Mufflon, NUFLI Editor, Multipaint, Retropaint, Photoshop/Gimp [https://csdb.dk/release/?id=237328].
- Specific failure modes (speckle, "worms", edge loss, wrong pairs): described in tool docs only generally (cell-boundary stripes, mess in cells, lost detail). Hand-fix lists from artists: **not found**.

## 6. Numeric rules of thumb

- 16+15+...+1 = 136 pair combinations (arithmetic, Earl Pottinger comment https://www.pagetable.com/?p=1747; Joe https://csdb.dk/release/?id=93206).
- Luma levels: 9 in later chips (pagetable, above).
- Colodore's "Max Luma Diff" slider: smaller values give smoother results; "Limit Max Luma Diff" restricts to hardware-safe range, but no numeric threshold given [https://pagetable.com/c64ref/colors/].
- Pepto's recommended gamma: PAL source 2.8, target sRGB 2.2 for conversion [https://www.pepto.de/projects/colorvic/].
- Viewing distance, CRT vs LCD cut-offs: **not found**. Only qualitative: Tobias and Electric say pieces are best on real CRT; Joe says chessboard dither mixes at a distance. No source gave a numeric "no finer than X".

## Source reliability summary
- Widely stated across independent sources: equal-luma pairs blend under PAL line alternation; equal blend only on PAL; converter output needs manual cleanup.
- One person's opinion: line patterns look awkward (Joe); dislike of dither (Jak T Rip); diagonal line dither as new (Tobias); Bayer as favourite (Convertron author).
- Gaps worth filling by direct reading: CSDb forum threads (not reachable via search here), Pixcen/Multipaint manuals, Facet/Mermaid tutorials if they exist.
