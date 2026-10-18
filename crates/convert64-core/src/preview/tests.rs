use super::*;
use crate::composite::Compositor;
use crate::layer::{DitherKind, Layer, SourceImage};

fn photo_frame(mode: Mode) -> TargetFrame {
    let px: Vec<[f32; 4]> = (0..320 * 200)
        .map(|i| {
            let (x, y) = ((i % 320) as f32 / 320.0, (i / 320) as f32 / 200.0);
            [x * x, y, (1.0 - x) * 0.6 + 0.2 * y, 1.0]
        })
        .collect();
    let l = Layer::new(
        "g",
        Arc::new(SourceImage::from_linear(320, 200, px).unwrap()),
    );
    let mut c = Compositor::new(SuperGrid::new(mode, 1).unwrap());
    c.update(&[l], &[]);
    c.frame().clone()
}

fn colours_in_cell(t: &Tile, cx: u32, cy: u32, cw: u32) -> usize {
    let mut seen = Vec::new();
    for y in cy * 8..cy * 8 + 8 {
        for x in cx * cw..cx * cw + cw {
            let i = ((y * t.width + x) * 4) as usize;
            let p = [t.rgba[i], t.rgba[i + 1], t.rgba[i + 2]];
            if !seen.contains(&p) {
                seen.push(p);
            }
        }
    }
    seen.len()
}

#[test]
fn a_tile_is_four_by_four_cells_and_legal() {
    let pal = Palette::default();
    for (mode, w, cw, most) in [
        (Mode::HiresBitmap, 32, 8, 2),
        (Mode::MulticolorBitmap, 16, 4, 4),
    ] {
        let src = SourceTile::gradient(mode, &pal, Some(ColourIndex::BLACK));
        for kind in [
            DitherKind::FloydSteinberg,
            DitherKind::Bayer4,
            DitherKind::Yliluoma2,
        ] {
            let t = method_tile(Method::new(kind), &Recipe::default(), mode, &pal, &src);
            assert_eq!((t.width, t.height), (w, 32), "{mode:?}");
            assert_eq!(t.rgba.len(), (w * 32 * 4) as usize);
            for cy in 0..4 {
                for cx in 0..4 {
                    assert!(colours_in_cell(&t, cx, cy, cw) <= most, "{mode:?} {kind:?}");
                }
            }
            // Every pixel is a palette colour.
            for p in t.rgba.chunks(4) {
                assert!(
                    pal.rgb_table()
                        .iter()
                        .any(|c| [c.r, c.g, c.b] == [p[0], p[1], p[2]])
                );
            }
        }
    }
}

#[test]
fn tiles_are_deterministic_and_differ_by_method() {
    let pal = Palette::default();
    let mode = Mode::HiresBitmap;
    let src = SourceTile::gradient(mode, &pal, None);
    let r = Recipe::default();
    let a = method_tile(Method::new(DitherKind::Atkinson), &r, mode, &pal, &src);
    let b = method_tile(Method::new(DitherKind::Atkinson), &r, mode, &pal, &src);
    assert_eq!(a, b);
    let none = method_tile(Method::new(DitherKind::None), &r, mode, &pal, &src);
    let checker = method_tile(Method::new(DitherKind::Bayer4), &r, mode, &pal, &src);
    assert_ne!(none, checker);
    assert_ne!(a, checker);
}

/// The tile is what the real solver gives the layer there: an ordered method's tile
/// matches the full-picture solve of a frame showing the same block, cell for cell,
/// when the rest of the picture is the flat background (ordered dithers carry no error
/// between cells, so the block's neighbours do not matter).
#[test]
fn an_ordered_tile_is_the_solvers_result_at_that_place() {
    let pal = Palette::default();
    let mode = Mode::HiresBitmap;
    let frame = photo_frame(mode);
    let src = SourceTile::crop(&frame, CellPos::new(10, 10), None);
    let recipe = Recipe::with_method(DitherKind::Bayer4, 1.0).unwrap();
    let tile = method_tile(recipe.method(), &recipe, mode, &pal, &src);
    // The same pixels solved as a full picture (cells outside the block locked).
    let s = Solver::new(&pal);
    let mut full = frame.clone();
    full.attributes_mut().set_recipes(vec![recipe], Vec::new());
    let mut steering = Steering::new();
    for c in CellSet::all().iter() {
        if !src.block_cells().contains(c) {
            steering.set_lock(c, true);
        }
    }
    let base = flat_solution(mode, ColourIndex::BLACK);
    let sol = s.solve(&full, &steering, Some(&base), &SolveOptions::default());
    let (ox, oy) = (
        (u32::from(src.origin().x) + 1) * 8,
        (u32::from(src.origin().y) + 1) * 8,
    );
    for y in 0..32 {
        for x in 0..32 {
            let c = pal.rgb(sol.image.pixel((ox + x) as u16, (oy + y) as u16));
            let i = ((y * 32 + x) * 4) as usize;
            assert_eq!(
                [c.r, c.g, c.b],
                [tile.rgba[i], tile.rgba[i + 1], tile.rgba[i + 2]]
            );
        }
    }
}

