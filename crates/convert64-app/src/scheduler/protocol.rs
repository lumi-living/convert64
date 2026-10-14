//! The solve worker's messages (#265): what the app sends the web worker and what comes
//! back, as bytes, so a message is one `ArrayBuffer` transferred between threads (no
//! `SharedArrayBuffer`, so static hosting needs no COOP/COEP headers).
//!
//! Every message starts with [`MAGIC`], the [`VERSION`], its [`Kind`] and the
//! submission's ticket. A [`Kind::Solve`] request then carries a small JSON header (the
//! grid and the core's serde types: recipes, pins, solver, divergences, background)
//! and a binary body: the composite's colour, coverage and attribute map, locks,
//! allow/deny lists, the palette, the other options, the modes and the previous
//! solution. A [`Kind::Result`] carries one solution. Floats travel as their bits, and
//! the JSON header writes each `f32` through `f64`, so the worker solves exactly the
//! frame the app composited and the app gets exactly the worker's solution.

use convert64_core::colour::{ColourIndex, ColourSet};
use convert64_core::composite::{AttrSource, TargetAttr};
use convert64_core::layer::Recipe;
use convert64_core::mode::Mode;
use convert64_core::model::BitmapImage;
use convert64_core::palette::Rgb8;
use convert64_core::solve::report::Dropped;
use convert64_core::solve::{
    CellReport, ClashReport, InputOptions, Pin, Solution, SolveMode, SolveOptions, SolverKind,
    Steering,
};
use convert64_core::{AttributeMap, CellPos, CellSet, Palette, SuperGrid, TargetFrame};
use serde_json::{Value, json};

/// A serde value as JSON. Through [`Value`], an `f32` is stored as the exact `f64` it
/// widens to, so it reads back bit for bit. (A macro, so the app needs no `serde`.)
macro_rules! to_json {
    ($v:expr) => {
        serde_json::to_value($v).expect("the core's serde types serialise")
    };
}

/// Reads header field `$name` of `$head` as the type the context wants.
macro_rules! from_json {
    ($head:expr, $name:expr) => {
        serde_json::from_value($head.get($name).cloned().unwrap_or(Value::Null))
            .or_else(|e| err(format!("{}: {e}", $name)))
    };
}

/// The first bytes of every message.
pub const MAGIC: [u8; 4] = *b"C64W";
/// The protocol version; both ends are the same build, so this only catches a stale
/// cached worker script.
pub const VERSION: u8 = 1;

/// What a message is.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    /// Worker → app: started, ready for requests (ticket 0).
    Ready = 0,
    /// App → worker: solve a submission; supersedes every earlier one.
    Solve = 1,
    /// App → worker: drop the current submission.
    Cancel = 2,
    /// Worker → app: a solution.
    Result = 3,
    /// Worker → app: nothing left to do; the ticket is the newest it has seen.
    Idle = 4,
}

/// A malformed message.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ProtocolError(pub String);

impl std::fmt::Display for ProtocolError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "solve worker message: {}", self.0)
    }
}

impl std::error::Error for ProtocolError {}

type Res<T> = Result<T, ProtocolError>;

fn err<T>(what: impl Into<String>) -> Res<T> {
    Err(ProtocolError(what.into()))
}

/// A message's kind and ticket.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Header {
    /// What it is.
    pub kind: Kind,
    /// The submission it belongs to (0 for [`Kind::Ready`]).
    pub ticket: u64,
}

/// A solve request, decoded (in the worker).
#[derive(Clone, Debug, PartialEq)]
pub struct SolveRequest {
    /// The submission.
    pub ticket: u64,
    /// The composite to solve.
    pub frame: TargetFrame,
    /// Pins, locks and allow/deny lists.
    pub steering: Steering,
    /// The palette.
    pub palette: Palette,
    /// The options (`layer_origins` filled in; `mode` is ignored).
    pub options: SolveOptions,
    /// Run the fast path with these dirty cells first and deliver it as provisional.
    pub preview: Option<CellSet>,
    /// The submitted solve.
    pub mode: SolveMode,
    /// The solution on screen.
    pub previous: Option<Solution>,
}

