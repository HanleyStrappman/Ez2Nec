//! `read_ez`: bytes to [`EzFile`].
//!
//! Layout (all little-endian), as decoded from EZNEC's own saves:
//! - a 0xBA-byte header;
//! - one 170-byte record per row, where record k holds row k of several tables
//!   (wire, source, load, transmission line, and ground medium k+2);
//! - 324 bytes that are zero except one byte in v6 and later saves;
//! - tagged chunks (`tag u16`, `len u16` counting the 4-byte header) to the end.
//!
//! Each table's count is in the header. Rows past a count hold stale values.

use super::bytes::{Bytes, EzError, latin1};
use super::model::*;

pub const HEADER_LEN: usize = 0xBA;
pub const RECORD_LEN: usize = 170;
pub const GAP_LEN: usize = 324;
/// Header 0x40. Every known file, from 2001 to EZNEC v7, has 5.
pub const FORMAT_BYTE: u8 = 5;

// Record field offsets.
const R_WIRE: usize = 0x00;
const R_SOURCE: usize = 0x20;
const R_LOAD: usize = 0x2F;
const R_LAPLACE: usize = 0x3D;
const R_TL: usize = 0x6D;
const R_RLC: usize = 0x85;
const R_MEDIUM: usize = 0x9A;

// Tail chunk tags.
const CHUNK_OPTIONS: u16 = 0x0b;
const CHUNK_INSULATION: u16 = 0x0c;
const CHUNK_WIRE_LOSS: u16 = 0x0d;
const CHUNK_TL_LOSS: u16 = 0x0e;
const CHUNK_L_NETWORKS: u16 = 0x11;
const CHUNK_1F: u16 = 0x1f;
const CHUNK_PROGRAM: u16 = 0x65;
const CHUNK_ENGINE: u16 = 0x66;

