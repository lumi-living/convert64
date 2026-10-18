//! The scheduler without the app: supersession, cancel, fast path then full search, and
//! output identical to the core called directly, for both drivers.

use super::*;
use convert64_core::geom::Vec2;
use convert64_core::layer::{DitherKind, Recipe, SourceImage, Transform};
use convert64_core::mode::Mode;
use convert64_core::solve::{Pin, Solver};
use convert64_core::{CellPos, CellSet, ColourSet, Compositor};
use std::time::Duration;

mod remote;

const WAIT: Duration = Duration::from_secs(120);

/// A colourful full-screen background and a small square on top at `x`.
fn layers(x: f32) -> Vec<Layer> {
    let (w, h) = (80u32, 50u32);
    let mut px = Vec::with_capacity((w * h * 4) as usize);
    for y in 0..h {
        for x in 0..w {
            px.extend_from_slice(&[
                (x * 255 / w) as u8,
                (y * 255 / h) as u8,
                ((x + y) * 3 % 256) as u8,
                255,
            ]);
        }
    }
    let bg = Arc::new(SourceImage::from_srgba8(w, h, &px).unwrap());
    let mut back = Layer::new("back", bg);
    back.set_transform(
        Transform::new(Vec2::new(0.0, 0.0), Vec2::new(4.0, 4.0), false, false).unwrap(),
    );
    back.set_recipe(Recipe::with_method(DitherKind::Bayer4, 0.8).unwrap());
    let sq = Arc::new(SourceImage::from_srgba8(1, 1, &[250, 220, 40, 255]).unwrap());
    let mut square = Layer::new("square", sq);
    square.set_transform(
        Transform::new(Vec2::new(x, 64.0), Vec2::new(24.0, 24.0), false, false).unwrap(),
    );
    vec![back, square]
}

fn snapshot(x: f32, previous: Option<&SolveResult>) -> Snapshot {
    Snapshot {
        layers: layers(x),
        regions: Vec::new(),
        steering: Steering::new(),
        grid: SuperGrid::new(Mode::MulticolorBitmap, 2).unwrap(),
        palette: Palette::pepto_pal(),
        options: SolveOptions::default(),
        previous: previous.map(|r| Arc::clone(&r.solution)),
    }
}

/// The core called directly, as the CLI does: one compositor across the snapshots.
struct Direct {
    compositor: Compositor,
    solver: Solver,
}

impl Direct {
    fn new() -> Self {
        Self {
            compositor: Compositor::new(SuperGrid::new(Mode::MulticolorBitmap, 2).unwrap()),
            solver: Solver::new(&Palette::pepto_pal()),
        }
    }

    /// Composites `s`; returns the changed cells.
    fn update(&mut self, s: &Snapshot) -> CellSet {
        self.compositor.update(&s.layers, &s.regions)
    }

    fn solve(&self, s: &Snapshot, mode: SolveMode) -> Solution {
        let opts = SolveOptions {
            mode,
            layer_origins: convert64_core::solve::dither::layer_origins(s.grid, &s.layers),
            ..s.options.clone()
        };
        self.solver.solve(
            self.compositor.frame(),
            &s.steering,
            s.previous.as_deref(),
            &opts,
        )
    }
}

fn stepper(preview_full: bool) -> Stepper {
    Stepper::new(Config {
        preview_full,
        ..Config::default()
    })
}

/// Steps to the end, polling after every stage.
fn drain(s: &mut Stepper) -> Vec<SolveResult> {
    let mut out = Vec::new();
    loop {
        let more = s.step();
        out.extend(s.poll());
        if !more {
            return out;
        }
    }
}

fn interactive() -> SolveMode {
    SolveMode::Interactive {
        dirty: CellSet::EMPTY,
    }
}

