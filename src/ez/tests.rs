//! Decoder tests. Every `.ez` image is built here in code, so no EZNEC file
//! is needed and none is in the repository.

#![allow(clippy::too_many_arguments, clippy::type_complexity, clippy::byte_char_slices)]

use super::*;

/// Builds an `.ez` image field by field.
struct Ez {
    header: Vec<u8>,
    records: Vec<Vec<u8>>,
    gap: Vec<u8>,
    chunks: Vec<(u16, Vec<u8>)>,
}

fn put(buf: &mut [u8], off: usize, bytes: &[u8]) {
    buf[off..off + bytes.len()].copy_from_slice(bytes);
}

fn floats(v: &[f32]) -> Vec<u8> {
    v.iter().flat_map(|x| x.to_le_bytes()).collect()
}

impl Ez {
    /// One wire along Y at 3.048 m height with a voltage source at its centre,
    /// free space, display unit feet, as an old (pre-v6) save.
    fn base() -> Ez {
        let mut e = Ez {
            header: vec![0; HEADER_LEN],
            records: Vec::new(),
            gap: vec![0; GAP_LEN],
            chunks: Vec::new(),
        };
        e.h16(0x00, 1).h16(0x30, 1).h16(0x32, 1);
        e.hf(0x05, 14.2)
            .h8(0x09, b'A')
            .hf(0x0A, 23.0)
            .hf(0x0E, 1.0)
            .hf(0x5A, 1.0)
            .hf(0x4C, 360.0);
        put(&mut e.header, 0x12, format!("{:<30}", "test model").as_bytes());
        e.h8(0x36, b'F')
            .h8(0x40, FORMAT_BYTE)
            .h8(0x41, b'F')
            .h8(0x75, b'Z')
            .h8(0x7A, b'A')
            .h8(0x7B, b'M');
        e.hf(0x66, 1.0).hf(0x6E, 75.0);
        e.wire(0, [0.0, -1.524, 3.048], [0.0, 1.524, 3.048], 0.002, 21);
        e.source(0, 1, 50.0, 1.0, 0.0, b'V');
        e
    }

    fn h8(&mut self, off: usize, v: u8) -> &mut Self {
        self.header[off] = v;
        self
    }
    fn h16(&mut self, off: usize, v: i16) -> &mut Self {
        put(&mut self.header, off, &v.to_le_bytes());
        self
    }
    fn h32(&mut self, off: usize, v: i32) -> &mut Self {
        put(&mut self.header, off, &v.to_le_bytes());
        self
    }
    fn hf(&mut self, off: usize, v: f32) -> &mut Self {
        put(&mut self.header, off, &v.to_le_bytes());
        self
    }

    /// Record `k`, growing the record table (and the record count) to hold it.
    fn rec(&mut self, k: usize) -> &mut Vec<u8> {
        while self.records.len() <= k {
            self.records.push(vec![0; RECORD_LEN]);
        }
        let n = self.records.len() as i16;
        self.h16(0x00, n);
        &mut self.records[k]
    }

    fn wire(&mut self, k: usize, a: [f32; 3], b: [f32; 3], dia: f32, segs: i16) -> &mut Self {
        let r = self.rec(k);
        put(r, 0x00, &floats(&[a[0], a[1], a[2], b[0], b[1], b[2], dia]));
        put(r, 0x1C, &(-1i16).to_le_bytes());
        put(r, 0x1E, &segs.to_le_bytes());
        self
    }

    fn source(&mut self, k: usize, wire: i16, pct: f32, mag: f32, phase: f32, kind: u8) -> &mut Self {
        let r = self.rec(k);
        put(r, 0x20, &wire.to_le_bytes());
        put(r, 0x22, &floats(&[pct, mag, phase]));
        r[0x2E] = kind;
        self
    }

    fn load(&mut self, k: usize, wire: i16, pct: f32, zr: f32, zx: f32) -> &mut Self {
        let r = self.rec(k);
        put(r, 0x2F, &wire.to_le_bytes());
        put(r, 0x31, &floats(&[pct, zr, zx]));
        r[0x85] = b'S';
        self
    }

    fn tl(&mut self, k: usize, wa: i16, pa: f32, wb: i16, pb: f32, z0: f32, len: f32, vf: f32) -> &mut Self {
        let r = self.rec(k);
        put(r, 0x6D, &wa.to_le_bytes());
        put(r, 0x6F, &pa.to_le_bytes());
        put(r, 0x73, &wb.to_le_bytes());
        put(r, 0x75, &floats(&[pb, z0, len, vf]));
        self
    }

