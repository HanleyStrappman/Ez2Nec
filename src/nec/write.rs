//! `write_deck`: an [`EzModel`] to a NEC deck for one target engine.
//!
//! Decoding follows EZNEC: which segment a source, load, line end or port is
//! on, RMS source values, the split-source junction rule, and its formulas for
//! lumped elements. The cards are chosen for the target engine, so that the
//! deck runs as the model states on that engine with nothing else needed.
//! Where an engine has no exact card for something, the deck says so in a
//! `CM` line instead of changing the model silently.

use std::fmt;

use super::complex::C;
use super::elements;
use crate::ez::*;
use crate::rules::{segment_at, split_junction};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Engine {
    Nec2,
    Nec4,
    Nec5,
}

impl Engine {
    pub fn parse(s: &str) -> Option<Engine> {
        match s.to_ascii_lowercase().replace(['-', '.'], "").as_str() {
            "nec2" | "2" => Some(Engine::Nec2),
            "nec4" | "nec42" | "4" => Some(Engine::Nec4),
            "nec5" | "5" => Some(Engine::Nec5),
            _ => None,
        }
    }

    pub fn name(self) -> &'static str {
        match self {
            Engine::Nec2 => "NEC-2",
            Engine::Nec4 => "NEC-4.2",
            Engine::Nec5 => "NEC-5",
        }
    }

    /// The longest input line the engine reads in full. nec2dx reads 80
    /// columns, NEC-4.2 120 and NEC-5 220; past that a value can change with
    /// no error.
    fn line_limit(self) -> usize {
        match self {
            Engine::Nec2 => 80,
            Engine::Nec4 => 120,
            Engine::Nec5 => 220,
        }
    }
}

#[derive(Debug, Clone)]
pub struct Options {
    pub engine: Engine,
    /// The `.ez` file's name, for the deck's comment lines.
    pub source_name: Option<String>,
    /// Write the model's stored frequency sweep instead of its one frequency.
    pub sweep: bool,
}

/// A written deck. `notes` repeats what the deck's `CM ez2nec:` lines say.
#[derive(Debug, Clone, PartialEq)]
pub struct Deck {
    pub text: String,
    pub notes: Vec<String>,
}

/// A model that can't be written for the engine.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConvertError(pub String);

impl fmt::Display for ConvertError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for ConvertError {}

/// Dummy wires (for NEC-2 current sources and for stubs) get tags from here
/// up, far from any real wire. necQt's model checks skip these tags.
const DUMMY_TAG: usize = 9901;
/// A shorted stub's far end: this conductance across the dummy segment.
const STUB_SHORT_S: f64 = 1e10;
const SQRT2: f64 = std::f64::consts::SQRT_2;
/// More frequencies than this in a stored sweep is taken as a mistake.
const MAX_SWEEP: usize = 10_000;

/// Formats a number with at most `sig` significant digits, in the shortest form.
fn number(x: f64, sig: usize) -> String {
    if x == 0.0 {
        return "0".to_string();
    }
    let sci = format!("{:.*e}", sig - 1, x);
    let rounded: f64 = sci.parse().unwrap_or(x);
    let exp: i32 = sci.split('e').nth(1).and_then(|e| e.parse().ok()).unwrap_or(0);
    if (-4..7).contains(&exp) {
        format!("{rounded}")
    } else {
        // Shortest mantissa, Fortran-style exponent.
        let s = format!("{rounded:e}");
        s.replace('e', "E")
    }
}

struct Writer {
    engine: Engine,
    lines: Vec<String>,
    notes: Vec<String>,
}

impl Writer {
    /// Adds a card. Precision drops from 7 digits only if the line would
    /// otherwise pass the engine's limit.
    fn card(&mut self, code: &str, ints: &[i64], floats: &[f64]) -> Result<(), ConvertError> {
        for sig in (5..=7).rev() {
            let mut line = code.to_string();
            for i in ints {
                line.push(' ');
                line.push_str(&i.to_string());
            }
            for &x in floats {
                line.push(' ');
                line.push_str(&number(x, sig));
            }
            if line.len() <= self.engine.line_limit() {
                self.lines.push(line);
                return Ok(());
            }
        }
        Err(ConvertError(format!(
            "a {code} card doesn't fit in {} columns for {}",
            self.engine.line_limit(),
            self.engine.name()
        )))
    }

    fn note(&mut self, text: impl Into<String>) {
        let text = text.into();
        if !self.notes.contains(&text) {
            self.notes.push(text);
        }
    }
}

fn sub(a: [f64; 3], b: [f64; 3]) -> [f64; 3] {
    [a[0] - b[0], a[1] - b[1], a[2] - b[2]]
}

fn norm(a: [f64; 3]) -> f64 {
    (a[0] * a[0] + a[1] * a[1] + a[2] * a[2]).sqrt()
}

fn f64s(p: &[f32; 3]) -> [f64; 3] {
    [p[0] as f64, p[1] as f64, p[2] as f64]
}

