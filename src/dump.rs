//! A text rendering of a decoded model, for checking against EZNEC's screens.
//!
//! Lengths are shown in the model's display unit, as EZNEC shows them.

use std::fmt::Write;

use crate::ez::*;
use crate::rules::{segment_at, split_junction};

/// Rounds to 7 significant digits (a float32's worth) and prints the shortest form.
fn num(x: f64) -> String {
    if x == 0.0 || !x.is_finite() {
        return format!("{x}");
    }
    let rounded: f64 = format!("{x:.6e}").parse().unwrap_or(x);
    format!("{rounded}")
}

fn f(x: f32) -> String {
    if x != 0.0 && x.abs() < 1e-3 {
        format!("{x:e}")
    } else {
        format!("{x}")
    }
}

fn hex(bytes: &[u8]) -> String {
    let mut s = String::new();
    for (i, b) in bytes.iter().enumerate() {
        if i > 0 {
            s.push(' ');
        }
        if i == 24 {
            let _ = write!(s, "… ({} bytes)", bytes.len());
            break;
        }
        let _ = write!(s, "{b:02x}");
    }
    if bytes.len() >= 2 && bytes.iter().all(|b| b.is_ascii_graphic() || *b == b' ') {
        let _ = write!(s, "  \"{}\"", String::from_utf8_lossy(bytes));
    }
    s
}