/// Decodes an `.ez` file.
///
/// A file whose layout doesn't match what is known is refused, and a code
/// byte with an unknown value in a field that changes the model is an error.
/// Fields that are placed but not understood, and any non-zero byte no step
/// accounts for, are listed in the report rather than guessed at.
pub fn read_ez(data: &[u8]) -> Result<EzFile, EzError> {
    if data.len() < HEADER_LEN {
        return Err(EzError::new(format!(
            "{} bytes is too short for an EZNEC file header ({HEADER_LEN} bytes)",
            data.len()
        )));
    }
    let mut b = Bytes::new(data);
    let mut notes = Vec::new();

    let format = b.u8(0x40)?;
    if format != FORMAT_BYTE {
        return Err(EzError::at(
            0x40,
            format!("format byte is {format}; only {FORMAT_BYTE} is known"),
        ));
    }

    // Counts.
    let n_records = count(&mut b, 0x00, "record count")?;
    let n_media = count(&mut b, 0x02, "ground media count")?;
    let n_wires = count(&mut b, 0x30, "wire count")?;
    let n_sources = count(&mut b, 0x32, "source count")?;
    let n_loads = count(&mut b, 0x34, "load count")?;
    let n_tls = count(&mut b, 0x78, "transmission line count")?;
    let n_lnets_hdr = b.i32(0x9C)?;
    for (n, what) in [
        (n_wires, "wires"),
        (n_sources, "sources"),
        (n_loads, "loads"),
        (n_tls, "transmission lines"),
    ] {
        if n > n_records {
            return Err(EzError::new(format!("{n} {what} but only {n_records} records")));
        }
    }
    if n_media >= 2 && n_media - 1 > n_records {
        return Err(EzError::new(format!(
            "{n_media} ground media but only {n_records} records"
        )));
    }

    let records_end = HEADER_LEN + RECORD_LEN * n_records;
    let chunks_start = records_end + GAP_LEN;
    if data.len() < chunks_start {
        return Err(EzError::new(format!(
            "{n_records} records need at least {chunks_start} bytes, but the file has {}",
            data.len()
        )));
    }

    // Header scalars.
    let frequency_mhz = b.f32(0x05)?;
    let title = latin1(b.slice(0x12, 30)?).trim_end_matches([' ', '\0']).to_string();
    let units = match b.u8(0x41)? {
        b'F' => LengthUnit::Feet,
        b'I' => LengthUnit::Inches,
        b'L' => LengthUnit::Millimetres,
        b'M' => LengthUnit::Metres,
        b'W' => LengthUnit::Wavelengths,
        c => return Err(bad_code(0x41, "display unit", c)),
    };

    let plot = read_plot(&mut b, &mut notes)?;
    let ref_level_dbi = b.f32(0x5E)?;
    let alt_swr_z0 = b.f32(0x6E)?;
    let header_resistivity = b.f32(0x62)?;
    let header_loss_factor = b.f32(0x66)?;
    let version_code = b.u32(0x90)?;

    // v6 and later copy the record and wire counts to 0x7C and 0x80. Older
    // files have 0 at 0x7C and something else (a float) at 0x80.
    let copy = b.i32(0x7C)?;
    if copy != 0 {
        if copy as usize != n_records {
            notes.push(format!("header 0x7C is {copy}, but the record count is {n_records}"));
        }
        let copy = b.i32(0x80)?;
        if copy as usize != n_wires {
            notes.push(format!("header 0x80 is {copy}, but the wire count is {n_wires}"));
        }
    } else {
        b.unknown(0x80, 4, "header 0x80 (a float in files before v6)")?;
    }

    for (off, len, label) in [
        (0x04, 1, "header 0x04 ('R' in nearly every file)"),
        (0x37, 6, "header 0x37"),
        (0x3D, 3, "header 0x3D (FF FF 18 in most files)"),
        (0x42, 2, "header 0x42 ('FT' in most files)"),
        (0x44, 4, "header 0x44 (float 1304.0 in every file)"),
        (0x50, 1, "header 0x50 ('Z'; 'S' with RLC or Laplace loads)"),
        (0x59, 1, "header 0x59 ('Z'; 'R' with L networks; v6 and later)"),
        (0x6A, 1, "header 0x6A (set in some files before v6)"),
        (0x72, 2, "header 0x72"),
        (0x74, 1, "header 0x74"),
        (0x76, 2, "header 0x76"),
        (0x84, 4, "header 0x84 (a float in some files before v6)"),
        (0x88, 4, "header 0x88 (float 0 or 5)"),
        (0x94, 4, "header 0x94 (float 40 in v6 and later)"),
        (0xA2, 1, "header 0xA2 (ground model copy, v7)"),
    ] {
        b.unknown(off, len, label)?;
    }

    // Load type applies to every load in the model. Old files leave it blank
    // for R + jX loads.
    let load_type = match b.u8(0x75)? {
        b' ' => b'Z',
        c => c,
    };
    if n_loads > 0 && !matches!(load_type, b'Z' | b'R' | b'L') {
        return Err(bad_code(0x75, "load type", load_type));
    }

    // Ground.
    let ground_code = b.u8(0x36)?;
    let ground_model_code = b.u8(0x7B)?;
    let real = match ground_code {
        b'F' | b'P' => false,
        b'R' => true,
        c => return Err(bad_code(0x36, "ground type", c)),
    };
    let mut media = Vec::new();
    if real {
        if n_media == 0 {
            return Err(EzError::at(0x02, "real ground with no ground media"));
        }
        media.push(read_medium(&mut b, 0xAA)?);
    } else {
        if n_media != 0 {
            notes.push(format!(
                "ground is {} but the media count is {n_media}",
                ground_code as char
            ));
        }
        b.leftover(0xAA, 16)?;
    }

    // Records.
    let mut wires = Vec::with_capacity(n_wires);
    let mut sources = Vec::with_capacity(n_sources);
    let mut loads = Vec::with_capacity(n_loads);
    let mut tls = Vec::with_capacity(n_tls);
    for k in 0..n_records {
        let r = HEADER_LEN + RECORD_LEN * k;

        if k < n_wires {
            wires.push(read_wire(&mut b, r + R_WIRE)?);
        } else {
            b.leftover(r + R_WIRE, R_SOURCE - R_WIRE)?;
        }

        if k < n_sources {
            sources.push(read_source(&mut b, r + R_SOURCE)?);
        } else {
            b.leftover(r + R_SOURCE, R_LOAD - R_SOURCE)?;
        }

        if k < n_loads {
            loads.push(read_load(&mut b, r, load_type)?);
        } else {
            b.leftover(r + R_LOAD, R_TL - R_LOAD)?;
            b.leftover(r + R_RLC, 18)?;
        }

        if k < n_tls {
            tls.push(read_tl(&mut b, r + R_TL)?);
        } else {
            b.leftover(r + R_TL, R_RLC - R_TL)?;
        }

        // Medium 1 is in the header; medium m (2, 3, …) is in record m − 2.
        if real && k + 2 <= n_media {
            media.push(read_medium(&mut b, r + R_MEDIUM)?);
        } else {
            b.leftover(r + R_MEDIUM, 16)?;
        }
    }

    b.unknown(
        records_end + 0x34,
        1,
        "byte 0x34 after the records (0x20 in v6 and later)",
    )?;

    // Chunks.
    let mut options: Option<Option<Sweep>> = None;
    let mut insulation: Option<Vec<Insulation>> = None;
    let mut wire_loss: Option<Vec<(f32, f32)>> = None;
    let mut tl_loss: Option<Vec<(f32, f32)>> = None;
    let mut l_networks: Option<Vec<LNetwork>> = None;
    let mut saved_by = SavedBy {
        version_code,
        ..SavedBy::default()
    };
    let mut engine = None;

    let mut pos = chunks_start;
    while pos < data.len() {
        if data.len() - pos < 4 {
            return Err(EzError::at(pos, "a chunk header needs 4 bytes"));
        }
        let tag = b.i16(pos)? as u16;
        let len = b.i16(pos + 2)? as u16 as usize;
        if len < 4 || pos + len > data.len() {
            return Err(EzError::at(
                pos,
                format!("chunk {tag:#04x} has length {len}, which doesn't fit the file"),
            ));
        }
        let c = Chunk {
            tag,
            start: pos + 4,
            end: pos + len,
        };
        match tag {
            CHUNK_OPTIONS => set_once(&mut options, c, read_options(&mut b, &c)?)?,
            CHUNK_INSULATION => set_once(&mut insulation, c, read_insulation(&mut b, &c)?)?,
            CHUNK_WIRE_LOSS => set_once(&mut wire_loss, c, read_pairs(&mut b, &c)?)?,
            CHUNK_TL_LOSS => set_once(&mut tl_loss, c, read_pairs(&mut b, &c)?)?,
            CHUNK_L_NETWORKS => set_once(&mut l_networks, c, read_l_networks(&mut b, &c)?)?,
            CHUNK_PROGRAM => match read_program(&mut b, &c, &mut saved_by) {
                Ok(()) => {}
                Err(e) => {
                    notes.push(format!("chunk 0x65 not understood ({e})"));
                    b.unknown(c.start, c.len(), "chunk 0x65")?;
                }
            },
            CHUNK_ENGINE => match read_engine(&mut b, &c) {
                Ok(e) => engine = Some(e),
                Err(e) => {
                    notes.push(format!("chunk 0x66 not understood ({e})"));
                    b.unknown(c.start, c.len(), "chunk 0x66")?;
                }
            },
            CHUNK_1F => b.unknown(c.start, c.len(), "chunk 0x1f")?,
            _ => b.unknown(c.start, c.len(), format!("chunk {tag:#04x} (unknown tag)"))?,
        }
        pos = c.end;
    }

    // Per-wire values from the chunks.
    if let Some(rows) = &wire_loss {
        check_rows(rows.len(), n_wires, 0x0d, "wire")?;
        if header_resistivity != 0.0 && rows.iter().any(|&(rho, _)| rho != header_resistivity) {
            notes.push(format!(
                "header resistivity {header_resistivity} Ω·m differs from chunk 0x0d's per-wire values; using the chunk"
            ));
        }
        for (w, &(rho, factor)) in wires.iter_mut().zip(rows) {
            w.resistivity_ohm_m = rho;
            w.loss_factor = factor;
        }
    } else {
        for w in &mut wires {
            w.resistivity_ohm_m = header_resistivity;
            w.loss_factor = header_loss_factor;
        }
    }
    if let Some(rows) = insulation {
        check_rows(rows.len(), n_wires, 0x0c, "wire")?;
        for (w, ins) in wires.iter_mut().zip(rows) {
            if ins.thickness_m != 0.0 || ins.third != 0.0 {
                w.insulation = Some(ins);
            }
        }
    }
    if let Some(rows) = tl_loss {
        check_rows(rows.len(), n_tls, 0x0e, "transmission line")?;
        for (t, &(db, second)) in tls.iter_mut().zip(&rows) {
            t.loss_db_per_m = db;
            t.loss_second = second;
        }
    }
    let l_networks = l_networks.unwrap_or_default();
    if l_networks.len() as i64 != n_lnets_hdr as i64 {
        notes.push(format!(
            "header 0x9C says {n_lnets_hdr} L networks, but chunk 0x11 has {}",
            l_networks.len()
        ));
    }

    // Wire numbers must refer to wires.
    let check_wire = |w: u16, what: &str| -> Result<(), EzError> {
        if w == 0 || w as usize > n_wires {
            Err(EzError::new(format!(
                "{what} is on wire {w}, but there are {n_wires} wires"
            )))
        } else {
            Ok(())
        }
    };
    for (i, s) in sources.iter().enumerate() {
        check_wire(s.wire, &format!("source {}", i + 1))?;
    }
    for (i, l) in loads.iter().enumerate() {
        check_wire(l.wire, &format!("load {}", i + 1))?;
    }
    for (i, t) in tls.iter().enumerate() {
        check_wire(t.wire_a, &format!("transmission line {} end 1", i + 1))?;
        if let TlEnd2::Wire { wire, .. } = t.end2 {
            check_wire(wire, &format!("transmission line {} end 2", i + 1))?;
        }
    }
    for (i, n) in l_networks.iter().enumerate() {
        for (p, port) in n.ports.iter().enumerate() {
            check_wire(port.wire, &format!("L network {} port {}", i + 1, p + 1))?;
        }
    }

    let ground = match ground_code {
        b'F' => Ground::FreeSpace,
        b'P' => Ground::Perfect,
        _ => Ground::Real {
            model: match ground_model_code {
                b'M' => RealGroundModel::Mininec,
                b'H' => RealGroundModel::HighAccuracy,
                c => return Err(bad_code(0x7B, "real ground model", c)),
            },
            media,
        },
    };

    let (unknown_fields, unclaimed) = b.into_report();
    Ok(EzFile {
        model: EzModel {
            title,
            frequency_mhz,
            units,
            wires,
            sources,
            loads,
            transmission_lines: tls,
            l_networks,
            ground,
            plot,
            sweep: options.flatten(),
            ref_level_dbi,
            alt_swr_z0,
            saved_by,
            engine,
        },
        report: DecodeReport {
            unknown_fields,
            unclaimed,
            notes,
        },
    })
}