#[test]
fn stepping_output_is_the_cores() {
    let mut s = stepper(false);
    let mut direct = Direct::new();

    let s1 = snapshot(40.0, None);
    s.submit(s1.clone(), SolveMode::Full);
    let r1 = s.finish().unwrap();
    direct.update(&s1);
    assert_eq!(*r1.solution, direct.solve(&s1, SolveMode::Full));
    assert_eq!(r1.mode, SolveMode::Full);
    assert!(!r1.provisional);
    assert_eq!(r1.solver(), SolverKind::Reference);
    assert_eq!(r1.fallback(), None);
    assert!(r1.timing.composite_ms > 0.0 && r1.timing.solve_ms > 0.0);

    // A 0.25 px nudge of the square: the fast path re-solves the cells it touched.
    let s2 = snapshot(40.25, Some(&r1));
    s.submit(s2.clone(), interactive());
    let r2 = s.finish().unwrap();
    let changed = direct.update(&s2);
    assert!(
        !changed.is_empty() && changed.len() < 100,
        "{}",
        changed.len()
    );
    let dirty = SolveMode::Interactive { dirty: changed };
    assert_eq!(r2.mode, dirty);
    assert_eq!(*r2.solution, direct.solve(&s2, dirty));
}

#[test]
fn worker_output_is_the_cores() {
    let mut s = Scheduler::new(Config::default());
    let mut direct = Direct::new();

    let s1 = snapshot(40.0, None);
    let t1 = s.submit(s1.clone(), SolveMode::Full);
    let r1 = wait(&s).unwrap();
    assert_eq!(r1.ticket, t1);
    direct.update(&s1);
    assert_eq!(*r1.solution, direct.solve(&s1, SolveMode::Full));

    let s2 = snapshot(40.25, Some(&r1));
    let t2 = s.submit(s2.clone(), interactive());
    let r2 = wait(&s).unwrap();
    assert_eq!(r2.ticket, t2);
    let dirty = SolveMode::Interactive {
        dirty: direct.update(&s2),
    };
    assert_eq!(r2.mode, dirty);
    assert_eq!(*r2.solution, direct.solve(&s2, dirty));
}

/// Waits for the worker's next result.
fn wait(s: &Scheduler) -> Option<SolveResult> {
    match &s.driver {
        Driver::Thread(w) => w.wait(WAIT),
        Driver::Steps(_) => unreachable!("a worker"),
    }
}

fn worker(s: &Scheduler) -> &Worker {
    match &s.driver {
        Driver::Thread(w) => w,
        Driver::Steps(_) => unreachable!("a worker"),
    }
}

#[test]
fn stepping_delivers_only_the_newest_submission() {
    let mut s = stepper(false);
    // Superseded between stages: the old job's solve never runs.
    s.submit(snapshot(40.0, None), SolveMode::Full);
    assert!(s.step());
    let b = s.submit(snapshot(48.0, None), SolveMode::Full);
    let got = drain(&mut s);
    assert_eq!(got.len(), 1);
    assert_eq!(got[0].ticket, b);

    // Superseded after its result was made but before it was taken: never delivered.
    s.submit(snapshot(56.0, None), SolveMode::Full);
    while s.step() {}
    let d = s.submit(snapshot(64.0, None), SolveMode::Full);
    assert!(s.poll().is_none());
    let got = drain(&mut s);
    assert_eq!(got.iter().map(|r| r.ticket).collect::<Vec<_>>(), [d]);
}

#[test]
fn worker_delivers_only_the_newest_submission() {
    let mut s = Scheduler::new(Config::default());
    let mut direct = Direct::new();
    s.submit(snapshot(40.0, None), SolveMode::Full);
    s.submit(snapshot(44.0, None), SolveMode::Full);
    let s3 = snapshot(48.0, None);
    let t3 = s.submit(s3.clone(), SolveMode::Full);
    assert!(worker(&s).wait_idle(WAIT));
    let r = s.poll().expect("the newest submission's result");
    assert_eq!(r.ticket, t3);
    assert!(s.poll().is_none());
    direct.update(&s3);
    assert_eq!(*r.solution, direct.solve(&s3, SolveMode::Full));

    // A finished result not yet taken is dropped by a newer submission.
    let t5 = s.submit(snapshot(52.0, None), SolveMode::Full);
    assert!(worker(&s).wait_idle(WAIT));
    let t6 = s.submit(snapshot(56.0, None), SolveMode::Full);
    assert!(t6 > t5);
    if let Some(r) = s.poll() {
        assert_eq!(r.ticket, t6);
    }
    assert!(worker(&s).wait_idle(WAIT));
    assert!(s.poll().is_none_or(|r| r.ticket == t6));
}