    fn chunk(&mut self, tag: u16, data: Vec<u8>) -> &mut Self {
        self.chunks.push((tag, data));
        self
    }

    /// A `00 00 01`, int32 count, rows chunk.
    fn rows(&mut self, tag: u16, n: i32, rows: &[f32]) -> &mut Self {
        let mut d = vec![0, 0, 1];
        d.extend(n.to_le_bytes());
        d.extend(floats(rows));
        self.chunk(tag, d)
    }

    fn bytes(&self) -> Vec<u8> {
        let mut out = self.header.clone();
        for r in &self.records {
            out.extend(r);
        }
        out.extend(&self.gap);
        for (tag, data) in &self.chunks {
            out.extend(tag.to_le_bytes());
            out.extend(((data.len() + 4) as u16).to_le_bytes());
            out.extend(data);
        }
        out
    }

    fn read(&self) -> EzFile {
        read_ez(&self.bytes()).expect("decodes")
    }

    fn err(&self) -> EzError {
        read_ez(&self.bytes()).expect_err("is refused")
    }
}

fn string(s: &str) -> Vec<u8> {
    let mut d = (s.len() as i32).to_le_bytes().to_vec();
    d.extend(s.as_bytes());
    d
}

#[test]
fn base_model_decodes() {
    let f = Ez::base().read();
    let m = &f.model;
    assert_eq!(m.title, "test model");
    assert_eq!(m.frequency_mhz, 14.2);
    assert_eq!(m.units, LengthUnit::Feet);
    assert_eq!(m.ground, Ground::FreeSpace);
    assert_eq!(m.wires.len(), 1);
    let w = &m.wires[0];
    assert_eq!((w.end1, w.end2), ([0.0, -1.524, 3.048], [0.0, 1.524, 3.048]));
    assert_eq!((w.diameter_m, w.awg, w.segments), (0.002, None, 21));
    assert_eq!((w.resistivity_ohm_m, w.loss_factor), (0.0, 1.0));
    assert_eq!(
        m.sources,
        vec![Source {
            wire: 1,
            position_pct: 50.0,
            magnitude: 1.0,
            phase_deg: 0.0,
            kind: SourceKind::Voltage
        }]
    );
    assert!(m.loads.is_empty() && m.transmission_lines.is_empty() && m.l_networks.is_empty());
    assert_eq!(m.plot.kind, PlotKind::Azimuth);
    assert_eq!(
        (m.plot.angle_deg, m.plot.step_deg, m.plot.range_deg),
        (23.0, 1.0, 360.0)
    );
    assert_eq!(m.sweep, None);
    assert_eq!(m.alt_swr_z0, 75.0);
    assert_eq!(m.saved_by.version_text(), None);
    assert!(f.report.unclaimed.is_empty(), "{:?}", f.report.unclaimed);
    assert!(f.report.notes.is_empty(), "{:?}", f.report.notes);
}

#[test]
fn unknown_layouts_are_refused() {
    assert!(read_ez(&[0u8; 20]).is_err());

    let mut e = Ez::base();
    e.h8(0x40, 6);
    assert_eq!(e.err().offset, Some(0x40));

    // A record count the file is too short for.
    let mut bytes = Ez::base().bytes();
    bytes[0] = 9;
    assert!(read_ez(&bytes).is_err());

    // More wires than records.
    let mut e = Ez::base();
    e.h16(0x30, 2);
    assert!(e.err().message.contains("2 wires"));

    // A chunk that runs past the end, and a stray byte after the chunks.
    let mut bytes = Ez::base().bytes();
    bytes.extend([0x0c, 0, 0x40, 0]);
    assert!(read_ez(&bytes).is_err());
    let mut bytes = Ez::base().bytes();
    bytes.push(0);
    assert!(read_ez(&bytes).is_err());
}