struct Geometry<'a> {
    wires: &'a [Wire],
    /// Segments per wire as written to the deck. They differ from the model's
    /// only for NEC-5, on a one-segment wire that carries a source, line or port.
    segs: Vec<u16>,
    /// NEC-5 attaches sources, lines and networks at segment ends.
    at_nodes: bool,
    grounded: bool,
}

/// Where a source, line end or network port attaches.
#[derive(Clone, Copy)]
struct Attach {
    /// The segment number for the card. For NEC-5 it names a segment end: a
    /// positive number is that segment's far end, and −1 is segment 1's near end.
    segment: i64,
    /// The junction (0 to N) for NEC-5, or the segment whose centre it is.
    index: u16,
}

impl Geometry<'_> {
    fn wire(&self, number: u16) -> &Wire {
        &self.wires[number as usize - 1]
    }

    fn segments(&self, number: u16) -> u16 {
        self.segs[number as usize - 1]
    }

    fn segment_length(&self, number: u16) -> f64 {
        let w = self.wire(number);
        norm(sub(f64s(&w.end2), f64s(&w.end1))) / self.segments(number) as f64
    }

    /// True if the wire's end joins another wire or touches a ground plane.
    fn end_connected(&self, number: u16, start: bool) -> bool {
        let w = self.wire(number);
        let point = if start { &w.end1 } else { &w.end2 };
        let on_ground = self.grounded && (point[2] as f64).abs() <= 1e-3 * self.segment_length(number);
        on_ground || !self.wires_at(f64s(point), number).is_empty()
    }

    /// Where something at `pct` percent along a wire attaches.
    ///
    /// On NEC-2 and NEC-4.2 it is the segment holding the point (EZNEC's floor
    /// rule). On NEC-5 it is the segment junction nearest the point, with a
    /// half rounded up. EZNEC's NEC-5 exports confirm it: a source at 50% of
    /// 20 segments is at the far end of segment 10, one at 30% of 21 at the
    /// far end of segment 6 (the floor rule would say 7), and line ends at
    /// 50% of 20 at segment 10. EZNEC breaks exact ties either way (2270ax2).
    /// EZNEC won't put a source on an open wire end with NEC-5, so a
    /// junction there moves one in.
    fn attach(&self, number: u16, pct: f32) -> Attach {
        let n = self.segments(number);
        if !self.at_nodes {
            let segment = segment_at(pct, n);
            return Attach {
                segment: segment as i64,
                index: segment,
            };
        }
        let mut j = split_junction(pct, n);
        if n > 1 {
            if j == 0 && !self.end_connected(number, true) {
                j = 1;
            } else if j == n && !self.end_connected(number, false) {
                j = n - 1;
            }
        }
        Attach {
            segment: if j == 0 { -1 } else { j as i64 },
            index: j,
        }
    }

    /// The point in space an attachment is at.
    fn position(&self, number: u16, a: Attach) -> [f64; 3] {
        let w = self.wire(number);
        let (p, q) = (f64s(&w.end1), f64s(&w.end2));
        let along = if self.at_nodes {
            a.index as f64
        } else {
            a.index as f64 - 0.5
        };
        let t = along / self.segments(number) as f64;
        [
            p[0] + (q[0] - p[0]) * t,
            p[1] + (q[1] - p[1]) * t,
            p[2] + (q[2] - p[2]) * t,
        ]
    }

    /// The other wires with an end at `point`: (wire number, true if it starts there).
    fn wires_at(&self, point: [f64; 3], except: u16) -> Vec<(u16, bool)> {
        let mut found = Vec::new();
        for (i, w) in self.wires.iter().enumerate() {
            let n = i as u16 + 1;
            if n == except {
                continue;
            }
            let tolerance = 1e-3 * self.segment_length(n).min(self.segment_length(except));
            if norm(sub(f64s(&w.end1), point)) <= tolerance {
                found.push((n, true));
            } else if norm(sub(f64s(&w.end2), point)) <= tolerance {
                found.push((n, false));
            }
        }
        found
    }
}

/// One feed point: a segment and the factor applied to the source's value there.
struct Feed {
    wire: u16,
    /// 1-based segment. For NEC-5 a negative number means the segment's first end.
    segment: i64,
    factor: f64,
}