/// A solution from the worker, decoded (in the app).
#[derive(Clone, Debug, PartialEq)]
pub struct SolveReply {
    /// The submission.
    pub ticket: u64,
    /// A full submission's fast-path preview; the full result follows.
    pub provisional: bool,
    /// The solve that ran.
    pub mode: SolveMode,
    /// How long it took in the worker, ms.
    pub solve_ms: f64,
    /// The picture.
    pub solution: Solution,
}

/// Reads a message's kind and ticket.
///
/// # Errors
/// If the message is not one of this build's.
pub fn header(msg: &[u8]) -> Res<Header> {
    let mut r = Reader::new(msg);
    r.header()
}

/// The [`Kind::Ready`] message.
pub fn encode_ready() -> Vec<u8> {
    Writer::message(Kind::Ready, 0).0
}

/// The [`Kind::Idle`] message of a worker whose newest ticket is `ticket`.
pub fn encode_idle(ticket: u64) -> Vec<u8> {
    Writer::message(Kind::Idle, ticket).0
}

/// A [`Kind::Cancel`] message for `ticket`.
pub fn encode_cancel(ticket: u64) -> Vec<u8> {
    Writer::message(Kind::Cancel, ticket).0
}

/// A [`Kind::Solve`] request.
#[expect(
    clippy::too_many_arguments,
    reason = "the parts of a request, borrowed from the job and the engine"
)]
pub fn encode_solve(
    ticket: u64,
    frame: &TargetFrame,
    steering: &Steering,
    palette: &Palette,
    options: &SolveOptions,
    preview: Option<CellSet>,
    mode: SolveMode,
    previous: Option<&Solution>,
) -> Vec<u8> {
    let attrs = frame.attributes();
    let pins: Vec<Value> = steering
        .pins()
        .map(|(cell, pin)| json!([to_json!(cell), to_json!(pin)]))
        .collect();
    let SolveOptions {
        solver,
        divergences,
        mode: _,
        background,
        luma_pair_bias,
        refinements,
        finalists,
        hysteresis,
        layer_origins,
        input,
        refine,
    } = options;
    let head = json!({
        "grid": to_json!(frame.grid()),
        "layers": to_json!(attrs.layer_recipes()),
        "regions": to_json!(attrs.region_recipes()),
        "pins": pins,
        "solver": to_json!(solver),
        "divergences": to_json!(divergences),
        "background": to_json!(background),
    });
    let head = serde_json::to_vec(&head).expect("JSON values always serialise");

    let mut w = Writer::message(Kind::Solve, ticket);
    w.bytes(&head);
    // The frame.
    w.bool(frame.prepared());
    let px = frame.colour().len();
    w.u32(len32(px));
    w.0.reserve(px * 22);
    for c in frame.colour() {
        for v in c {
            w.f32(*v);
        }
    }
    for v in frame.coverage() {
        w.f32(*v);
    }
    for a in attrs.pixels() {
        let TargetAttr { source, strength } = *a;
        w.u16(source_code(source));
        w.f32(strength);
    }
    // Steering besides the pins.
    w.cells(steering.locks());
    let lists = |w: &mut Writer, get: &dyn Fn(CellPos) -> ColourSet, default: ColourSet| {
        let cells: Vec<(CellPos, ColourSet)> = all_cells()
            .map(|c| (c, get(c)))
            .filter(|(_, s)| *s != default)
            .collect();
        w.u32(len32(cells.len()));
        for (c, s) in cells {
            w.u16(c.x);
            w.u16(c.y);
            w.u16(s.bits());
        }
    };
    lists(&mut w, &|c| steering.allow(c), ColourSet::ALL);
    lists(&mut w, &|c| steering.deny(c), ColourSet::EMPTY);
    // The palette.
    w.str(palette.name());
    for c in palette.rgb_table() {
        let Rgb8 { r, g, b } = *c;
        w.0.extend_from_slice(&[r, g, b]);
    }
    // The options not in the header.
    w.f32(*luma_pair_bias);
    w.bool(*refine);
    w.u8(*refinements);
    w.u8(*finalists);
    w.opt_f32(*hysteresis);
    w.u32(len32(layer_origins.len()));
    for (x, y) in layer_origins {
        w.i64(*x);
        w.i64(*y);
    }
    let InputOptions { luma_weight } = input;
    w.f32(*luma_weight);
    // The modes and the previous solution.
    match preview {
        Some(dirty) => {
            w.bool(true);
            w.cells(dirty);
        }
        None => w.bool(false),
    }
    w.mode(mode);
    match previous {
        Some(s) => {
            w.bool(true);
            w.solution(s);
        }
        None => w.bool(false),
    }
    w.0
}