#[test]
fn cancel_delivers_nothing() {
    let mut s = Scheduler::stepping(Config::default());
    s.submit(snapshot(40.0, None), SolveMode::Full);
    assert!(s.tick());
    s.cancel();
    assert!(!s.is_busy());
    assert!(!s.tick());
    assert!(s.poll().is_none());

    let mut w = Scheduler::new(Config::default());
    w.submit(snapshot(40.0, None), SolveMode::Full);
    w.cancel();
    assert!(worker(&w).wait_idle(WAIT));
    assert!(w.poll().is_none());
    // Still usable.
    let t = w.submit(snapshot(40.0, None), SolveMode::Full);
    assert_eq!(wait(&w).map(|r| r.ticket), Some(t));
}

#[test]
fn a_full_solve_previews_the_fast_path_then_delivers_the_full_search() {
    let mut s = stepper(true);
    let mut direct = Direct::new();
    let s1 = snapshot(40.0, None);
    s.submit(s1.clone(), SolveMode::Full);
    let r1 = s.finish().unwrap();
    direct.update(&s1);

    // A drag (fast path), then the release (full search) before the drag's solve ran.
    let s2 = snapshot(41.0, Some(&r1));
    s.submit(s2.clone(), interactive());
    assert!(s.step());
    let s3 = snapshot(42.0, Some(&r1));
    let t3 = s.submit(s3.clone(), SolveMode::Full);
    let got = drain(&mut s);
    assert_eq!(got.len(), 2);
    assert!(got.iter().all(|r| r.ticket == t3));
    let (preview, full) = (&got[0], &got[1]);

    // The drag's composite was never solved, so its cells count as changed too.
    let dirty = direct.update(&s2).union(direct.update(&s3));
    assert!(preview.provisional);
    assert_eq!(preview.mode, SolveMode::Interactive { dirty });
    assert_eq!(*preview.solution, direct.solve(&s3, preview.mode));
    assert!(!full.provisional);
    assert_eq!(full.mode, SolveMode::Full);
    assert_eq!(*full.solution, direct.solve(&s3, SolveMode::Full));
}

#[test]
fn the_worker_ends_a_drag_with_the_full_search() {
    let mut s = Scheduler::new(Config::default());
    s.submit(snapshot(40.0, None), SolveMode::Full);
    let r1 = wait(&s).unwrap();
    let mut tickets = Vec::new();
    for x in [41.0, 42.0, 43.0] {
        tickets.push(s.submit(snapshot(x, Some(&r1)), interactive()));
    }
    let release = s.submit(snapshot(43.0, Some(&r1)), SolveMode::Full);
    assert!(tickets.iter().all(|t| *t < release));
    assert!(worker(&s).wait_idle(WAIT));
    let last = s.poll().unwrap();
    assert_eq!(last.ticket, release);
    assert_eq!(last.mode, SolveMode::Full);
    assert!(!last.provisional);
}

#[test]
fn the_fast_path_covers_superseded_composites() {
    let mut s = stepper(false);
    let mut direct = Direct::new();
    let s1 = snapshot(40.0, None);
    s.submit(s1.clone(), SolveMode::Full);
    let r1 = s.finish().unwrap();
    direct.update(&s1);

    // The square moves far, then on again; the first move is composited, never solved.
    let s2 = snapshot(160.0, Some(&r1));
    s.submit(s2.clone(), interactive());
    assert!(s.step());
    let s3 = snapshot(240.0, Some(&r1));
    s.submit(s3.clone(), interactive());
    let r3 = s.finish().unwrap();

    let d12 = direct.update(&s2);
    let d23 = direct.update(&s3);
    // The square's first place is only in the first update.
    assert!(!d12.intersection(d23).eq(&d12));
    let mode = SolveMode::Interactive {
        dirty: d12.union(d23),
    };
    assert_eq!(r3.mode, mode);
    assert_eq!(*r3.solution, direct.solve(&s3, mode));
}