/// Where a source goes, by EZNEC's rules, for engines whose sources are on segments.
fn feeds(s: &Source, g: &Geometry, source_no: usize, w: &mut Writer) -> Result<Vec<Feed>, ConvertError> {
    let n = g.segments(s.wire);
    if !s.kind.is_split() {
        return Ok(vec![Feed {
            wire: s.wire,
            segment: segment_at(s.position_pct, n) as i64,
            factor: 1.0,
        }]);
    }
    // A split voltage source puts half the voltage on each side; a split
    // current source puts the full current on each.
    let share = if s.kind == SourceKind::SplitVoltage { 0.5 } else { 1.0 };
    let j = split_junction(s.position_pct, n);
    if j > 0 && j < n {
        return Ok(vec![
            Feed {
                wire: s.wire,
                segment: j as i64,
                factor: share,
            },
            Feed {
                wire: s.wire,
                segment: j as i64 + 1,
                factor: share,
            },
        ]);
    }
    // At a wire's end the other half goes on the wire joined there.
    let at_start = j == 0;
    let wire = g.wire(s.wire);
    let point = f64s(if at_start { &wire.end1 } else { &wire.end2 });
    let others = g.wires_at(point, s.wire);
    let Some(&(other, other_starts_here)) = others.first() else {
        return Err(ConvertError(format!(
            "source {source_no} is a split source at an open end of wire {}, which EZNEC doesn't allow",
            s.wire
        )));
    };
    if others.len() > 1 {
        w.note(format!(
            "source {source_no}: several wires meet at the split source's junction; wire {other} takes the second half"
        ));
    }
    // Both halves must drive current the same way through the junction. If the
    // two wires' directions oppose there, the second half's sign flips.
    let same_direction = at_start != other_starts_here;
    Ok(vec![
        Feed {
            wire: s.wire,
            segment: if at_start { 1 } else { n as i64 },
            factor: share,
        },
        Feed {
            wire: other,
            segment: if other_starts_here { 1 } else { g.segments(other) as i64 },
            factor: if same_direction { share } else { -share },
        },
    ])
}

/// NEC-5 places sources at segment ends, so a split source is one source at
/// its junction and any other source goes to the nearest junction.
fn nec5_feed(s: &Source, g: &Geometry) -> Feed {
    Feed {
        wire: s.wire,
        segment: g.attach(s.wire, s.position_pct).segment,
        factor: 1.0,
    }
}

fn close_enough(computed: C, stored: (f32, f32)) -> bool {
    let stored = C::new(stored.0 as f64, stored.1 as f64);
    (computed - stored).abs() <= 1e-3 * stored.abs().max(1e-6)
}

/// The frequencies of a stored sweep: start, then steps up to stop.
fn sweep_frequencies(s: &Sweep) -> Result<Vec<f64>, ConvertError> {
    // Through seven digits, so that 14.3 stored as a float is 14.3 again.
    let exact = |x: f32| number(x as f64, 7).parse::<f64>().unwrap_or(x as f64);
    let (start, stop, step) = (exact(s.start_mhz), exact(s.stop_mhz), exact(s.step_mhz));
    if !(start > 0.0 && step > 0.0 && stop >= start) {
        return Err(ConvertError(format!(
            "the stored sweep, {start} to {stop} MHz in steps of {step}, can't be run"
        )));
    }
    let count = ((stop - start) / step + 1e-6).floor() + 1.0;
    if count > MAX_SWEEP as f64 {
        return Err(ConvertError(format!(
            "the stored sweep, {start} to {stop} MHz in steps of {step}, has more than {MAX_SWEEP} frequencies"
        )));
    }
    Ok((0..count as usize)
        .map(|i| exact((start + i as f64 * step) as f32))
        .collect())
}

/// The cards for one frequency, by kind, before they are put in order.
struct Cards {
    notes: Vec<String>,
    /// Segments per wire and wire radii (metres) as written.
    segs: Vec<u16>,
    radii: Vec<f64>,
    dummies: Vec<[f64; 3]>,
    scale: f64,
    ge: i64,
    loads: Vec<String>,
    ground: Vec<String>,
    sources: Vec<String>,
    networks: Vec<String>,
    pattern: Vec<String>,
    /// True if a load or network card's values depend on the frequency.
    varies: bool,
    /// True if the engine loses the ground after its first frequency.
    ground_lapses: bool,
}