#[test]
fn unknown_codes_are_errors() {
    for (off, code) in [(0x36, b'Q'), (0x41, b'Q'), (0x7A, b'Q'), (0x09, b'Q')] {
        let mut e = Ez::base();
        e.h8(off, code);
        assert_eq!(e.err().offset, Some(off), "offset {off:#x}");
    }
    let mut e = Ez::base();
    e.source(0, 1, 50.0, 1.0, 0.0, b'Q');
    assert_eq!(e.err().offset, Some(HEADER_LEN + 0x2E));

    // A source on a wire that doesn't exist.
    let mut e = Ez::base();
    e.source(0, 2, 50.0, 1.0, 0.0, b'V');
    assert!(e.err().message.contains("wire 2"));
}

#[test]
fn source_kinds() {
    for (code, kind) in [
        (b'V', SourceKind::Voltage),
        (b'I', SourceKind::Current),
        (b'W', SourceKind::SplitVoltage),
        (b'J', SourceKind::SplitCurrent),
    ] {
        let mut e = Ez::base();
        e.source(0, 1, 23.0, 0.75, 37.0, code);
        let s = &e.read().model.sources[0];
        assert_eq!(
            (s.kind, s.position_pct, s.magnitude, s.phase_deg),
            (kind, 23.0, 0.75, 37.0)
        );
    }
}

#[test]
fn rows_past_a_count_are_ignored() {
    // Record 1 exists for a second source, and holds a stale copy of wire 1.
    let mut e = Ez::base();
    e.source(1, 1, 23.0, 0.75, 37.0, b'V').h16(0x32, 2);
    e.wire(1, [9.0, 9.0, 9.0], [8.0, 8.0, 8.0], 0.5, 3);
    let f = e.read();
    assert_eq!(f.model.wires.len(), 1);
    assert_eq!(f.model.sources.len(), 2);
    assert!(f.report.unclaimed.is_empty());
}

#[test]
fn unaccounted_bytes_are_reported() {
    let mut e = Ez::base();
    e.h8(0x48, 0x7F); // nothing is known to live here
    let f = e.read();
    assert_eq!(
        f.report.unclaimed,
        vec![Unclaimed {
            offset: 0x48,
            bytes: vec![0x7F]
        }]
    );

    // A placed-but-undecoded field is listed with its label instead.
    let mut e = Ez::base();
    e.h8(0x04, b'R');
    let f = e.read();
    assert!(f.report.unclaimed.is_empty());
    assert!(
        f.report
            .unknown_fields
            .iter()
            .any(|u| u.offset == 0x04 && u.bytes == [b'R'])
    );
}

#[test]
fn real_ground_media() {
    let mut e = Ez::base();
    e.h8(0x36, b'R')
        .h8(0x7B, b'H')
        .h16(0x02, 1)
        .hf(0xAA, 0.0123)
        .hf(0xAE, 17.0);
    assert_eq!(
        e.read().model.ground,
        Ground::Real {
            model: RealGroundModel::HighAccuracy,
            media: vec![Medium {
                conductivity_s_per_m: 0.0123,
                rel_permittivity: 17.0,
                r_coord_m: 0.0,
                height_m: 0.0
            }],
        }
    );

    // The second medium is in record 0.
    e.h8(0x7B, b'M').h16(0x02, 2);
    put(e.rec(0), 0x9A, &floats(&[0.0045, 17.0, 5.0, -0.6096]));
    let Ground::Real { model, media } = e.read().model.ground else {
        panic!()
    };
    assert_eq!(model, RealGroundModel::Mininec);
    assert_eq!(media.len(), 2);
    assert_eq!(
        media[1],
        Medium {
            conductivity_s_per_m: 0.0045,
            rel_permittivity: 17.0,
            r_coord_m: 5.0,
            height_m: -0.6096
        }
    );

    // Real ground needs a model code and at least one medium.
    e.h8(0x7B, b'F');
    assert_eq!(e.err().offset, Some(0x7B));
    e.h8(0x7B, b'M').h16(0x02, 0);
    assert_eq!(e.err().offset, Some(0x02));

    let mut e = Ez::base();
    e.h8(0x36, b'P').hf(0xAA, 0.005).hf(0xAE, 13.0); // stale constants
    let f = e.read();
    assert_eq!(f.model.ground, Ground::Perfect);
    assert!(f.report.unclaimed.is_empty());
}