fn count(b: &mut Bytes, off: usize, what: &str) -> Result<usize, EzError> {
    let n = b.i16(off)?;
    usize::try_from(n).map_err(|_| EzError::at(off, format!("{what} is {n}")))
}

fn bad_code(off: usize, what: &str, c: u8) -> EzError {
    let shown = if c.is_ascii_graphic() {
        format!("'{}'", c as char)
    } else {
        format!("{c:#04x}")
    };
    EzError::at(off, format!("unknown {what} code {shown}"))
}

fn wire_number(b: &mut Bytes, off: usize) -> Result<u16, EzError> {
    let w = b.i16(off)?;
    u16::try_from(w).map_err(|_| EzError::at(off, format!("wire number {w}")))
}

fn read_plot(b: &mut Bytes, notes: &mut Vec<String>) -> Result<Plot, EzError> {
    let kind = match b.u8(0x7A)? {
        b'A' => PlotKind::Azimuth,
        b'E' => PlotKind::Elevation,
        b'3' => PlotKind::ThreeD,
        b'2' => PlotKind::Code2,
        c => return Err(bad_code(0x7A, "plot type", c)),
    };
    let cut = match b.u8(0x09)? {
        b'A' => PlotCut::Azimuth,
        b'E' => PlotCut::Elevation,
        c => return Err(bad_code(0x09, "plot cut", c)),
    };
    let step_deg = b.f32(0x0E)?;
    let step_copy = b.f32(0x5A)?;
    if step_copy != step_deg {
        notes.push(format!("plot step is {step_deg} at 0x0E but {step_copy} at 0x5A"));
    }
    Ok(Plot {
        kind,
        cut,
        angle_deg: b.f32(0x0A)?,
        step_deg,
        range_deg: b.f32(0x4C)?,
        ground_wave_distance_m: b.f32(0x51)?,
    })
}