/// The model's cards with frequency-dependent values computed at `f_mhz`.
fn cards_at(m: &EzModel, options: &Options, f_mhz: f64) -> Result<Cards, ConvertError> {
    let engine = options.engine;
    let design_mhz = m.frequency_mhz as f64;
    // How a value computed for the frequency is described in a note.
    let computed = if options.sweep {
        "computed for each frequency of the sweep".to_string()
    } else {
        format!(
            "computed at {} MHz, and is right only at that frequency",
            number(f_mhz, 7)
        )
    };
    let mut varies = false;
    let mut w = Writer {
        engine,
        lines: Vec::new(),
        notes: Vec::new(),
    };
    let grounded = !matches!(m.ground, Ground::FreeSpace);
    let mut g = Geometry {
        wires: &m.wires,
        segs: m.wires.iter().map(|x| x.segments).collect(),
        at_nodes: engine == Engine::Nec5,
        grounded,
    };
    if m.wires.is_empty() {
        return Err(ConvertError("the model has no wires".into()));
    }
    if m.wires.len() >= DUMMY_TAG {
        return Err(ConvertError(format!(
            "{} wires is more than this converter handles",
            m.wires.len()
        )));
    }

    // NEC-5 can't attach anything to a one-segment wire, the usual EZNEC feed
    // point for transmission lines. Such a wire gets two segments.
    if engine == Engine::Nec5 {
        let mut used: Vec<u16> = m.sources.iter().map(|s| s.wire).collect();
        for t in &m.transmission_lines {
            used.push(t.wire_a);
            if let TlEnd2::Wire { wire, .. } = t.end2 {
                used.push(wire);
            }
        }
        used.extend(m.l_networks.iter().flat_map(|n| n.ports.iter().map(|p| p.wire)));
        used.sort_unstable();
        used.dedup();
        for wire in used {
            if g.segs[wire as usize - 1] == 1 {
                g.segs[wire as usize - 1] = 2;
                w.note(format!(
                    "wire {wire} has one segment and carries a source, line or network; NEC-5 attaches those at segment ends, so the wire is written with two segments"
                ));
            }
        }
    }

    // Geometry is written in the model's display unit, with GS to metres.
    let scale = match m.units {
        LengthUnit::Wavelengths => 1.0,
        u => u.metres_per_unit(design_mhz),
    };

    // Cards that follow GE are collected by kind, then written in order.
    let mut dummies: Vec<[f64; 3]> = Vec::new(); // each dummy wire's centre, metres
    let dummy = |dummies: &mut Vec<[f64; 3]>| -> i64 {
        let tag = DUMMY_TAG + dummies.len();
        dummies.push([0.0, tag as f64, tag as f64]);
        tag as i64
    };
    let mut loads = Writer {
        engine,
        lines: Vec::new(),
        notes: Vec::new(),
    };
    let mut sources = Writer {
        engine,
        lines: Vec::new(),
        notes: Vec::new(),
    };
    let mut networks = Writer {
        engine,
        lines: Vec::new(),
        notes: Vec::new(),
    };

    // Wire loss.
    let rho: Vec<f32> = m.wires.iter().map(|x| x.resistivity_ohm_m).collect();
    if rho.iter().all(|&r| r == rho[0]) {
        if rho[0] != 0.0 {
            loads.card("LD", &[5, 0, 0, 0], &[1.0 / rho[0] as f64])?;
        }
    } else {
        for (i, &r) in rho.iter().enumerate() {
            if r != 0.0 {
                loads.card("LD", &[5, i as i64 + 1, 0, 0], &[1.0 / r as f64])?;
            }
        }
    }
    if m.wires.iter().any(|x| x.loss_factor != 1.0) {
        w.note("a wire's second loss value is not 1; its meaning is unknown and it is not used");
    }

    // Insulation.
    let mut radii: Vec<f64> = m.wires.iter().map(|x| x.diameter_m as f64 / 2.0).collect();
    for (i, wire) in m.wires.iter().enumerate() {
        let Some(ins) = &wire.insulation else { continue };
        let tag = i as i64 + 1;
        let radius = wire.diameter_m as f64 / 2.0;
        let outer = radius + ins.thickness_m as f64;
        let eps = ins.rel_permittivity as f64;
        if ins.third != 0.0 {
            w.note(format!(
                "wire {tag}: the insulation's third value is not 0; its meaning is unknown and it is not used"
            ));
        }
        if ins.thickness_m <= 0.0 || eps <= 0.0 {
            continue;
        }
        match engine {
            Engine::Nec4 => loads.card("IS", &[0, tag, 0, 0], &[eps, 0.0, outer])?,
            Engine::Nec2 | Engine::Nec5 => {
                // NEC-2 has no IS card, and NEC-5 reads one but has no storage
                // for it and stops. The standard equivalent is a bare wire of
                // radius a·(b/a)^(1 − 1/εr) with a distributed series inductance
                // of (μ0/2π)(1 − 1/εr)·ln(b/a). On NEC-4.2 it agrees with the IS
                // card to 0.1 ohm in 1500 on EZNEC's insulated test dipole.
                let ratio = outer / radius;
                let l_per_m = 2e-7 * (1.0 - 1.0 / eps) * ratio.ln();
                radii[i] = radius * ratio.powf(1.0 - 1.0 / eps);
                loads.card("LD", &[2, tag, 0, 0], &[0.0, l_per_m, 0.0])?;
                w.note(format!(
                    "wire {tag}: {} cannot model insulation; it is a bare wire of equivalent radius with a distributed inductance (LD 2)",
                    engine.name()
                ));
            }
        }
    }

    // Loads.
    for (i, load) in m.loads.iter().enumerate() {
        let no = i + 1;
        let tag = load.wire as i64;
        let seg = segment_at(load.position_pct, g.segments(load.wire)) as i64;
        let fixed = |z_at: &dyn Fn(f64) -> C,
                     what: &str,
                     w: &mut Writer,
                     loads: &mut Writer|
         -> Result<(), ConvertError> {
            let z = z_at(design_mhz);
            if !close_enough(z, load.z_at_design) {
                w.note(format!(
                    "load {no}: computed {} {:+} ohm at the model's frequency, but the file stores {} {:+}; using the computed value",
                    number(z.re, 6),
                    z.im,
                    load.z_at_design.0,
                    load.z_at_design.1
                ));
            }
            w.note(format!("load {no} ({what}) is written as R + jX, {computed}"));
            let z = z_at(f_mhz);
            loads.card("LD", &[4, tag, seg, seg], &[z.re, z.im])
        };
        match &load.value {
            LoadValue::Impedance { r, x } => loads.card("LD", &[4, tag, seg, seg], &[*r as f64, *x as f64])?,
            LoadValue::Rlc {
                form,
                r,
                l_h,
                c_f,
                r_freq_mhz,
            } => {
                let (r, l, c, rf) = (*r as f64, *l_h as f64, *c_f as f64, *r_freq_mhz as f64);
                match form {
                    RlcForm::Series | RlcForm::Parallel => {
                        let kind = if *form == RlcForm::Series { 0 } else { 1 };
                        if rf > 0.0 && r != 0.0 {
                            varies = true;
                            w.note(format!(
                                "load {no}: R is {} ohm at {} MHz and scales as the square root of frequency; the deck has its value {computed}",
                                number(r, 7),
                                number(rf, 7)
                            ));
                        }
                        loads.card("LD", &[kind, tag, seg, seg], &[elements::r_at(r, rf, f_mhz), l, c])?;
                    }
                    RlcForm::Trap => {
                        let z_at = |f: f64| elements::rlc(RlcForm::Trap, r, l, c, rf, f).unwrap_or(C::new(0.0, 0.0));
                        varies = true;
                        fixed(&z_at, "a trap", &mut w, &mut loads)?;
                    }
                }
            }
            LoadValue::Laplace { numerator, denominator } => {
                let z_at = |f: f64| elements::laplace(numerator, denominator, f);
                varies = true;
                fixed(&z_at, "a Laplace load", &mut w, &mut loads)?;
            }
        }
    }

    // Sources. EZNEC's values are RMS; NEC's are peak.
    for (i, s) in m.sources.iter().enumerate() {
        let value = C::polar(SQRT2 * s.magnitude as f64, s.phase_deg as f64);
        let current = matches!(s.kind, SourceKind::Current | SourceKind::SplitCurrent);
        if engine == Engine::Nec5 {
            let feed = nec5_feed(s, &g);
            let kind = if current { 4 } else { 0 };
            sources.card("EX", &[kind, feed.wire as i64, feed.segment, 0], &[value.re, value.im])?;
            if s.kind.is_split() {
                w.note(format!(
                    "source {}: NEC-5 places sources at segment ends, so the split source is one source at its junction",
                    i + 1
                ));
            }
            continue;
        }
        for feed in feeds(s, &g, i + 1, &mut w)? {
            let v = value * feed.factor;
            let (tag, seg) = (feed.wire as i64, feed.segment);
            match (current, engine) {
                (false, _) => sources.card("EX", &[0, tag, seg, 0], &[v.re, v.im])?,
                (true, Engine::Nec4) => sources.card("EX", &[6, tag, seg, 0], &[v.re, v.im])?,
                (true, _) => {
                    // NEC-2 has no current source. A voltage of jI on a far dummy
                    // segment, joined by a network with Y12 = j1 S, forces current I.
                    let d = dummy(&mut dummies);
                    sources.card("EX", &[0, d, 1, 0], &[-v.im, v.re])?;
                    networks.card("NT", &[d, 1, tag, seg], &[0.0, 0.0, 0.0, 1.0, 0.0, 0.0])?;
                    w.note(format!(
                        "source {}: NEC-2 has no current source; wire {d} is a dummy that drives it through a network",
                        i + 1
                    ));
                }
            }
        }
    }

    // Transmission lines.
    for (i, t) in m.transmission_lines.iter().enumerate() {
        let no = i + 1;
        let (ta, a) = (t.wire_a as i64, g.attach(t.wire_a, t.position_a_pct));
        let sa = a.segment;
        let z0 = t.z0_ohm.abs() as f64;
        let vf = t.velocity_factor as f64;
        if vf <= 0.0 || z0 == 0.0 {
            return Err(ConvertError(format!(
                "transmission line {no} has Z0 {z0} and velocity factor {vf}"
            )));
        }
        let lossy = t.loss_db_per_m != 0.0;
        if t.loss_second != 0.0 {
            w.note(format!(
                "transmission line {no}: the second loss value is not 0; its meaning is unknown and it is not used"
            ));
        }
        match &t.end2 {
            TlEnd2::Wire { wire, position_pct } => {
                let b = g.attach(*wire, *position_pct);
                let (tb, sb) = (*wire as i64, b.segment);
                let distance = norm(sub(g.position(t.wire_a, a), g.position(*wire, b)));
                if lossy {
                    let length = t.length_m.map_or(distance, |l| l as f64);
                    let (y11, mut y12) = elements::lossy_line(z0, length, vf, t.loss_db_per_m as f64, f_mhz);
                    if t.reversed() {
                        y12 = -y12;
                    }
                    networks.card(
                        "NT",
                        &[ta, sa, tb, sb],
                        &[y11.re, y11.im, y12.re, y12.im, y11.re, y11.im],
                    )?;
                    varies = true;
                    w.note(format!(
                        "transmission line {no} has loss, which NEC's TL card lacks; it is a network (NT) {computed}"
                    ));
                } else {
                    // NEC's TL card takes the electrical length; 0 means the
                    // distance between the two segments.
                    let length = match t.length_m {
                        Some(l) => l as f64 / vf,
                        None if vf == 1.0 => 0.0,
                        None => distance / vf,
                    };
                    networks.card("TL", &[ta, sa, tb, sb], &[t.z0_ohm as f64, length, 0.0, 0.0, 0.0, 0.0])?;
                }
            }
            TlEnd2::ShortedStub | TlEnd2::OpenStub => {
                let Some(length) = t.length_m else {
                    return Err(ConvertError(format!("transmission line {no} is a stub with no length")));
                };
                let open = t.end2 == TlEnd2::OpenStub;
                let kind = if open { "an open" } else { "a shorted" };
                let d = dummy(&mut dummies);
                if lossy {
                    let stub = if open {
                        elements::lossy_open_stub
                    } else {
                        elements::lossy_shorted_stub
                    };
                    let y = stub(z0, length as f64, vf, t.loss_db_per_m as f64, f_mhz);
                    networks.card("NT", &[ta, sa, d, 1], &[y.re, y.im, 0.0, 0.0, 0.0, 0.0])?;
                    varies = true;
                    w.note(format!(
                        "transmission line {no} is {kind} stub with loss; it is a network (NT) {computed}"
                    ));
                } else {
                    // As EZNEC writes it: a short is a large admittance across
                    // the dummy segment, an open end is none.
                    let end = if open { 0.0 } else { STUB_SHORT_S };
                    networks.card("TL", &[ta, sa, d, 1], &[z0, length as f64 / vf, 0.0, 0.0, end, 0.0])?;
                }
                w.note(format!(
                    "transmission line {no} is {kind} stub; wire {d} is a dummy that carries its far end"
                ));
            }
        }
    }

    // L networks.
    for (i, n) in m.l_networks.iter().enumerate() {
        let [p1, p2] = &n.ports;
        let (y11, y12, y22) = elements::l_network(n, f_mhz);
        networks.card(
            "NT",
            &[
                p1.wire as i64,
                g.attach(p1.wire, p1.position_pct).segment,
                p2.wire as i64,
                g.attach(p2.wire, p2.position_pct).segment,
            ],
            &[y11.re, y11.im, y12.re, y12.im, y22.re, y22.im],
        )?;
        varies = true;
        w.note(format!("L network {} is a network (NT) {computed}", i + 1));
    }

    // Ground and pattern.
    let below_ground = m.wires.iter().any(|x| {
        let tolerance = 1e-3 * norm(sub(f64s(&x.end2), f64s(&x.end1))) / x.segments as f64;
        (x.end1[2] as f64) < -tolerance || (x.end2[2] as f64) < -tolerance
    });
    let mut ground = Writer {
        engine,
        lines: Vec::new(),
        notes: Vec::new(),
    };
    let mut cliff = false; // NEC-2 takes the second medium's shape from RP
    let ge = match &m.ground {
        Ground::FreeSpace => 0,
        Ground::Perfect => {
            ground.card("GN", &[1], &[])?;
            1
        }
        Ground::Real { model, media } => {
            let first = &media[0];
            // NEC-5 has no second medium: there GD's distance and drop fields
            // mean something else, and EZNEC deletes the medium.
            let media = if engine == Engine::Nec5 && media.len() > 1 {
                w.note("NEC-5 has no second ground medium; only the first is used, as in EZNEC");
                &media[..1]
            } else {
                &media[..]
            };
            let second_kind = if engine == Engine::Nec2 { 0 } else { 2 };
            let gd = |medium: &Medium, ground: &mut Writer| {
                ground.card(
                    "GD",
                    &[second_kind, 0, 0, 0],
                    &[
                        medium.rel_permittivity as f64,
                        medium.conductivity_s_per_m as f64,
                        medium.r_coord_m as f64,
                        -(medium.height_m as f64),
                    ],
                )
            };
            match model {
                RealGroundModel::HighAccuracy => {
                    ground.card(
                        "GN",
                        &[2, 0, 0, 0],
                        &[first.rel_permittivity as f64, first.conductivity_s_per_m as f64],
                    )?;
                    if let Some(second) = media.get(1) {
                        gd(second, &mut ground)?;
                        cliff = true;
                    }
                }
                RealGroundModel::Mininec => {
                    // The MININEC-type ground: perfect ground for the currents,
                    // the real ground for the far field only.
                    ground.card("GN", &[1], &[])?;
                    w.note("MININEC-type ground: perfect ground for the currents (GN 1), real ground for the far field (GD)");
                    match media.get(1) {
                        None => gd(first, &mut ground)?,
                        Some(second) => {
                            gd(second, &mut ground)?;
                            w.note(format!(
                                "two ground media with the MININEC-type ground: the far field uses a perfect ground out to the second medium, not the first medium (diel {}, cond {} S/m), which {} cannot apply here",
                                first.rel_permittivity,
                                first.conductivity_s_per_m,
                                engine.name()
                            ));
                        }
                    }
                    cliff = true;
                }
            }
            if media.len() > 2 {
                w.note(format!("{} ground media: only the first two are used", media.len()));
            }
            if below_ground { -1 } else { 1 }
        }
    };
    let has_ground = !matches!(m.ground, Ground::FreeSpace);
    if below_ground
        && !matches!(
            m.ground,
            Ground::Real {
                model: RealGroundModel::HighAccuracy,
                ..
            }
        )
        && has_ground
    {
        w.note("wires go below z = 0, which this ground type does not allow");
    }

    let p = &m.plot;
    let step = if p.step_deg > 0.0 { p.step_deg as f64 } else { 1.0 };
    let mode = if engine == Engine::Nec2 && cliff { 3 } else { 0 };
    let points = |range: f64| (range / step).round() as i64 + 1;
    let elevation = match p.kind {
        PlotKind::Elevation => true,
        PlotKind::Azimuth | PlotKind::ThreeD => false,
        PlotKind::Code2 => p.cut == PlotCut::Elevation,
    };
    let mut pattern = Writer {
        engine,
        lines: Vec::new(),
        notes: Vec::new(),
    };
    if p.kind == PlotKind::ThreeD {
        let thetas = if has_ground { 19 } else { 37 };
        pattern.card("RP", &[mode, thetas, 73, 1001], &[0.0, 0.0, 5.0, 5.0])?;
    } else if p.ground_wave_distance_m != 0.0 && has_ground && !elevation {
        // EZNEC allows ground wave analysis with an azimuth plot only, and
        // the angle field then holds the observation height (RP's z).
        // NEC-2 and NEC-4.2 return fields near zero over GN 1 (perfect
        // ground); NEC-5 with the GD card gives the real-ground field.
        if engine != Engine::Nec5
            && matches!(
                m.ground,
                Ground::Real {
                    model: RealGroundModel::Mininec,
                    ..
                }
            )
        {
            w.note(format!(
                "ground wave with the MININEC-type ground: {} computes it over perfect ground and the field comes out near zero; use the high-accuracy ground or NEC-5",
                engine.name()
            ));
        }
        pattern.card(
            "RP",
            &[1, 1, points(360.0), 1000],
            &[p.angle_deg as f64, 0.0, 0.0, step, p.ground_wave_distance_m as f64],
        )?;
    } else if elevation {
        // From the horizon over the top to the other horizon; all the way
        // round in free space.
        let (start, range) = if has_ground { (90.0, 180.0) } else { (180.0, 360.0) };
        pattern.card(
            "RP",
            &[mode, points(range), 1, 1000],
            &[start, p.angle_deg as f64, -step, 0.0],
        )?;
    } else {
        pattern.card(
            "RP",
            &[mode, 1, points(360.0), 1000],
            &[90.0 - p.angle_deg as f64, 0.0, 0.0, step],
        )?;
    }

    // NEC-5 with GN 1 and a GD card (the MININEC-type ground) solves its
    // first frequency over perfect ground and every later one over the GD
    // card's ground, with a stepped FR card or a new one, unless the ground
    // cards are given again. On a 10 ft dipole the second frequency's
    // resistance came out 2.58 ohms for 1.62, and the gain 2 dB low.
    let ground_lapses = engine == Engine::Nec5 && ground.lines.iter().any(|l| l.starts_with("GD"));
    Ok(Cards {
        notes: w.notes,
        segs: g.segs,
        radii,
        dummies,
        scale,
        ge,
        loads: loads.lines,
        ground: ground.lines,
        sources: sources.lines,
        networks: networks.lines,
        pattern: pattern.lines,
        varies,
        ground_lapses,
    })
}