#[test]
fn loads() {
    let mut e = Ez::base();
    e.h16(0x34, 1).load(0, 1, 30.0, 12.5, -34.5);
    let l = &e.read().model.loads[0];
    assert_eq!((l.wire, l.position_pct), (1, 30.0));
    assert_eq!(l.value, LoadValue::Impedance { r: 12.5, x: -34.5 });

    // Old files leave the load type blank for R + jX.
    e.h8(0x75, b' ');
    assert_eq!(
        e.read().model.loads[0].value,
        LoadValue::Impedance { r: 12.5, x: -34.5 }
    );

    for (code, form) in [
        (b'S', RlcForm::Series),
        (b'P', RlcForm::Parallel),
        (b'T', RlcForm::Trap),
    ] {
        let mut e = Ez::base();
        e.h16(0x34, 1).h8(0x75, b'R').load(0, 1, 30.0, 1.5, -138.89);
        e.rec(0)[0x86] = code;
        put(e.rec(0), 0x87, &floats(&[1.5, 2.25e-6, 33e-12, 21.0]));
        let f = e.read();
        let l = &f.model.loads[0];
        assert_eq!(
            l.value,
            LoadValue::Rlc {
                form,
                r: 1.5,
                l_h: 2.25e-6,
                c_f: 33e-12,
                r_freq_mhz: 21.0
            }
        );
        assert_eq!(l.z_at_design, (1.5, -138.89));
        assert!(f.report.unclaimed.is_empty());
    }

    let mut e = Ez::base();
    e.h16(0x34, 1).h8(0x75, b'L').load(0, 1, 30.0, 0.9167, 0.0);
    let coeffs: Vec<f32> = (1..=12).map(|i| i as f32).collect();
    put(e.rec(0), 0x3D, &floats(&coeffs));
    assert_eq!(
        e.read().model.loads[0].value,
        LoadValue::Laplace {
            numerator: [1.0, 2.0, 3.0, 4.0, 5.0, 6.0],
            denominator: [7.0, 8.0, 9.0, 10.0, 11.0, 12.0]
        }
    );

    let mut e = Ez::base();
    e.h16(0x34, 1).h8(0x75, b'Q').load(0, 1, 30.0, 1.0, 1.0);
    assert_eq!(e.err().offset, Some(0x75));
}

#[test]
fn transmission_lines() {
    let mut e = Ez::base();
    e.wire(1, [0.3048, -1.524, 3.048], [0.3048, 1.524, 3.048], 0.002, 21)
        .h16(0x30, 2);
    e.h16(0x78, 2);
    e.tl(0, 1, 50.0, 2, 50.0, -75.0, 0.0, 0.66);
    e.tl(1, 1, 50.0, -1, 0.0, 450.0, 1.2192, 1.0);
    e.rows(0x0e, 2, &[0.0, 0.0, 0.049212597, 0.0]);
    let f = e.read();
    let t = &f.model.transmission_lines;
    assert_eq!(
        t[0].end2,
        TlEnd2::Wire {
            wire: 2,
            position_pct: 50.0
        }
    );
    assert!(t[0].reversed());
    assert_eq!((t[0].z0_ohm, t[0].length_m, t[0].velocity_factor), (-75.0, None, 0.66));
    assert_eq!(t[0].loss_db_per_m, 0.0);
    assert_eq!(t[1].end2, TlEnd2::ShortedStub);
    assert!(!t[1].reversed());
    assert_eq!(t[1].length_m, Some(1.2192));
    assert_eq!(t[1].loss_db_per_m, 0.049212597);
    assert!(f.report.unclaimed.is_empty());

    // Any other negative end-2 wire is not understood.
    e.tl(1, 1, 50.0, -2, 0.0, 450.0, 1.2192, 1.0);
    assert_eq!(e.err().offset, Some(HEADER_LEN + RECORD_LEN + 0x73));
}