fn read_medium(b: &mut Bytes, off: usize) -> Result<Medium, EzError> {
    let [conductivity_s_per_m, rel_permittivity, r_coord_m, height_m] = b.f32s::<4>(off)?;
    Ok(Medium {
        conductivity_s_per_m,
        rel_permittivity,
        r_coord_m,
        height_m,
    })
}

fn read_wire(b: &mut Bytes, off: usize) -> Result<Wire, EzError> {
    let c = b.f32s::<6>(off)?;
    let diameter_m = b.f32(off + 0x18)?;
    let gauge = b.i16(off + 0x1C)?;
    let segments = b.i16(off + 0x1E)?;
    let segments = u16::try_from(segments)
        .ok()
        .filter(|&s| s > 0)
        .ok_or_else(|| EzError::at(off + 0x1E, format!("wire has {segments} segments")))?;
    Ok(Wire {
        end1: [c[0], c[1], c[2]],
        end2: [c[3], c[4], c[5]],
        diameter_m,
        awg: (gauge != -1).then_some(gauge),
        segments,
        resistivity_ohm_m: 0.0,
        loss_factor: 1.0,
        insulation: None,
    })
}

fn read_source(b: &mut Bytes, off: usize) -> Result<Source, EzError> {
    let wire = wire_number(b, off)?;
    let [position_pct, magnitude, phase_deg] = b.f32s::<3>(off + 2)?;
    let kind = match b.u8(off + 0x0E)? {
        b'V' => SourceKind::Voltage,
        b'I' => SourceKind::Current,
        b'W' => SourceKind::SplitVoltage,
        b'J' => SourceKind::SplitCurrent,
        c => return Err(bad_code(off + 0x0E, "source type", c)),
    };
    Ok(Source {
        wire,
        position_pct,
        magnitude,
        phase_deg,
        kind,
    })
}