#[test]
fn steering_palette_and_foreign_previous_widen_the_fast_path() {
    let mut s = stepper(false);
    s.submit(snapshot(40.0, None), SolveMode::Full);
    let r1 = s.finish().unwrap();

    // A pin: only its cell.
    let mut s2 = snapshot(40.0, Some(&r1));
    let cell = CellPos::new(3, 4);
    s2.steering.set_pin(
        cell,
        Pin {
            colours: ColourSet::from_bits(1 << 2),
            full: false,
        },
    );
    s.submit(s2, interactive());
    let r2 = s.finish().unwrap();
    let mut only = CellSet::EMPTY;
    only.insert(cell);
    assert_eq!(r2.mode, SolveMode::Interactive { dirty: only });

    // A submitted dirty cell is re-solved too.
    let mut extra = CellSet::EMPTY;
    extra.insert(CellPos::new(9, 9));
    let mut s3 = snapshot(40.0, Some(&r2));
    s3.steering = r2_steering(cell);
    s.submit(s3, SolveMode::Interactive { dirty: extra });
    let r3 = s.finish().unwrap();
    assert_eq!(r3.mode, SolveMode::Interactive { dirty: extra });

    // Another palette: every cell.
    let mut s4 = snapshot(40.0, Some(&r3));
    s4.steering = r2_steering(cell);
    s4.palette = Palette::colodore(50.0, 100.0, 50.0);
    s.submit(s4, interactive());
    let r4 = s.finish().unwrap();
    assert_eq!(
        r4.mode,
        SolveMode::Interactive {
            dirty: CellSet::all()
        }
    );

    // A previous solution the scheduler did not make: every cell.
    let mut s5 = snapshot(40.0, None);
    s5.steering = r2_steering(cell);
    s5.palette = Palette::colodore(50.0, 100.0, 50.0);
    s5.previous = Some(Arc::new((*r4.solution).clone()));
    s.submit(s5, interactive());
    let r5 = s.finish().unwrap();
    assert_eq!(
        r5.mode,
        SolveMode::Interactive {
            dirty: CellSet::all()
        }
    );
}

fn r2_steering(cell: CellPos) -> Steering {
    let mut st = Steering::new();
    st.set_pin(
        cell,
        Pin {
            colours: ColourSet::from_bits(1 << 2),
            full: false,
        },
    );
    st
}

#[test]
fn tick_keeps_to_the_frame_budget() {
    // No budget: one stage per frame (composite, then solve).
    let mut s = Stepper::new(Config {
        preview_full: false,
        frame_budget_ms: 0.0,
    });
    s.submit(snapshot(40.0, None), SolveMode::Full);
    assert!(s.tick());
    assert!(s.poll().is_none());
    assert!(!s.tick());
    assert!(s.poll().is_some());

    // A budget that fits every measured stage: the whole submission in one frame.
    let mut s = Stepper::new(Config {
        preview_full: false,
        frame_budget_ms: 1e9,
    });
    s.submit(snapshot(40.0, None), SolveMode::Full);
    while s.tick() {}
    s.poll().unwrap();
    s.submit(snapshot(44.0, None), SolveMode::Full);
    assert!(!s.tick());
    assert!(s.poll().is_some());
}

/// Stage timings of the stepping driver at 4× (the default factor): a full solve, then
/// drags of the square, each followed by its release. Run in release:
/// `cargo test --release -p convert64-app --lib stage_timings -- --ignored --nocapture`.
#[test]
#[ignore = "benchmark: run in release with --ignored --nocapture"]
fn stage_timings() {
    let grid = SuperGrid::new(Mode::MulticolorBitmap, 4).unwrap();
    let snap = |x: f32, previous: Option<&SolveResult>| Snapshot {
        grid,
        ..snapshot(x, previous)
    };
    let mut s = stepper(true);
    s.submit(snap(40.0, None), SolveMode::Full);
    let first = s.finish().unwrap();
    println!(
        "cold: composite {:.1} ms, full solve {:.1} ms",
        first.timing.composite_ms, first.timing.solve_ms
    );
    let mut base = first;
    let (mut comp, mut fast, mut full) = (Vec::new(), Vec::new(), Vec::new());
    for i in 0..9 {
        let x = 40.0 + 0.25 * (i + 1) as f32;
        s.submit(snap(x, Some(&base)), interactive());
        let r = s.finish().unwrap();
        comp.push(r.timing.composite_ms);
        fast.push(r.timing.solve_ms);
        s.submit(snap(x, Some(&r)), SolveMode::Full);
        let got = drain(&mut s);
        full.push(got.last().unwrap().timing.solve_ms);
        base = got.last().unwrap().clone();
    }
    let median = |mut v: Vec<f64>| {
        v.sort_by(f64::total_cmp);
        v[v.len() / 2]
    };
    println!(
        "nudge: composite {:.1} ms, fast path {:.1} ms; release: full solve {:.1} ms",
        median(comp),
        median(fast),
        median(full),
    );
}