#[test]
fn the_gradient_carries_seeded_jitter_of_two_levels() {
    let pal = Palette::default();
    let a = SourceTile::gradient(Mode::HiresBitmap, &pal, None);
    let b = SourceTile::gradient(Mode::HiresBitmap, &pal, None);
    assert_eq!(a, b, "seeded: the same every time");
    // Neighbouring pixels on a diagonal step differ by the ramp plus at most ±2 levels
    // each, so some pairs that the ramp alone would make equal differ.
    let (w, _) = block_px(Mode::HiresBitmap);
    let srgb: Vec<[u8; 3]> = a.colour.iter().map(|c| c.map(linear_to_srgb8)).collect();
    let mut jittered = 0;
    for y in 0..w - 1 {
        for x in 1..w {
            // (x, y) and (x - 1, y + 1) lie on the same anti-diagonal: the same ramp t.
            let (p, q) = (srgb[y * w + x], srgb[(y + 1) * w + x - 1]);
            let d = p.iter().zip(q).map(|(a, b)| a.abs_diff(b)).max().unwrap();
            if d > 0 {
                jittered += 1;
            }
        }
    }
    assert!(jittered > w * w / 4, "{jittered}");
    // It runs from dark to light along the diagonal.
    let lum = |c: [f32; 3]| luminance(c);
    assert!(lum(a.colour[0]) < 0.05);
    assert!(lum(*a.colour.last().unwrap()) > 0.8);
}

#[test]
fn a_crop_stays_on_the_screen_and_takes_the_frames_pixels() {
    let frame = photo_frame(Mode::MulticolorBitmap);
    assert_eq!(
        SourceTile::crop(&frame, CellPos::new(0, 0), None).origin(),
        CellPos::new(0, 0)
    );
    assert_eq!(
        SourceTile::crop(&frame, CellPos::new(39, 24), None).origin(),
        CellPos::new(34, 19)
    );
    let t = SourceTile::crop(&frame, CellPos::new(12, 7), Some(ColourIndex::BLUE));
    assert_eq!(t.origin(), CellPos::new(10, 5));
    assert_eq!(t.background(), Some(ColourIndex::BLUE));
    let (fw, _) = frame.grid().target_size();
    let (x0, y0) = (10 * 4, 5 * 8);
    assert_eq!(t.colour[0], frame.colour()[y0 * fw as usize + x0]);
    // Hi-res has no background.
    let h = SourceTile::crop(
        &photo_frame(Mode::HiresBitmap),
        CellPos::new(5, 5),
        Some(ColourIndex::BLUE),
    );
    assert_eq!(h.background(), None);
}

#[test]
fn keys_change_with_settings_and_source_but_not_the_method() {
    let pal = Palette::default();
    let opts = SolveOptions::default();
    let r = Recipe::default();
    let mode = Mode::MulticolorBitmap;
    let base = settings_hash(&r, mode, &pal, &opts);
    assert_eq!(base, settings_hash(&r, mode, &pal, &opts));
    let other_method = r.with_dither_method(Method::new(DitherKind::Bayer8));
    assert_eq!(
        base,
        settings_hash(&other_method, mode, &pal, &opts),
        "the method is its own field"
    );
    let weaker = Recipe::with_method(DitherKind::FloydSteinberg, 0.5).unwrap();
    assert_ne!(base, settings_hash(&weaker, mode, &pal, &opts), "strength");
    assert_ne!(
        base,
        settings_hash(&r, Mode::HiresBitmap, &pal, &opts),
        "mode"
    );
    assert_ne!(
        base,
        settings_hash(&r, mode, &Palette::pepto_ntsc(), &opts),
        "palette"
    );
    let native = SolveOptions {
        solver: SolverKind::Native,
        ..SolveOptions::default()
    };
    assert_ne!(base, settings_hash(&r, mode, &pal, &native), "solver");
    let interactive = SolveOptions {
        mode: SolveMode::Interactive {
            dirty: CellSet::EMPTY,
        },
        ..SolveOptions::default()
    };
    assert_eq!(
        base,
        settings_hash(&r, mode, &pal, &interactive),
        "the solve mode is not a setting"
    );

    let g = SourceTile::gradient(mode, &pal, Some(ColourIndex::BLACK));
    let g_blue = SourceTile::gradient(mode, &pal, Some(ColourIndex::BLUE));
    let crop = SourceTile::crop(
        &photo_frame(mode),
        CellPos::new(20, 12),
        Some(ColourIndex::BLACK),
    );
    assert_eq!(
        g.hash(),
        SourceTile::gradient(mode, &pal, Some(ColourIndex::BLACK)).hash()
    );
    assert_ne!(g.hash(), g_blue.hash(), "background");
    assert_ne!(g.hash(), crop.hash(), "source");
}