#[test]
fn wire_loss_and_insulation() {
    // Header resistivity applies to every wire when there is no chunk.
    let mut e = Ez::base();
    e.hf(0x62, 4.0e-8);
    assert_eq!(e.read().model.wires[0].resistivity_ohm_m, 4.0e-8);

    // Chunk 0x0d gives per-wire values and wins.
    let mut e = Ez::base();
    e.wire(1, [0.0, 1.524, 3.048], [0.0, 2.4, 2.1], 0.002, 9).h16(0x30, 2);
    e.rows(0x0d, 2, &[1.74e-8, 1.0, 4.0e-8, 1.0]);
    e.rows(0x0c, 2, &[1.0, 0.0, 0.0, 2.3, 0.0005, 0.0]);
    let f = e.read();
    let w = &f.model.wires;
    assert_eq!((w[0].resistivity_ohm_m, w[1].resistivity_ohm_m), (1.74e-8, 4.0e-8));
    assert_eq!(w[0].insulation, None);
    assert_eq!(
        w[1].insulation,
        Some(Insulation {
            rel_permittivity: 2.3,
            thickness_m: 0.0005,
            third: 0.0
        })
    );
    assert!(f.report.notes.is_empty());

    // Too few rows for the wires is refused; so is a length that doesn't fit the rows.
    let mut e = Ez::base();
    e.rows(0x0d, 0, &[]);
    assert!(e.err().message.contains("0x0d"));
    let mut e = Ez::base();
    e.rows(0x0d, 1, &[1.0]);
    assert!(e.err().message.contains("0x0d"));
    let mut e = Ez::base();
    e.rows(0x0d, 1, &[0.0, 1.0]).rows(0x0d, 1, &[0.0, 1.0]);
    assert!(e.err().message.contains("twice"));
}

/// Chunk 0x11 for the given networks: per port (wire, %, R, L, C, R Freq, connection, Zr, Zx).
fn lnet_chunk(nets: &[[(i32, f32, f32, f32, f32, f32, u8, f32, f32); 2]]) -> Vec<u8> {
    let n = nets.len() as i32;
    let mut d = vec![0, 0, 2];
    d.extend(n.to_le_bytes());
    d.extend(n.to_le_bytes());
    d.extend(0i32.to_le_bytes());
    for net in nets {
        for p in net {
            d.extend(p.0.to_le_bytes());
        }
    }
    d.extend(0i32.to_le_bytes());
    d.extend(0f32.to_le_bytes());
    for net in nets {
        for p in net {
            d.extend(p.1.to_le_bytes());
        }
    }
    d.extend(0f32.to_le_bytes());
    d.extend([0; 12]);
    for net in nets {
        for p in net {
            d.extend(floats(&[p.7, p.8]));
        }
    }
    d.extend(n.to_le_bytes());
    d.extend([0; 32]);
    for net in nets {
        let (a, b) = (&net[0], &net[1]);
        d.extend(floats(&[a.2, b.2, a.3, b.3, a.4, b.4, a.5, b.5]));
    }
    d.extend([0; 2]);
    for net in nets {
        d.extend([net[0].6, net[1].6]);
    }
    d
}

#[test]
fn l_networks() {
    let mut e = Ez::base();
    e.wire(1, [0.0, 1.524, 3.048], [0.0, 2.4, 2.1], 0.002, 9);
    e.wire(2, [0.6, -1.524, 3.048], [0.6, 1.524, 3.048], 0.002, 21);
    e.h16(0x30, 3).h32(0x9C, 2);
    let chunk = lnet_chunk(&[
        [
            (3, 50.0, 4.7, 2.25e-6, 33e-12, 7.0, b'S', 6.69, -138.9),
            (2, 50.0, 0.0, 0.0, 1e-10, 0.0, b'P', 0.0, -112.1),
        ],
        [
            (2, 30.0, 2.2, 1.5e-6, 47e-12, 0.0, b'S', 2.2, -104.6),
            (3, 70.0, 0.0, 0.0, 0.0, 0.0, b'S', 0.0, 0.0),
        ],
    ]);
    assert_eq!(chunk.len() + 4, 213, "the length EZNEC v7 writes for two networks");
    e.chunk(0x11, chunk);
    let f = e.read();
    let n = &f.model.l_networks;
    assert_eq!(n.len(), 2);
    assert_eq!(
        n[0].ports[0],
        LPort {
            wire: 3,
            position_pct: 50.0,
            r_ohm: 4.7,
            l_h: 2.25e-6,
            c_f: 33e-12,
            r_freq_mhz: 7.0,
            connection: Connection::Series,
            z_at_design: (6.69, -138.9),
        }
    );
    assert_eq!(
        (n[0].ports[1].wire, n[0].ports[1].c_f, n[0].ports[1].connection),
        (2, 1e-10, Connection::Parallel)
    );
    assert_eq!(
        (n[1].ports[0].wire, n[1].ports[0].position_pct, n[1].ports[0].l_h),
        (2, 30.0, 1.5e-6)
    );
    assert_eq!((n[1].ports[1].wire, n[1].ports[1].position_pct), (3, 70.0));
    assert!(f.report.unclaimed.is_empty() && f.report.notes.is_empty());

    // One network is 147 bytes.
    let one = lnet_chunk(&[[
        (1, 50.0, 0.0, 2.25e-6, 33e-12, 0.0, b'S', 0.0, 0.0),
        (1, 30.0, 0.0, 0.0, 0.0, 0.0, b'S', 0.0, 0.0),
    ]]);
    assert_eq!(one.len() + 4, 147);
}