/// `r` is the record start: a load row spans +0x2F…+0x6C and +0x85…+0x96.
fn read_load(b: &mut Bytes, r: usize, load_type: u8) -> Result<Load, EzError> {
    let wire = wire_number(b, r + R_LOAD)?;
    let [position_pct, zr, zx] = b.f32s::<3>(r + R_LOAD + 2)?;
    b.unknown(r + R_RLC, 1, "load row +0x85 ('S' when a load is set)")?;
    let value = match load_type {
        b'Z' => LoadValue::Impedance { r: zr, x: zx },
        b'R' => {
            let form = match b.u8(r + R_RLC + 1)? {
                b'S' => RlcForm::Series,
                b'P' => RlcForm::Parallel,
                b'T' => RlcForm::Trap,
                c => return Err(bad_code(r + R_RLC + 1, "RLC connection", c)),
            };
            let [rr, l_h, c_f, r_freq_mhz] = b.f32s::<4>(r + R_RLC + 2)?;
            LoadValue::Rlc {
                form,
                r: rr,
                l_h,
                c_f,
                r_freq_mhz,
            }
        }
        b'L' => LoadValue::Laplace {
            numerator: b.f32s::<6>(r + R_LAPLACE)?,
            denominator: b.f32s::<6>(r + R_LAPLACE + 24)?,
        },
        c => return Err(bad_code(0x75, "load type", c)),
    };
    Ok(Load {
        wire,
        position_pct,
        value,
        z_at_design: (zr, zx),
    })
}