/// Decodes a [`Kind::Solve`] request.
///
/// # Errors
/// If `msg` is not a well-formed solve request of this build.
pub fn decode_solve(msg: &[u8]) -> Res<SolveRequest> {
    let mut r = Reader::new(msg);
    let Header { kind, ticket } = r.header()?;
    if kind != Kind::Solve {
        return err(format!("expected a solve request, got {kind:?}"));
    }
    let head: Value = serde_json::from_slice(r.bytes()?)
        .or_else(|e| err(format!("the JSON header does not parse: {e}")))?;
    let grid: SuperGrid = from_json!(head, "grid")?;
    let layers: Vec<Recipe> = from_json!(head, "layers")?;
    let regions: Vec<Recipe> = from_json!(head, "regions")?;
    let pins: Vec<(CellPos, Pin)> = from_json!(head, "pins")?;
    let solver: SolverKind = from_json!(head, "solver")?;
    let divergences = from_json!(head, "divergences")?;
    let background = from_json!(head, "background")?;

    let prepared = r.bool()?;
    let px = r.u32()? as usize;
    let (tw, th) = grid.target_size();
    if px != tw as usize * th as usize {
        return err(format!("{px} pixels for a {tw} × {th} target"));
    }
    let mut colour = Vec::with_capacity(px);
    for _ in 0..px {
        colour.push([r.f32()?, r.f32()?, r.f32()?]);
    }
    let mut coverage = Vec::with_capacity(px);
    for _ in 0..px {
        coverage.push(r.f32()?);
    }
    let mut attrs = Vec::with_capacity(px);
    for _ in 0..px {
        let source = source_from_code(r.u16()?);
        attrs.push(TargetAttr {
            source,
            strength: r.f32()?,
        });
    }
    let attributes = AttributeMap::from_parts(grid, layers, regions, attrs);
    let frame = TargetFrame::from_parts(grid, colour, coverage, attributes, prepared);

    let mut steering = Steering::new();
    for (cell, pin) in pins {
        steering.set_pin(cell, pin);
    }
    for cell in r.cells()?.iter() {
        steering.set_lock(cell, true);
    }
    for allow in [true, false] {
        for _ in 0..r.u32()? {
            let cell = r.cell()?;
            let set = ColourSet::from_bits(r.u16()?);
            if allow {
                steering.set_allow(cell, set);
            } else {
                steering.set_deny(cell, set);
            }
        }
    }

    let name = r.str()?;
    let mut rgb = [Rgb8::new(0, 0, 0); 16];
    for c in &mut rgb {
        *c = Rgb8::new(r.u8()?, r.u8()?, r.u8()?);
    }
    let palette = Palette::from_rgb(name, rgb);

    let luma_pair_bias = r.f32()?;
    let refine = r.bool()?;
    let refinements = r.u8()?;
    let finalists = r.u8()?;
    let hysteresis = r.opt_f32()?;
    let mut layer_origins = Vec::new();
    for _ in 0..r.u32()? {
        layer_origins.push((r.i64()?, r.i64()?));
    }
    let input = InputOptions {
        luma_weight: r.f32()?,
    };
    let options = SolveOptions {
        solver,
        divergences,
        mode: SolveMode::Full,
        background,
        luma_pair_bias,
        refinements,
        finalists,
        hysteresis,
        layer_origins,
        input,
        refine,
    };

    let preview = if r.bool()? { Some(r.cells()?) } else { None };
    let mode = r.mode()?;
    let previous = if r.bool()? { Some(r.solution()?) } else { None };
    r.end()?;
    Ok(SolveRequest {
        ticket,
        frame,
        steering,
        palette,
        options,
        preview,
        mode,
        previous,
    })
}

/// A [`Kind::Result`] message.
pub fn encode_result(
    ticket: u64,
    provisional: bool,
    mode: SolveMode,
    solve_ms: f64,
    solution: &Solution,
) -> Vec<u8> {
    let mut w = Writer::message(Kind::Result, ticket);
    w.bool(provisional);
    w.mode(mode);
    w.0.extend_from_slice(&solve_ms.to_bits().to_le_bytes());
    w.solution(solution);
    w.0
}