fn options_chunk(a: &str, b: &str, sweep: [f32; 3]) -> Vec<u8> {
    let mut d = vec![0, 0, 1];
    d.extend(16i32.to_le_bytes());
    d.extend([0, 0]);
    d.extend((a.len() as i32).to_le_bytes());
    d.extend((b.len() as i32).to_le_bytes());
    d.extend([0; 4]);
    d.extend(floats(&sweep));
    d.extend(a.as_bytes());
    d.extend(b.as_bytes());
    d.extend(12345i16.to_le_bytes());
    d
}

#[test]
fn sweep_and_saved_by() {
    let mut e = Ez::base();
    e.chunk(0x0b, options_chunk("x", "", [0.0; 3]));
    assert_eq!(e.read().model.sweep, None);

    let mut e = Ez::base();
    e.chunk(0x0b, options_chunk("G:\\model", "$Sweep$.txt", [14.0, 14.3, 0.05]));
    e.h32(0x90, 70_000_004).h32(0x7C, 1).h32(0x80, 1);
    let mut prog = vec![0, 0, 2];
    prog.extend(2i32.to_le_bytes());
    prog.extend(44000.5f64.to_le_bytes());
    prog.extend(string("EZNEC Pro/2+"));
    prog.extend(string("7.0.4"));
    prog.extend(70_000_004i32.to_le_bytes());
    prog.extend(string("\r\nCalc Engine = Ext NEC-5\r\n"));
    e.chunk(0x65, prog);
    let mut eng = vec![0, 0, 2];
    eng.extend(8i32.to_le_bytes());
    eng.extend(string("Ext NEC-5"));
    eng.extend(string("C:\\nec\\nec5.exe"));
    e.chunk(0x66, eng);
    e.chunk(0x77, vec![1, 2, 3]); // a tag no one knows

    let f = e.read();
    let m = &f.model;
    assert_eq!(
        m.sweep,
        Some(Sweep {
            start_mhz: 14.0,
            stop_mhz: 14.3,
            step_mhz: 0.05
        })
    );
    assert_eq!(m.saved_by.program.as_deref(), Some("EZNEC Pro/2+"));
    assert_eq!(m.saved_by.version_text().as_deref(), Some("7.0.4"));
    assert_eq!(m.saved_by.ole_date, Some(44000.5));
    assert_eq!(m.saved_by.text.as_deref(), Some("Calc Engine = Ext NEC-5"));
    assert_eq!(
        m.engine,
        Some(CalcEngine {
            kind: EngineKind::ExtNec5,
            name: "Ext NEC-5".into(),
            path: "C:\\nec\\nec5.exe".into()
        })
    );
    assert!(f.report.unclaimed.is_empty());
    assert!(
        f.report
            .unknown_fields
            .iter()
            .any(|u| u.label.contains("0x77") && u.bytes == [1, 2, 3])
    );
}

#[test]
fn version_text_from_the_header_code() {
    for (code, text) in [(60_000_025, "6.0.25"), (50_000_001, "5.0.1"), (40_000_035, "4.0.35")] {
        let mut e = Ez::base();
        e.h32(0x90, code);
        assert_eq!(e.read().model.saved_by.version_text().as_deref(), Some(text));
    }
}

#[test]
fn copies_that_disagree_are_noted() {
    let mut e = Ez::base();
    e.hf(0x5A, 3.0);
    let f = e.read();
    assert_eq!(f.model.plot.step_deg, 1.0);
    assert_eq!(f.report.notes.len(), 1);
}

#[test]
fn dump_shows_the_display_unit() {
    let text = crate::dump::dump(&Ez::base().read());
    assert!(text.contains("(0, -5, 10) to (0, 5, 10)"), "{text}");
    assert!(text.contains("wire 1 at 50% (segment 11)"), "{text}");
    assert!(text.contains("Unaccounted non-zero bytes (0)"), "{text}");
}