fn read_tl(b: &mut Bytes, off: usize) -> Result<TransmissionLine, EzError> {
    let wire_a = wire_number(b, off)?;
    let position_a_pct = b.f32(off + 2)?;
    let wire_b = b.i16(off + 6)?;
    let [position_b_pct, z0_ohm, length_m, velocity_factor] = b.f32s::<4>(off + 8)?;
    let end2 = match wire_b {
        -1 => TlEnd2::ShortedStub,
        w => TlEnd2::Wire {
            wire: u16::try_from(w)
                .map_err(|_| EzError::at(off + 6, format!("transmission line end 2 wire number {w}")))?,
            position_pct: position_b_pct,
        },
    };
    Ok(TransmissionLine {
        wire_a,
        position_a_pct,
        end2,
        z0_ohm,
        length_m: (length_m != 0.0).then_some(length_m),
        velocity_factor,
        loss_db_per_m: 0.0,
        loss_second: 0.0,
    })
}

#[derive(Clone, Copy)]
struct Chunk {
    tag: u16,
    /// Data start, after the 4-byte chunk header.
    start: usize,
    end: usize,
}

impl Chunk {
    fn len(&self) -> usize {
        self.end - self.start
    }

    fn need(&self, rel_end: usize) -> Result<(), EzError> {
        if rel_end > self.len() {
            Err(EzError::at(
                self.start,
                format!(
                    "chunk {:#04x} needs {rel_end} data bytes but has {}",
                    self.tag,
                    self.len()
                ),
            ))
        } else {
            Ok(())
        }
    }

    fn done(&self, rel_end: usize) -> Result<(), EzError> {
        if rel_end != self.len() {
            Err(EzError::at(
                self.start,
                format!(
                    "chunk {:#04x} has {} data bytes, but its layout uses {rel_end}",
                    self.tag,
                    self.len()
                ),
            ))
        } else {
            Ok(())
        }
    }
}

/// Reads the `00 00 <rev>` prefix most chunks begin with.
fn chunk_prefix(b: &mut Bytes, c: &Chunk, revs: &[u8]) -> Result<u8, EzError> {
    c.need(3)?;
    let p = b.slice(c.start, 3)?;
    if p[0] != 0 || p[1] != 0 || !revs.contains(&p[2]) {
        return Err(EzError::at(
            c.start,
            format!(
                "chunk {:#04x} begins {:02x} {:02x} {:02x}, an unknown revision",
                c.tag, p[0], p[1], p[2]
            ),
        ));
    }
    Ok(p[2])
}

/// Reads `00 00 01`, an int32 row count, and checks the length for `row_len`-byte rows.
fn rows_chunk(b: &mut Bytes, c: &Chunk, row_len: usize) -> Result<usize, EzError> {
    chunk_prefix(b, c, &[1])?;
    c.need(7)?;
    let n = b.i32(c.start + 3)?;
    let n = usize::try_from(n).map_err(|_| EzError::at(c.start + 3, format!("row count {n}")))?;
    c.done(7 + n * row_len)?;
    Ok(n)
}

fn set_once<T>(slot: &mut Option<T>, c: Chunk, value: T) -> Result<(), EzError> {
    if slot.is_some() {
        return Err(EzError::at(c.start - 4, format!("chunk {:#04x} appears twice", c.tag)));
    }
    *slot = Some(value);
    Ok(())
}

fn check_rows(rows: usize, expected: usize, tag: u16, what: &str) -> Result<(), EzError> {
    if rows < expected {
        return Err(EzError::new(format!(
            "chunk {tag:#04x} has {rows} rows for {expected} {what}s"
        )));
    }
    Ok(())
}