/// Decodes a [`Kind::Result`] message.
///
/// # Errors
/// If `msg` is not a well-formed result of this build.
pub fn decode_result(msg: &[u8]) -> Res<SolveReply> {
    let mut r = Reader::new(msg);
    let Header { kind, ticket } = r.header()?;
    if kind != Kind::Result {
        return err(format!("expected a result, got {kind:?}"));
    }
    let provisional = r.bool()?;
    let mode = r.mode()?;
    let solve_ms = f64::from_bits(r.u64()?);
    let solution = r.solution()?;
    r.end()?;
    Ok(SolveReply {
        ticket,
        provisional,
        mode,
        solve_ms,
        solution,
    })
}

fn all_cells() -> impl Iterator<Item = CellPos> {
    (0..CellSet::ROWS).flat_map(|y| (0..CellSet::COLS).map(move |x| CellPos::new(x, y)))
}

const NO_SOURCE: u16 = u16::MAX;
const REGION_BIT: u16 = 0x8000;

fn source_code(s: AttrSource) -> u16 {
    match s {
        AttrSource::None => NO_SOURCE,
        AttrSource::Layer(i) => i,
        AttrSource::Region(i) => REGION_BIT | i,
    }
}

fn source_from_code(c: u16) -> AttrSource {
    if c == NO_SOURCE {
        AttrSource::None
    } else if c & REGION_BIT != 0 {
        AttrSource::Region(c & !REGION_BIT)
    } else {
        AttrSource::Layer(c)
    }
}

fn len32(n: usize) -> u32 {
    u32::try_from(n).expect("solve messages are far below 4 GB")
}

/// Little-endian writer.
struct Writer(Vec<u8>);

impl Writer {
    fn message(kind: Kind, ticket: u64) -> Self {
        let mut w = Self(Vec::new());
        w.0.extend_from_slice(&MAGIC);
        w.u8(VERSION);
        w.u8(kind as u8);
        w.u64(ticket);
        w
    }

    fn u8(&mut self, v: u8) {
        self.0.push(v);
    }

    fn bool(&mut self, v: bool) {
        self.u8(u8::from(v));
    }

    fn u16(&mut self, v: u16) {
        self.0.extend_from_slice(&v.to_le_bytes());
    }

    fn u32(&mut self, v: u32) {
        self.0.extend_from_slice(&v.to_le_bytes());
    }

    fn u64(&mut self, v: u64) {
        self.0.extend_from_slice(&v.to_le_bytes());
    }

    fn i64(&mut self, v: i64) {
        self.0.extend_from_slice(&v.to_le_bytes());
    }

    fn f32(&mut self, v: f32) {
        self.u32(v.to_bits());
    }

    fn opt_f32(&mut self, v: Option<f32>) {
        match v {
            Some(v) => {
                self.bool(true);
                self.f32(v);
            }
            None => self.bool(false),
        }
    }

    /// Length-prefixed bytes.
    fn bytes(&mut self, b: &[u8]) {
        self.u32(len32(b.len()));
        self.0.extend_from_slice(b);
    }

    fn str(&mut self, s: &str) {
        self.bytes(s.as_bytes());
    }

    /// A cell set as 1000 bits, row-major.
    fn cells(&mut self, set: CellSet) {
        let mut bits = [0u8; 125];
        for (i, c) in all_cells().enumerate() {
            if set.contains(c) {
                bits[i / 8] |= 1 << (i % 8);
            }
        }
        self.0.extend_from_slice(&bits);
    }

    fn mode(&mut self, mode: SolveMode) {
        match mode {
            SolveMode::Full => self.u8(0),
            SolveMode::Interactive { dirty } => {
                self.u8(1);
                self.cells(dirty);
            }
        }
    }

    fn colour(&mut self, c: ColourIndex) {
        self.u8(c.get());
    }

    fn opt_colour(&mut self, c: Option<ColourIndex>) {
        self.u8(c.map_or(u8::MAX, ColourIndex::get));
    }