/// Renders the model and the decode report as text.
pub fn dump(file: &EzFile) -> String {
    let m = &file.model;
    let mut o = String::new();
    let unit = m.units;
    let scale = unit.metres_per_unit(m.frequency_mhz as f64);
    let len = |metres: f32| num(metres as f64 / scale);
    let point = |p: &[f32; 3]| format!("({}, {}, {})", len(p[0]), len(p[1]), len(p[2]));
    let segs_of = |wire: u16| m.wires[wire as usize - 1].segments;
    let at = |wire: u16, pct: f32| {
        format!(
            "wire {wire} at {}% (segment {})",
            f(pct),
            segment_at(pct, segs_of(wire))
        )
    };

    let _ = writeln!(o, "Title:      {}", m.title);
    let _ = writeln!(o, "Frequency:  {} MHz", f(m.frequency_mhz));
    let _ = writeln!(o, "Units:      {}", unit.symbol());
    let mut saved = String::new();
    if let Some(p) = &m.saved_by.program {
        saved.push_str(p);
    }
    if let Some(v) = m.saved_by.version_text() {
        if !saved.is_empty() {
            saved.push(' ');
        }
        let _ = write!(saved, "v{v}");
    }
    if saved.is_empty() {
        saved.push_str("not recorded");
    }
    let _ = writeln!(o, "Saved by:   {saved}");
    if let Some(e) = &m.engine {
        let kind = match e.kind {
            EngineKind::EzCalcD => "EZCalcD (built-in NEC-2D)".to_string(),
            EngineKind::ExtNec4 => "external NEC-4".to_string(),
            EngineKind::ExtNec5 => "external NEC-5".to_string(),
            EngineKind::Other(c) => format!("code {c}"),
        };
        let path = if e.path.is_empty() {
            String::new()
        } else {
            format!(", {}", e.path)
        };
        let _ = writeln!(o, "Engine:     {kind}: {}{path}", e.name);
    }

    match &m.ground {
        Ground::FreeSpace => {
            let _ = writeln!(o, "Ground:     free space");
        }
        Ground::Perfect => {
            let _ = writeln!(o, "Ground:     perfect");
        }
        Ground::Real { model, media } => {
            let name = match model {
                RealGroundModel::Mininec => "MININEC",
                RealGroundModel::HighAccuracy => "high accuracy",
            };
            let _ = writeln!(o, "Ground:     real, {name}, {} media", media.len());
            for (i, g) in media.iter().enumerate() {
                let _ = writeln!(
                    o,
                    "  medium {}: cond {} S/m, diel {}, height {} {u}, R coord {} {u}",
                    i + 1,
                    f(g.conductivity_s_per_m),
                    f(g.rel_permittivity),
                    len(g.height_m),
                    len(g.r_coord_m),
                    u = unit.symbol(),
                );
            }
        }
    }

    let _ = writeln!(o, "\nWires ({}), ends in {}:", m.wires.len(), unit.symbol());
    for (i, w) in m.wires.iter().enumerate() {
        let (dia, dia_unit) = match unit {
            LengthUnit::Feet | LengthUnit::Inches => (w.diameter_m as f64 / 0.0254, "in"),
            LengthUnit::Metres | LengthUnit::Millimetres => (w.diameter_m as f64 * 1000.0, "mm"),
            LengthUnit::Wavelengths => (w.diameter_m as f64 / scale, "wl"),
        };
        let gauge = w.awg.map(|g| format!(" (#{g} AWG)")).unwrap_or_default();
        let _ = write!(
            o,
            "  {:>3}: {} to {}, dia {} {dia_unit}{gauge}, {} segs",
            i + 1,
            point(&w.end1),
            point(&w.end2),
            num(dia),
            w.segments
        );
        if w.resistivity_ohm_m != 0.0 {
            let _ = write!(o, ", resistivity {} ohm-m", f(w.resistivity_ohm_m));
        }
        if w.loss_factor != 1.0 {
            let _ = write!(o, ", loss factor {}", f(w.loss_factor));
        }
        if let Some(ins) = &w.insulation {
            let _ = write!(
                o,
                ", insulation diel {} thickness {} {}",
                f(ins.rel_permittivity),
                len(ins.thickness_m),
                unit.symbol()
            );
            if ins.third != 0.0 {
                let _ = write!(o, " third value {}", f(ins.third));
            }
        }
        o.push('\n');
    }

    let _ = writeln!(o, "\nSources ({}):", m.sources.len());
    for (i, s) in m.sources.iter().enumerate() {
        let (kind, q) = match s.kind {
            SourceKind::Voltage => ("voltage", "V"),
            SourceKind::Current => ("current", "A"),
            SourceKind::SplitVoltage => ("split voltage", "V"),
            SourceKind::SplitCurrent => ("split current", "A"),
        };
        let place = if s.kind.is_split() {
            let n = segs_of(s.wire);
            let j = split_junction(s.position_pct, n);
            let where_ = if j == 0 {
                "the wire's start".to_string()
            } else if j == n {
                "the wire's end".to_string()
            } else {
                format!("between segments {j} and {}", j + 1)
            };
            format!("wire {} at {}% ({where_})", s.wire, f(s.position_pct))
        } else {
            at(s.wire, s.position_pct)
        };
        let _ = writeln!(
            o,
            "  {:>3}: {kind}, {place}, {} {q} RMS at {} deg",
            i + 1,
            f(s.magnitude),
            f(s.phase_deg)
        );
    }

    if !m.loads.is_empty() {
        let _ = writeln!(o, "\nLoads ({}):", m.loads.len());
        for (i, l) in m.loads.iter().enumerate() {
            let z = format!(
                "{} {} j{} ohm",
                f(l.z_at_design.0),
                if l.z_at_design.1 < 0.0 { "-" } else { "+" },
                f(l.z_at_design.1.abs())
            );
            let value = match &l.value {
                LoadValue::Impedance { .. } => format!("R + jX {z}"),
                LoadValue::Rlc {
                    form,
                    r,
                    l_h,
                    c_f,
                    r_freq_mhz,
                } => {
                    let form = match form {
                        RlcForm::Series => "series",
                        RlcForm::Parallel => "parallel",
                        RlcForm::Trap => "trap",
                    };
                    format!(
                        "{form} RLC R {} ohm, L {} H, C {} F, R Freq {} MHz (stored Z {z})",
                        f(*r),
                        f(*l_h),
                        f(*c_f),
                        f(*r_freq_mhz)
                    )
                }
                LoadValue::Laplace { numerator, denominator } => format!(
                    "Laplace num {:?} den {:?}, s^0 first (stored Z {z})",
                    numerator, denominator
                ),
            };
            let _ = writeln!(o, "  {:>3}: {}, {value}", i + 1, at(l.wire, l.position_pct));
        }
    }

    if !m.transmission_lines.is_empty() {
        let _ = writeln!(o, "\nTransmission lines ({}):", m.transmission_lines.len());
        for (i, t) in m.transmission_lines.iter().enumerate() {
            let length = match t.length_m {
                Some(l) => format!("length {} {}", len(l), unit.symbol()),
                None => "actual distance".to_string(),
            };
            let _ = write!(
                o,
                "  {:>3}: {} to {}, Z0 {} ohm{}, {length}, VF {}",
                i + 1,
                at(t.wire_a, t.position_a_pct),
                match &t.end2 {
                    TlEnd2::Wire { wire, position_pct } => at(*wire, *position_pct),
                    TlEnd2::ShortedStub => "a shorted stub end".to_string(),
                    TlEnd2::OpenStub => "an open stub end".to_string(),
                },
                f(t.z0_ohm.abs()),
                if t.reversed() { " reversed" } else { "" },
                f(t.velocity_factor)
            );
            if t.loss_db_per_m != 0.0 {
                let _ = write!(
                    o,
                    ", loss {} dB/100 ft ({} dB/m)",
                    num(t.loss_db_per_m as f64 * 30.48),
                    f(t.loss_db_per_m)
                );
            }
            if t.loss_second != 0.0 {
                let _ = write!(o, ", second loss value {}", f(t.loss_second));
            }
            o.push('\n');
        }
    }

    if !m.l_networks.is_empty() {
        let _ = writeln!(o, "\nL networks ({}):", m.l_networks.len());
        for (i, n) in m.l_networks.iter().enumerate() {
            for (p, port) in n.ports.iter().enumerate() {
                let conn = match port.connection {
                    Connection::Series => "Ser",
                    Connection::Parallel => "Par",
                };
                let _ = writeln!(
                    o,
                    "  {:>3} port {}: {}, R {} ohm, L {} H, C {} F, R Freq {} MHz, {conn} (stored Z {} + j{})",
                    i + 1,
                    p + 1,
                    at(port.wire, port.position_pct),
                    f(port.r_ohm),
                    f(port.l_h),
                    f(port.c_f),
                    f(port.r_freq_mhz),
                    f(port.z_at_design.0),
                    f(port.z_at_design.1)
                );
            }
        }
    }

    let p = &m.plot;
    let kind = match p.kind {
        PlotKind::Azimuth => "azimuth",
        PlotKind::Elevation => "elevation",
        PlotKind::ThreeD => "3D",
        PlotKind::Code2 => "code '2'",
    };
    let cut = match p.cut {
        PlotCut::Azimuth => "azimuth",
        PlotCut::Elevation => "elevation",
    };
    let _ = writeln!(
        o,
        "\nPlot:       {kind} (2D cut {cut}), angle {} deg, step {} deg, range {} deg",
        f(p.angle_deg),
        f(p.step_deg),
        f(p.range_deg)
    );
    if p.ground_wave_distance_m != 0.0 {
        let _ = writeln!(
            o,
            "Ground wave distance: {} {}",
            len(p.ground_wave_distance_m),
            unit.symbol()
        );
    }
    match &m.sweep {
        Some(s) => {
            let _ = writeln!(
                o,
                "Sweep:      {} to {} MHz, step {}",
                f(s.start_mhz),
                f(s.stop_mhz),
                f(s.step_mhz)
            );
        }
        None => {
            let _ = writeln!(o, "Sweep:      none");
        }
    }
    let _ = writeln!(o, "Ref Level:  {} dBi", f(m.ref_level_dbi));
    let _ = writeln!(o, "Alt SWR Z0: {} ohm", f(m.alt_swr_z0));
    if let Some(t) = &m.saved_by.text {
        let _ = writeln!(o, "File text:  {t}");
    }

    let r = &file.report;
    if !r.notes.is_empty() {
        let _ = writeln!(o, "\nNotes:");
        for n in &r.notes {
            let _ = writeln!(o, "  {n}");
        }
    }
    let known: Vec<_> = r.unknown_fields.iter().filter(|u| !u.is_zero()).collect();
    let _ = writeln!(o, "\nPlaced but not decoded ({} non-zero):", known.len());
    for u in known {
        let _ = writeln!(o, "  {:#06x} {}: {}", u.offset, u.label, hex(&u.bytes));
    }
    let _ = writeln!(o, "\nUnaccounted non-zero bytes ({}):", r.unclaimed.len());
    for u in &r.unclaimed {
        let _ = writeln!(o, "  {:#06x}: {}", u.offset, hex(&u.bytes));
    }
    o
}