/// Chunk `0x0b`: sweep settings and two strings (a sweep output file name, it seems).
fn read_options(b: &mut Bytes, c: &Chunk) -> Result<Option<Sweep>, EzError> {
    chunk_prefix(b, c, &[1])?;
    c.need(33)?;
    let s = c.start;
    b.unknown(s + 3, 4, "chunk 0x0b +3 (int32 16 in every file)")?;
    b.unknown(s + 7, 2, "chunk 0x0b +7")?;
    let len_a = b.i32(s + 9)?;
    let len_b = b.i32(s + 13)?;
    b.unknown(s + 17, 4, "chunk 0x0b +17")?;
    let [start_mhz, stop_mhz, step_mhz] = b.f32s::<3>(s + 21)?;
    let (Ok(len_a), Ok(len_b)) = (usize::try_from(len_a), usize::try_from(len_b)) else {
        return Err(EzError::at(
            s + 9,
            format!("chunk 0x0b string lengths {len_a}, {len_b}"),
        ));
    };
    let mut p = 33;
    c.need(p + len_a + len_b + 2)?;
    b.unknown(s + p, len_a, "chunk 0x0b string 1")?;
    p += len_a;
    b.unknown(s + p, len_b, "chunk 0x0b string 2")?;
    p += len_b;
    b.unknown(s + p, 2, "chunk 0x0b end (int16 12345 in every file)")?;
    c.done(p + 2)?;
    Ok(
        (start_mhz != 0.0 || stop_mhz != 0.0 || step_mhz != 0.0).then_some(Sweep {
            start_mhz,
            stop_mhz,
            step_mhz,
        }),
    )
}

/// Chunk `0x0c`: per wire εr, thickness (m) and a third value.
fn read_insulation(b: &mut Bytes, c: &Chunk) -> Result<Vec<Insulation>, EzError> {
    let n = rows_chunk(b, c, 12)?;
    (0..n)
        .map(|i| {
            let [rel_permittivity, thickness_m, third] = b.f32s::<3>(c.start + 7 + 12 * i)?;
            Ok(Insulation {
                rel_permittivity,
                thickness_m,
                third,
            })
        })
        .collect()
}

/// Chunks `0x0d` (resistivity, factor per wire) and `0x0e` (dB/m, second value per TL).
fn read_pairs(b: &mut Bytes, c: &Chunk) -> Result<Vec<(f32, f32)>, EzError> {
    let n = rows_chunk(b, c, 8)?;
    (0..n)
        .map(|i| {
            let [a, z] = b.f32s::<2>(c.start + 7 + 8 * i)?;
            Ok((a, z))
        })
        .collect()
}