#[test]
fn the_cache_keeps_the_newest() {
    let mut c = TileCache::new(2);
    let key = |s| TileKey {
        method: Method::new(DitherKind::Atkinson),
        settings: s,
        source: 1,
    };
    let t = Arc::new(Tile {
        width: 1,
        height: 1,
        rgba: vec![0; 4],
    });
    c.insert(key(1), Arc::clone(&t));
    c.insert(key(2), Arc::clone(&t));
    c.insert(key(2), Arc::clone(&t));
    assert_eq!(c.len(), 2);
    c.insert(key(3), t);
    assert!(!c.contains(&key(1)));
    assert!(c.contains(&key(2)) && c.contains(&key(3)));
}

/// Times every method's thumbnail on both sources (UX §6a's benchmark):
/// `cargo test -p convert64-core --release --features parallel -- --ignored --nocapture thumbnail_timing`.
#[test]
#[ignore = "timing, run by hand"]
fn thumbnail_timing() {
    use std::time::Instant;
    let pal = Palette::default();
    for mode in [Mode::MulticolorBitmap, Mode::HiresBitmap] {
        let ts = TileSolver::new(&pal, SolveOptions::default());
        let frame = photo_frame(mode);
        let sources = [
            (
                "gradient",
                SourceTile::gradient(mode, &pal, Some(ColourIndex::BLACK)),
            ),
            (
                "picture",
                SourceTile::crop(&frame, CellPos::new(20, 12), Some(ColourIndex::BLACK)),
            ),
        ];
        let r = Recipe::default();
        let all = Instant::now();
        let mut slowest = (0.0, DitherKind::None);
        for (_, src) in &sources {
            for info in &catalogue::CATALOGUE {
                let t = Instant::now();
                let _ = ts.render(Method::new(info.kind), &r, src);
                let ms = t.elapsed().as_secs_f64() * 1e3;
                if ms > slowest.0 {
                    slowest = (ms, info.kind);
                }
            }
        }
        let n = 2 * catalogue::CATALOGUE.len();
        let total = all.elapsed().as_secs_f64() * 1e3;
        println!(
            "{mode:?}: {n} thumbnails in {total:.1} ms ({:.2} ms each), slowest {:?} {:.1} ms",
            total / n as f64,
            slowest.1,
            slowest.0
        );
    }
}

/// Times a full solve of a whole picture per method, to check the catalogue's *slow*
/// badges (over 200 ms natively):
/// `cargo test -p convert64-core --release --features parallel -- --ignored --nocapture full_solve_timing`.
#[test]
#[ignore = "timing, run by hand"]
fn full_solve_timing() {
    use std::time::Instant;
    let pal = Palette::default();
    let s = Solver::new(&pal);
    for mode in [Mode::MulticolorBitmap, Mode::HiresBitmap] {
        let frame = photo_frame(mode);
        let mut slow = Vec::new();
        for info in &catalogue::CATALOGUE {
            let mut f = frame.clone();
            let r = Recipe::with_method(info.kind, 1.0).unwrap();
            f.attributes_mut().set_recipes(vec![r], Vec::new());
            let t = Instant::now();
            let _ = s.solve(&f, &Steering::new(), None, &SolveOptions::default());
            let ms = t.elapsed().as_secs_f64() * 1e3;
            println!("{mode:?} {:<20} {ms:7.1} ms", info.id());
            if ms > 200.0 {
                slow.push(info.kind);
            }
        }
        println!("{mode:?}: over 200 ms: {slow:?}");
    }
}

/// A recipe change that blends the same (here the method) reaches the frame the solver
/// reads, and the cells it governs are dirty: the picker's live preview and a committed
/// method both depend on it (#260; before, the frame kept the old recipe).
#[test]
fn a_method_change_reaches_the_frame_and_dirties_its_cells() {
    let px: Vec<[f32; 4]> = (0..64 * 40).map(|_| [0.5, 0.5, 0.5, 1.0]).collect();
    let mut l = Layer::new("g", Arc::new(SourceImage::from_linear(64, 40, px).unwrap()));
    let mut c = Compositor::new(SuperGrid::new(Mode::MulticolorBitmap, 1).unwrap());
    let all = c.update(std::slice::from_ref(&l), &[]);
    assert!(
        c.update(std::slice::from_ref(&l), &[]).is_empty(),
        "nothing changed"
    );
    l.set_recipe(Recipe::with_method(DitherKind::Bayer4, 1.0).unwrap());
    let cells = c.update(std::slice::from_ref(&l), &[]);
    let r = c.frame().attributes().recipe(AttrSource::Layer(0)).unwrap();
    assert_eq!(r.method().kind, DitherKind::Bayer4);
    assert_eq!(cells, all, "every cell the layer covers");
}