    fn sets(&mut self, sets: &[ColourSet]) {
        self.u32(len32(sets.len()));
        for s in sets {
            self.u16(s.bits());
        }
    }

    fn solution(&mut self, s: &Solution) {
        let Solution {
            image,
            background,
            sets,
            error,
            refined,
            rounds,
            report,
            solver,
            fallback,
        } = s;
        match image.mode() {
            Mode::MulticolorBitmap => self.u8(0),
            Mode::HiresBitmap => self.u8(1),
            other => unreachable!("a solution is a bitmap, not {other:?}"),
        }
        self.bytes(image.bitmap());
        self.bytes(&image.screen_bytes());
        self.bytes(&image.colour_bytes());
        self.opt_colour(image.background());
        self.opt_colour(*background);
        self.sets(sets);
        self.f32(*error);
        self.u8(*refined);
        self.u32(len32(rounds.len()));
        for r in rounds {
            self.sets(r);
        }
        let ClashReport { cells } = report;
        self.u32(len32(cells.len()));
        for c in cells {
            let CellReport {
                cell,
                wanted,
                chosen,
                dropped,
                error,
                luma_cross,
            } = c;
            self.u16(cell.x);
            self.u16(cell.y);
            self.u32(len32(wanted.len()));
            for (colour, weight) in wanted {
                self.colour(*colour);
                self.f32(*weight);
            }
            self.u16(chosen.bits());
            self.u32(len32(dropped.len()));
            for d in dropped {
                let Dropped {
                    colour,
                    weight,
                    cost,
                } = *d;
                self.colour(colour);
                self.f32(weight);
                self.f32(cost);
            }
            self.f32(*error);
            self.bool(*luma_cross);
        }
        self.u8(match solver {
            SolverKind::Reference => 0,
            SolverKind::Native => 1,
            SolverKind::Png2c64 => 2,
        });
        match fallback {
            Some(f) => {
                self.bool(true);
                self.str(f);
            }
            None => self.bool(false),
        }
    }
}

/// Little-endian reader.
struct Reader<'a> {
    buf: &'a [u8],
    pos: usize,
}

impl<'a> Reader<'a> {
    const fn new(buf: &'a [u8]) -> Self {
        Self { buf, pos: 0 }
    }