/// Chunk `0x11`: L networks. Arrays hold 2n + 2 elements, where network k
/// (1-based) port p is element 2(k − 1) + p and the first and last are unused.
fn read_l_networks(b: &mut Bytes, c: &Chunk) -> Result<Vec<LNetwork>, EzError> {
    chunk_prefix(b, c, &[2])?;
    c.need(11)?;
    let s = c.start;
    let n = b.i32(s + 3)?;
    let n2 = b.i32(s + 7)?;
    if n != n2 || n < 0 {
        return Err(EzError::at(s + 3, format!("L network counts {n} and {n2}")));
    }
    let n = n as usize;
    let elems = 2 * n + 2;
    let wires_at = 11;
    let pcts_at = wires_at + 4 * elems;
    let z_at = pcts_at + 4 * elems + 12;
    let n3_at = z_at + 8 * 2 * n;
    let blocks_at = n3_at + 4 + 32;
    let conn_at = blocks_at + 32 * n + 2;
    c.done(conn_at + 2 * n)?;

    // The unused first and last array elements.
    for (off, label) in [
        (wires_at, "chunk 0x11 port wire element 0"),
        (wires_at + 4 * (elems - 1), "chunk 0x11 port wire last element"),
        (pcts_at, "chunk 0x11 port % element 0"),
        (pcts_at + 4 * (elems - 1), "chunk 0x11 port % last element"),
    ] {
        b.unknown(s + off, 4, label)?;
    }
    b.unknown(s + pcts_at + 4 * elems, 12, "chunk 0x11 12 bytes after the port %")?;
    let n3 = b.i32(s + n3_at)?;
    if n3 as usize != n {
        return Err(EzError::at(s + n3_at, format!("L network count {n3}, expected {n}")));
    }
    b.unknown(s + n3_at + 4, 32, "chunk 0x11 32 bytes before the networks")?;
    b.unknown(s + blocks_at + 32 * n, 2, "chunk 0x11 2 bytes before the connections")?;

    let mut nets = Vec::with_capacity(n);
    for k in 0..n {
        let block = b.f32s::<8>(s + blocks_at + 32 * k)?;
        let mut port = |p: usize| -> Result<LPort, EzError> {
            let e = 2 * k + p + 1; // array element
            let i = 2 * k + p; // index into the per-port lists
            let wire = b.i32(s + wires_at + 4 * e)?;
            let wire = u16::try_from(wire)
                .map_err(|_| EzError::at(s + wires_at + 4 * e, format!("L network port wire {wire}")))?;
            let position_pct = b.f32(s + pcts_at + 4 * e)?;
            let [zr, zx] = b.f32s::<2>(s + z_at + 8 * i)?;
            let connection = match b.u8(s + conn_at + i)? {
                b'S' => Connection::Series,
                b'P' => Connection::Parallel,
                c => return Err(bad_code(s + conn_at + i, "L network connection", c)),
            };
            Ok(LPort {
                wire,
                position_pct,
                r_ohm: block[p],
                l_h: block[2 + p],
                c_f: block[4 + p],
                r_freq_mhz: block[6 + p],
                connection,
                z_at_design: (zr, zx),
            })
        };
        let p1 = port(0)?;
        let p2 = port(1)?;
        nets.push(LNetwork { ports: [p1, p2] });
    }
    Ok(nets)
}

fn read_string(b: &mut Bytes, c: &Chunk, p: &mut usize) -> Result<String, EzError> {
    c.need(*p + 4)?;
    let n = b.i32(c.start + *p)?;
    let n = usize::try_from(n).map_err(|_| EzError::at(c.start + *p, format!("string length {n}")))?;
    *p += 4;
    c.need(*p + n)?;
    let s = latin1(b.slice(c.start + *p, n)?);
    *p += n;
    Ok(s)
}

/// Chunk `0x65`: save date, program name and version, and free text.
fn read_program(b: &mut Bytes, c: &Chunk, out: &mut SavedBy) -> Result<(), EzError> {
    chunk_prefix(b, c, &[1, 2])?;
    c.need(15)?;
    b.unknown(c.start + 3, 4, "chunk 0x65 +3")?;
    let date = b.f64(c.start + 7)?;
    let mut p = 15;
    let program = read_string(b, c, &mut p)?;
    let version = read_string(b, c, &mut p)?;
    c.need(p + 4)?;
    let code = b.u32(c.start + p)?;
    p += 4;
    let text = read_string(b, c, &mut p)?;
    c.done(p)?;
    out.ole_date = Some(date);
    out.program = Some(program);
    out.version = Some(version);
    let text = text.trim().to_string();
    out.text = (!text.is_empty()).then_some(text);
    if out.version_code == 0 {
        out.version_code = code;
    }
    Ok(())
}

/// Chunk `0x66`: the calculating engine's code, name and path.
fn read_engine(b: &mut Bytes, c: &Chunk) -> Result<CalcEngine, EzError> {
    chunk_prefix(b, c, &[2])?;
    c.need(7)?;
    let code = b.u32(c.start + 3)?;
    let mut p = 7;
    let name = read_string(b, c, &mut p)?;
    let path = read_string(b, c, &mut p)?;
    c.done(p)?;
    Ok(CalcEngine {
        kind: EngineKind::from_code(code),
        name,
        path,
    })
}