/// Writes the deck for `options.engine`.
pub fn write_deck(m: &EzModel, options: &Options) -> Result<Deck, ConvertError> {
    let engine = options.engine;
    let design_mhz = m.frequency_mhz as f64;
    let frequencies = match (&m.sweep, options.sweep) {
        (_, false) => vec![design_mhz],
        (Some(s), true) => sweep_frequencies(s)?,
        (None, true) => return Err(ConvertError("the model stores no frequency sweep".into())),
    };
    let first = cards_at(m, options, frequencies[0])?;
    let mut notes = first.notes.clone();
    let mut note = |text: String| {
        if !notes.contains(&text) {
            notes.push(text);
        }
    };
    if let Some(s) = &m.sweep {
        if options.sweep {
            note(format!(
                "the deck runs the stored sweep, {} to {} MHz in steps of {} ({} frequencies), not the model's frequency of {} MHz",
                s.start_mhz,
                s.stop_mhz,
                s.step_mhz,
                frequencies.len(),
                number(design_mhz, 7)
            ));
            if frequencies.len() > 1 {
                if first.varies {
                    note("loads and networks whose values depend on frequency are written again, with FR and RP, for each frequency".into());
                } else if first.ground_lapses {
                    note("NEC-5 keeps this ground for one frequency only, so the ground cards are written again, with FR and RP, for each frequency".into());
                }
            }
        } else {
            note(format!(
                "the model stores a sweep from {} to {} MHz in steps of {}; the deck runs the model's frequency only",
                s.start_mhz, s.stop_mhz, s.step_mhz
            ));
        }
    }
    let (scale, radii, dummies) = (first.scale, &first.radii, &first.dummies);

    // Assemble.
    let mut out = Writer {
        engine,
        lines: Vec::new(),
        notes: Vec::new(),
    };
    let title = if m.title.is_empty() { "(no title)" } else { &m.title };
    out.lines.push(format!("CM {title}"));
    let from = options
        .source_name
        .as_deref()
        .map(|n| format!(" from {n}"))
        .unwrap_or_default();
    out.lines.push(format!(
        "CM ez2nec {}: converted{from} for {}",
        env!("CARGO_PKG_VERSION"),
        engine.name()
    ));
    for line in notes.iter() {
        // Keep comment lines within the engine's limit too.
        let mut rest = line.as_str();
        let width = engine.line_limit() - "CM ez2nec:   ".len();
        let mut first = true;
        while !rest.is_empty() {
            let cut = if rest.len() <= width {
                rest.len()
            } else {
                rest[..width].rfind(' ').unwrap_or(width)
            };
            out.lines
                .push(format!("CM ez2nec: {}{}", if first { "" } else { "  " }, &rest[..cut]));
            rest = rest[cut..].trim_start();
            first = false;
        }
    }
    out.lines.push("CE".into());
    for (i, wire) in m.wires.iter().enumerate() {
        let mut v: Vec<f64> = wire.end1.iter().chain(&wire.end2).map(|&c| c as f64 / scale).collect();
        v.push(radii[i] / scale);
        out.card("GW", &[i as i64 + 1, first.segs[i] as i64], &v)?;
    }
    for (i, c) in dummies.iter().enumerate() {
        // A short wire far from the model: 2 cm long, 0.5 mm radius. NEC-5
        // needs two segments to have a segment end to attach to.
        let v = [
            (c[0] - 0.01) / scale,
            c[1] / scale,
            c[2] / scale,
            (c[0] + 0.01) / scale,
            c[1] / scale,
            c[2] / scale,
            0.0005 / scale,
        ];
        out.card(
            "GW",
            &[(DUMMY_TAG + i) as i64, if engine == Engine::Nec5 { 2 } else { 1 }],
            &v,
        )?;
    }
    if scale != 1.0 {
        out.card("GS", &[0, 0], &[scale])?;
    }
    out.card("GE", &[first.ge], &[])?;
    if first.varies || first.ground_lapses {
        // NEC keeps the sources from one run to the next. A new group of LD
        // cards replaces all loads, and a new group of NT and TL cards all
        // networks, so each frequency gets every one of them again, and the
        // ground cards too (see `ground_lapses`).
        for (i, &f) in frequencies.iter().enumerate() {
            let cards = if i == 0 { None } else { Some(cards_at(m, options, f)?) };
            let cards = cards.as_ref().unwrap_or(&first);
            out.lines.extend(cards.loads.iter().cloned());
            out.card("FR", &[0, 1, 0, 0], &[f, 0.0])?;
            out.lines.extend(first.ground.iter().cloned());
            if i == 0 {
                out.lines.extend(first.sources.iter().cloned());
            }
            out.lines.extend(cards.networks.iter().cloned());
            out.lines.extend(cards.pattern.iter().cloned());
        }
    } else {
        out.lines.extend(first.loads.iter().cloned());
        let step = frequencies.get(1).map_or(0.0, |f| f - frequencies[0]);
        out.card("FR", &[0, frequencies.len() as i64, 0, 0], &[frequencies[0], step])?;
        out.lines.extend(first.ground.iter().cloned());
        out.lines.extend(first.sources.iter().cloned());
        out.lines.extend(first.networks.iter().cloned());
        out.lines.extend(first.pattern.iter().cloned());
    }
    out.lines.push("EN".into());

    let mut text = out.lines.join("\n");
    text.push('\n');
    Ok(Deck { text, notes })
}