    fn take(&mut self, n: usize) -> Res<&'a [u8]> {
        match self.buf.get(self.pos..self.pos.saturating_add(n)) {
            Some(b) => {
                self.pos += n;
                Ok(b)
            }
            None => err(format!("truncated at byte {}", self.pos)),
        }
    }

    fn array<const N: usize>(&mut self) -> Res<[u8; N]> {
        Ok(self.take(N)?.try_into().expect("took N bytes"))
    }

    fn header(&mut self) -> Res<Header> {
        if self.array::<4>()? != MAGIC {
            return err("not a convert64 solve worker message");
        }
        let version = self.u8()?;
        if version != VERSION {
            return err(format!("version {version}, expected {VERSION}"));
        }
        let kind = match self.u8()? {
            0 => Kind::Ready,
            1 => Kind::Solve,
            2 => Kind::Cancel,
            3 => Kind::Result,
            4 => Kind::Idle,
            k => return err(format!("unknown kind {k}")),
        };
        Ok(Header {
            kind,
            ticket: self.u64()?,
        })
    }

    fn end(&self) -> Res<()> {
        if self.pos == self.buf.len() {
            Ok(())
        } else {
            err(format!("{} bytes left over", self.buf.len() - self.pos))
        }
    }

    fn u8(&mut self) -> Res<u8> {
        Ok(self.array::<1>()?[0])
    }

    fn bool(&mut self) -> Res<bool> {
        match self.u8()? {
            0 => Ok(false),
            1 => Ok(true),
            b => err(format!("{b} is not a bool")),
        }
    }

    fn u16(&mut self) -> Res<u16> {
        Ok(u16::from_le_bytes(self.array()?))
    }

    fn u32(&mut self) -> Res<u32> {
        Ok(u32::from_le_bytes(self.array()?))
    }

    fn u64(&mut self) -> Res<u64> {
        Ok(u64::from_le_bytes(self.array()?))
    }

    fn i64(&mut self) -> Res<i64> {
        Ok(i64::from_le_bytes(self.array()?))
    }

    fn f32(&mut self) -> Res<f32> {
        Ok(f32::from_bits(self.u32()?))
    }

    fn opt_f32(&mut self) -> Res<Option<f32>> {
        Ok(if self.bool()? {
            Some(self.f32()?)
        } else {
            None
        })
    }

    fn bytes(&mut self) -> Res<&'a [u8]> {
        let n = self.u32()? as usize;
        self.take(n)
    }

    fn str(&mut self) -> Res<String> {
        String::from_utf8(self.bytes()?.to_vec()).or_else(|_| err("a string is not UTF-8"))
    }

    fn cell(&mut self) -> Res<CellPos> {
        let (x, y) = (self.u16()?, self.u16()?);
        if x >= CellSet::COLS || y >= CellSet::ROWS {
            return err(format!("cell ({x}, {y}) is off the screen"));
        }
        Ok(CellPos::new(x, y))
    }

    fn cells(&mut self) -> Res<CellSet> {
        let bits = self.array::<125>()?;
        let mut set = CellSet::EMPTY;
        for (i, c) in all_cells().enumerate() {
            if bits[i / 8] & (1 << (i % 8)) != 0 {
                set.insert(c);
            }
        }
        Ok(set)
    }

    fn mode(&mut self) -> Res<SolveMode> {
        match self.u8()? {
            0 => Ok(SolveMode::Full),
            1 => Ok(SolveMode::Interactive {
                dirty: self.cells()?,
            }),
            m => err(format!("unknown solve mode {m}")),
        }
    }

    fn colour(&mut self) -> Res<ColourIndex> {
        let c = self.u8()?;
        ColourIndex::new(c).or_else(|_| err(format!("{c} is not a colour")))
    }

    fn opt_colour(&mut self) -> Res<Option<ColourIndex>> {
        match self.u8()? {
            u8::MAX => Ok(None),
            c => ColourIndex::new(c)
                .map(Some)
                .or_else(|_| err(format!("{c} is not a colour"))),
        }
    }

    fn sets(&mut self) -> Res<Vec<ColourSet>> {
        let n = self.u32()?;
        (0..n)
            .map(|_| Ok(ColourSet::from_bits(self.u16()?)))
            .collect()
    }

    fn solution(&mut self) -> Res<Solution> {
        let mode = self.u8()?;
        let bitmap = self.bytes()?;
        let screen = self.bytes()?;
        let colour = self.bytes()?;
        let image_bg = self.opt_colour()?;
        let image = match (mode, image_bg) {
            (0, Some(bg)) => BitmapImage::multicolor_from_bytes(bitmap, screen, colour, bg.get()),
            (1, None) => BitmapImage::hires_from_bytes(bitmap, screen),
            _ => return err("a solution's image is neither multicolor nor hi-res"),
        }
        .or_else(|e| err(format!("a solution's image: {e}")))?;
        let background = self.opt_colour()?;
        let sets = self.sets()?;
        let error = self.f32()?;
        let refined = self.u8()?;
        let mut rounds = Vec::new();
        for _ in 0..self.u32()? {
            rounds.push(self.sets()?);
        }
        let mut cells = Vec::new();
        for _ in 0..self.u32()? {
            let cell = self.cell()?;
            let mut wanted = Vec::new();
            for _ in 0..self.u32()? {
                wanted.push((self.colour()?, self.f32()?));
            }
            let chosen = ColourSet::from_bits(self.u16()?);
            let mut dropped = Vec::new();
            for _ in 0..self.u32()? {
                dropped.push(Dropped {
                    colour: self.colour()?,
                    weight: self.f32()?,
                    cost: self.f32()?,
                });
            }
            cells.push(CellReport {
                cell,
                wanted,
                chosen,
                dropped,
                error: self.f32()?,
                luma_cross: self.bool()?,
            });
        }
        let solver = match self.u8()? {
            0 => SolverKind::Reference,
            1 => SolverKind::Native,
            2 => SolverKind::Png2c64,
            s => return err(format!("unknown solver {s}")),
        };
        let fallback = if self.bool()? {
            Some(self.str()?)
        } else {
            None
        };
        Ok(Solution {
            image,
            background,
            sets,
            error,
            refined,
            rounds,
            report: ClashReport { cells },
            solver,
            fallback,
        })
    }
}
