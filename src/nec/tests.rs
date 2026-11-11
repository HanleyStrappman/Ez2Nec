//! Deck writer tests. Expected cards come from EZNEC's own exports and from
//! runs on the engines; see the comments on each rule in `write.rs`.

use super::write::*;
use crate::ez::*;

const FT: f32 = 0.3048;

fn wire(a: [f32; 3], b: [f32; 3], segments: u16) -> Wire {
    Wire {
        end1: a.map(|v| v * FT),
        end2: b.map(|v| v * FT),
        diameter_m: 0.002,
        awg: None,
        segments,
        resistivity_ohm_m: 0.0,
        loss_factor: 1.0,
        insulation: None,
    }
}

fn source(wire: u16, pct: f32, kind: SourceKind) -> Source {
    Source {
        wire,
        position_pct: pct,
        magnitude: 1.0,
        phase_deg: 0.0,
        kind,
    }
}

/// A 10 ft dipole along Y at 10 ft, 21 segments, fed at the centre, in free space.
fn base() -> EzModel {
    EzModel {
        title: "test".into(),
        frequency_mhz: 14.2,
        units: LengthUnit::Feet,
        wires: vec![wire([0.0, -5.0, 10.0], [0.0, 5.0, 10.0], 21)],
        sources: vec![source(1, 50.0, SourceKind::Voltage)],
        loads: vec![],
        transmission_lines: vec![],
        l_networks: vec![],
        ground: Ground::FreeSpace,
        plot: Plot {
            kind: PlotKind::Azimuth,
            cut: PlotCut::Azimuth,
            angle_deg: 23.0,
            step_deg: 1.0,
            range_deg: 360.0,
            ground_wave_distance_m: 0.0,
        },
        sweep: None,
        ref_level_dbi: 0.0,
        alt_swr_z0: 75.0,
        saved_by: SavedBy::default(),
        engine: None,
    }
}

fn deck(m: &EzModel, engine: Engine) -> Deck {
    write_deck(
        m,
        &Options {
            engine,
            source_name: Some("test.ez".into()),
        },
    )
    .expect("converts")
}

/// The deck's cards with the given code, without the code.
fn cards(d: &Deck, code: &str) -> Vec<String> {
    d.text
        .lines()
        .filter(|l| l.split(' ').next() == Some(code))
        .map(|l| l[code.len()..].trim().to_string())
        .collect()
}

fn mininec() -> Ground {
    Ground::Real {
        model: RealGroundModel::Mininec,
        media: vec![Medium {
            conductivity_s_per_m: 0.005,
            rel_permittivity: 13.0,
            r_coord_m: 0.0,
            height_m: 0.0,
        }],
    }
}

const ENGINES: [Engine; 3] = [Engine::Nec2, Engine::Nec4, Engine::Nec5];

#[test]
fn engine_names_parse() {
    assert_eq!(Engine::parse("nec2"), Some(Engine::Nec2));
    assert_eq!(Engine::parse("NEC-4.2"), Some(Engine::Nec4));
    assert_eq!(Engine::parse("nec4"), Some(Engine::Nec4));
    assert_eq!(Engine::parse("5"), Some(Engine::Nec5));
    assert_eq!(Engine::parse("nec3"), None);
}

#[test]
fn base_deck_in_free_space() {
    for engine in ENGINES {
        let d = deck(&base(), engine);
        let lines: Vec<&str> = d.text.lines().collect();
        assert_eq!(lines[0], "CM test");
        assert!(
            lines[1].starts_with("CM ez2nec") && lines[1].contains("test.ez"),
            "{}",
            lines[1]
        );
        // Geometry in the display unit, with GS to metres.
        assert_eq!(cards(&d, "GW"), ["1 21 0 -5 10 0 5 10 0.00328084"]);
        assert_eq!(cards(&d, "GS"), ["0 0 0.3048"]);
        assert_eq!(cards(&d, "GE"), ["0"]);
        assert_eq!(cards(&d, "FR"), ["0 1 0 0 14.2 0"]);
        assert!(cards(&d, "GN").is_empty() && cards(&d, "GD").is_empty());
        // 1 V RMS is 1.414214 V peak, on the centre segment.
        assert_eq!(cards(&d, "EX"), ["0 1 11 0 1.414214 0"]);
        assert_eq!(cards(&d, "RP"), ["0 1 361 1000 67 0 0 1"]);
        assert_eq!(*lines.last().unwrap(), "EN");
        // Card order: comments, geometry, GE, then program control.
        let order: Vec<&str> = lines.iter().map(|l| &l[..2]).filter(|c| *c != "CM").collect();
        assert_eq!(order, ["CE", "GW", "GS", "GE", "FR", "EX", "RP", "EN"]);
    }
}

#[test]
fn units() {
    let mut m = base();
    m.units = LengthUnit::Millimetres;
    let d = deck(&m, Engine::Nec4);
    assert_eq!(cards(&d, "GW"), ["1 21 0 -1524 3048 0 1524 3048 1"]);
    assert_eq!(cards(&d, "GS"), ["0 0 0.001"]);
    // Wavelength units are written in metres, with no GS.
    m.units = LengthUnit::Wavelengths;
    let d = deck(&m, Engine::Nec4);
    assert_eq!(cards(&d, "GW"), ["1 21 0 -1.524 3.048 0 1.524 3.048 0.001"]);
    assert!(cards(&d, "GS").is_empty());
}

#[test]
fn source_phase_and_magnitude() {
    let mut m = base();
    m.sources = vec![Source {
        wire: 1,
        position_pct: 23.0,
        magnitude: 0.75,
        phase_deg: 90.0,
        kind: SourceKind::Voltage,
    }];
    // 23% of 21 segments is segment 5; 0.75 V RMS at 90 degrees is j1.06066 peak.
    let ex = cards(&deck(&m, Engine::Nec4), "EX");
    let f: Vec<f64> = ex[0].split(' ').map(|x| x.parse().unwrap()).collect();
    assert_eq!(&f[..4], [0.0, 1.0, 5.0, 0.0]);
    assert!(f[4].abs() < 1e-9 && (f[5] - 1.06066).abs() < 1e-6);
}

#[test]
fn current_source_per_engine() {
    let mut m = base();
    m.sources[0].kind = SourceKind::Current;

    // NEC-4.2 and NEC-5 have a current source of their own.
    assert_eq!(cards(&deck(&m, Engine::Nec4), "EX"), ["6 1 11 0 1.414214 0"]);
    assert_eq!(cards(&deck(&m, Engine::Nec5), "EX"), ["4 1 11 0 1.414214 0"]);

    // NEC-2 drives it from a dummy wire: jI volts, and a network with Y12 = j1 S.
    let d = deck(&m, Engine::Nec2);
    assert_eq!(cards(&d, "EX"), ["0 9901 1 0 0 1.414214"]);
    assert_eq!(cards(&d, "NT"), ["9901 1 1 11 0 0 0 1 0 0"]);
    let gw = cards(&d, "GW");
    assert_eq!(gw.len(), 2);
    assert!(gw[1].starts_with("9901 1 "), "{}", gw[1]);
    assert!(d.notes.iter().any(|n| n.contains("no current source")));
}

#[test]
fn split_source_inside_a_wire() {
    let mut m = base();
    m.sources[0].kind = SourceKind::SplitVoltage;
    // 50% of 21: junction 11, so segments 11 and 12 with half the voltage each.
    assert_eq!(
        cards(&deck(&m, Engine::Nec2), "EX"),
        ["0 1 11 0 0.7071068 0", "0 1 12 0 0.7071068 0"]
    );
    // 50% of 20: segments 10 and 11. 30% of 21: segments 6 and 7.
    m.wires[0].segments = 20;
    assert_eq!(
        cards(&deck(&m, Engine::Nec4), "EX"),
        ["0 1 10 0 0.7071068 0", "0 1 11 0 0.7071068 0"]
    );
    m.wires[0].segments = 21;
    m.sources[0].position_pct = 30.0;
    assert_eq!(
        cards(&deck(&m, Engine::Nec4), "EX"),
        ["0 1 6 0 0.7071068 0", "0 1 7 0 0.7071068 0"]
    );

    // A split current source puts the full current on each side.
    m.sources[0].kind = SourceKind::SplitCurrent;
    assert_eq!(
        cards(&deck(&m, Engine::Nec4), "EX"),
        ["6 1 6 0 1.414214 0", "6 1 7 0 1.414214 0"]
    );

    // NEC-5 feeds the junction itself, with the whole value.
    m.sources[0].kind = SourceKind::SplitVoltage;
    let d = deck(&m, Engine::Nec5);
    assert_eq!(cards(&d, "EX"), ["0 1 6 0 1.414214 0"]);
}

#[test]
fn split_source_at_a_wire_end() {
    // Wire 2 starts where wire 1 ends (EZNEC's split-start test).
    let mut m = base();
    m.wires.push(wire([0.0, 5.0, 10.0], [0.0, 8.0, 7.0], 9));
    m.sources = vec![source(2, 0.0, SourceKind::SplitVoltage)];
    // The source's own wire first, then the wire joined there; same sign,
    // since current runs the same way through both.
    assert_eq!(
        cards(&deck(&m, Engine::Nec2), "EX"),
        ["0 2 1 0 0.7071068 0", "0 1 21 0 0.7071068 0"]
    );
    assert_eq!(cards(&deck(&m, Engine::Nec5), "EX"), ["0 2 -1 0 1.414214 0"]);

    // At the end of wire 1 instead: segments 21 and wire 2's first.
    m.sources = vec![source(1, 100.0, SourceKind::SplitVoltage)];
    assert_eq!(
        cards(&deck(&m, Engine::Nec4), "EX"),
        ["0 1 21 0 0.7071068 0", "0 2 1 0 0.7071068 0"]
    );
    assert_eq!(cards(&deck(&m, Engine::Nec5), "EX"), ["0 1 21 0 1.414214 0"]);

    // Two wires that both start at the junction run opposite ways through it,
    // so the second half's sign flips (EZNEC's export of wa1x70ssb).
    let mut m = base();
    m.wires = vec![
        wire([0.0, 0.0, 30.0], [-10.0, 30.0, 30.0], 19),
        wire([0.0, 0.0, 30.0], [-10.0, -30.0, 30.0], 19),
    ];
    m.sources = vec![source(1, 0.0, SourceKind::SplitCurrent)];
    assert_eq!(
        cards(&deck(&m, Engine::Nec4), "EX"),
        ["6 1 1 0 1.414214 0", "6 2 1 0 -1.414214 0"]
    );
    let d = deck(&m, Engine::Nec2);
    assert_eq!(cards(&d, "EX"), ["0 9901 1 0 0 1.414214", "0 9902 1 0 0 -1.414214"]);
    assert_eq!(cards(&d, "NT"), ["9901 1 1 1 0 0 0 1 0 0", "9902 1 2 1 0 0 0 1 0 0"]);

    // EZNEC refuses a split source at an open end.
    let mut m = base();
    m.sources = vec![source(1, 100.0, SourceKind::SplitVoltage)];
    let e = write_deck(
        &m,
        &Options {
            engine: Engine::Nec2,
            source_name: None,
        },
    )
    .unwrap_err();
    assert!(e.0.contains("open end"), "{e}");
}

#[test]
fn loads() {
    let mut m = base();
    let load = |value| Load {
        wire: 1,
        position_pct: 30.0,
        value,
        z_at_design: (0.0, 0.0),
    };
    m.loads = vec![
        load(LoadValue::Impedance { r: 12.5, x: -34.5 }),
        load(LoadValue::Rlc {
            form: RlcForm::Series,
            r: 1.5,
            l_h: 2.25e-6,
            c_f: 33e-12,
            r_freq_mhz: 0.0,
        }),
        load(LoadValue::Rlc {
            form: RlcForm::Parallel,
            r: 5000.0,
            l_h: 3.3e-6,
            c_f: 17e-12,
            r_freq_mhz: 0.0,
        }),
    ];
    // 30% of 21 segments is segment 7.
    assert_eq!(
        cards(&deck(&m, Engine::Nec4), "LD"),
        [
            "4 1 7 7 12.5 -34.5",
            "0 1 7 7 1.5 2.25E-6 3.3E-11",
            "1 1 7 7 5000 3.3E-6 1.7E-11"
        ]
    );

    // R given at another frequency is scaled by the square root of the ratio.
    m.loads = vec![load(LoadValue::Rlc {
        form: RlcForm::Series,
        r: 4.7,
        l_h: 2.25e-6,
        c_f: 0.0,
        r_freq_mhz: 7.0,
    })];
    let d = deck(&m, Engine::Nec4);
    assert_eq!(cards(&d, "LD"), ["0 1 7 7 6.694112 2.25E-6 0"]);
    assert!(d.notes.iter().any(|n| n.contains("square root")));

    // A trap and a Laplace load have no card, so they are R + jX at the frequency.
    m.loads = vec![Load {
        wire: 1,
        position_pct: 30.0,
        value: LoadValue::Laplace {
            numerator: [1.0, 3.0, 5.0, 7.0, 9.0, 11.0],
            denominator: [2.0, 4.0, 6.0, 8.0, 10.0, 12.0],
        },
        z_at_design: (0.9166667, 1.5566798e-10),
    }];
    let d = deck(&m, Engine::Nec2);
    assert_eq!(cards(&d, "LD"), ["4 1 7 7 0.9166667 1.55668E-10"]);
    assert!(d.notes.iter().any(|n| n.contains("only at that frequency")));
    assert!(!d.notes.iter().any(|n| n.contains("but the file stores")));

    // A stored value that disagrees with the computed one is pointed out.
    m.loads[0].z_at_design = (2.0, 0.0);
    assert!(
        deck(&m, Engine::Nec2)
            .notes
            .iter()
            .any(|n| n.contains("but the file stores"))
    );

    m.loads = vec![load(LoadValue::Rlc {
        form: RlcForm::Trap,
        r: 2.0,
        l_h: 1e-6,
        c_f: 0.0,
        r_freq_mhz: 0.0,
    })];
    let ld = cards(&deck(&m, Engine::Nec5), "LD");
    assert!(ld[0].starts_with("4 1 7 7 2 89.2212"), "{}", ld[0]);
}

#[test]
fn wire_loss() {
    let mut m = base();
    m.wires.push(wire([0.0, 5.0, 10.0], [0.0, 8.0, 7.0], 9));
    m.wires[0].resistivity_ohm_m = 4.0e-8;
    m.wires[1].resistivity_ohm_m = 4.0e-8;
    // One card covers every wire when they are alike.
    assert_eq!(cards(&deck(&m, Engine::Nec2), "LD"), ["5 0 0 0 2.5E7"]);
    m.wires[1].resistivity_ohm_m = 0.0;
    assert_eq!(cards(&deck(&m, Engine::Nec2), "LD"), ["5 1 0 0 2.5E7"]);
    m.wires[1].resistivity_ohm_m = 1.74e-8;
    assert_eq!(
        cards(&deck(&m, Engine::Nec4), "LD"),
        ["5 1 0 0 2.5E7", "5 2 0 0 5.747127E7"]
    );
}

#[test]
fn insulation() {
    let mut m = base();
    m.wires[0].diameter_m = 0.0020524196;
    m.wires[0].insulation = Some(Insulation {
        rel_permittivity: 2.3,
        thickness_m: 0.0005,
        third: 0.0,
    });

    // NEC-4.2 has the IS card: tag, relative permittivity, conductivity, outer radius in metres.
    let d = deck(&m, Engine::Nec4);
    assert_eq!(cards(&d, "IS"), ["0 1 0 0 2.3 0 0.00152621"]);
    assert_eq!(cards(&d, "GW"), ["1 21 0 -5 10 0 5 10 0.00336683"]);
    assert!(cards(&d, "LD").is_empty());

    // NEC-2 and NEC-5 get the equivalent radius and a distributed inductance.
    for engine in [Engine::Nec2, Engine::Nec5] {
        let d = deck(&m, engine);
        assert!(cards(&d, "IS").is_empty());
        assert_eq!(cards(&d, "GW"), ["1 21 0 -5 10 0 5 10 0.00421359"]);
        assert_eq!(cards(&d, "LD"), ["2 1 0 0 0 4.486868E-8 0"]);
        assert!(d.notes.iter().any(|n| n.contains("cannot model insulation")));
    }
}

fn two_wires() -> EzModel {
    let mut m = base();
    m.wires.push(wire([1.0, -5.0, 10.0], [1.0, 5.0, 10.0], 21));
    m
}

fn line(end2: TlEnd2, z0: f32, length_m: Option<f32>, vf: f32) -> TransmissionLine {
    TransmissionLine {
        wire_a: 1,
        position_a_pct: 50.0,
        end2,
        z0_ohm: z0,
        length_m,
        velocity_factor: vf,
        loss_db_per_m: 0.0,
        loss_second: 0.0,
    }
}

#[test]
fn transmission_lines() {
    let mut m = two_wires();
    let to2 = || TlEnd2::Wire {
        wire: 2,
        position_pct: 50.0,
    };

    // NEC's TL length is electrical: 3.21 m at VF 0.66.
    m.transmission_lines = vec![line(to2(), 75.0, Some(3.21), 0.66)];
    assert_eq!(cards(&deck(&m, Engine::Nec4), "TL"), ["1 11 2 11 75 4.863636 0 0 0 0"]);
    // Reversed: negative Z0.
    m.transmission_lines = vec![line(to2(), -75.0, Some(3.21), 0.66)];
    assert_eq!(cards(&deck(&m, Engine::Nec2), "TL"), ["1 11 2 11 -75 4.863636 0 0 0 0"]);
    // Actual distance at VF 1 is NEC's own zero length.
    m.transmission_lines = vec![line(to2(), 75.0, None, 1.0)];
    assert_eq!(cards(&deck(&m, Engine::Nec4), "TL"), ["1 11 2 11 75 0 0 0 0 0"]);
    // Actual distance at another VF: 1 ft apart, 0.3048 m / 0.66.
    m.transmission_lines = vec![line(to2(), 75.0, None, 0.66)];
    assert_eq!(cards(&deck(&m, Engine::Nec4), "TL"), ["1 11 2 11 75 0.4618182 0 0 0 0"]);

    // A line with loss is a network, as in EZNEC's export of tl-loss:
    // NT 1,11,2,11,2.6686E-4,-1.647E-3,-1.99E-4,1.3429E-2,2.6686E-4,-1.647E-3
    let mut lossy = line(to2(), 75.0, Some(3.21), 0.66);
    lossy.loss_db_per_m = 0.049212597;
    m.transmission_lines = vec![lossy.clone()];
    let d = deck(&m, Engine::Nec2);
    assert!(cards(&d, "TL").is_empty());
    let nt = cards(&d, "NT");
    let f: Vec<f64> = nt[0].split(' ').map(|x| x.parse().unwrap()).collect();
    assert_eq!(&f[..4], [1.0, 11.0, 2.0, 11.0]);
    for (got, want) in f[4..]
        .iter()
        .zip([2.6686e-4, -1.647e-3, -1.99e-4, 1.3429e-2, 2.6686e-4, -1.647e-3])
    {
        assert!((got - want).abs() < 3e-4 * 1.3429e-2, "{got} {want}");
    }
    assert!(nt[0].len() + 3 <= 80, "fits nec2dx's 80 columns: {}", nt[0]);
    assert!(d.notes.iter().any(|n| n.contains("has loss")));
    // Reversed with loss: Y12 changes sign.
    lossy.z0_ohm = -75.0;
    m.transmission_lines = vec![lossy];
    let nt = cards(&deck(&m, Engine::Nec2), "NT");
    let f: Vec<f64> = nt[0].split(' ').map(|x| x.parse().unwrap()).collect();
    assert!(f[6] > 0.0 && f[7] < 0.0);
}

#[test]
fn shorted_stub() {
    let mut m = base();
    m.transmission_lines = vec![line(TlEnd2::ShortedStub, 450.0, Some(1.2192), 1.0)];
    let d = deck(&m, Engine::Nec4);
    // The far end is a dummy wire with a large conductance across it.
    assert_eq!(cards(&d, "TL"), ["1 11 9901 1 450 1.2192 0 0 1E10 0"]);
    assert!(cards(&d, "GW")[1].starts_with("9901 1 "));
    // NEC-5 needs two segments on the dummy to have a segment end to use.
    let d = deck(&m, Engine::Nec5);
    assert_eq!(cards(&d, "TL"), ["1 11 9901 1 450 1.2192 0 0 1E10 0"]);
    assert!(cards(&d, "GW")[1].starts_with("9901 2 "));

    // A stub needs a length.
    m.transmission_lines = vec![line(TlEnd2::ShortedStub, 450.0, None, 1.0)];
    assert!(
        write_deck(
            &m,
            &Options {
                engine: Engine::Nec4,
                source_name: None
            }
        )
        .is_err()
    );
}

#[test]
fn l_network() {
    let mut m = two_wires();
    let port = |wire, pct, r, l, c| LPort {
        wire,
        position_pct: pct,
        r_ohm: r,
        l_h: l,
        c_f: c,
        r_freq_mhz: 0.0,
        connection: Connection::Series,
        z_at_design: (0.0, 0.0),
    };
    m.l_networks = vec![LNetwork {
        ports: [port(2, 50.0, 4.7, 2.25e-6, 33e-12), port(1, 30.0, 0.0, 0.0, 1e-10)],
    }];
    let d = deck(&m, Engine::Nec4);
    // EZNEC: 2.4336E-4,7.1916E-3,-2.434E-4,-7.192E-3,2.4344E-4,1.6114E-2
    let nt = cards(&d, "NT");
    let f: Vec<f64> = nt[0].split(' ').map(|x| x.parse().unwrap()).collect();
    assert_eq!(&f[..4], [2.0, 11.0, 1.0, 7.0]);
    for (got, want) in f[4..]
        .iter()
        .zip([2.4336e-4, 7.1916e-3, -2.4336e-4, -7.1916e-3, 2.4336e-4, 1.6114e-2])
    {
        assert!((got - want).abs() < 1e-6, "{got} {want}");
    }
    assert!(d.notes.iter().any(|n| n.contains("L network 1")));
    // It must fit nec2dx's 80 columns too.
    let nt = cards(&deck(&m, Engine::Nec2), "NT");
    assert!(nt[0].len() + 3 <= 80, "{}", nt[0]);
}

#[test]
fn grounds() {
    let mut m = base();
    m.ground = Ground::Perfect;
    let d = deck(&m, Engine::Nec4);
    assert_eq!(
        (cards(&d, "GE"), cards(&d, "GN")),
        (vec!["1".to_string()], vec!["1".to_string()])
    );

    m.ground = Ground::Real {
        model: RealGroundModel::HighAccuracy,
        media: vec![Medium {
            conductivity_s_per_m: 0.005,
            rel_permittivity: 13.0,
            r_coord_m: 0.0,
            height_m: 0.0,
        }],
    };
    for engine in ENGINES {
        let d = deck(&m, engine);
        assert_eq!(cards(&d, "GN"), ["2 0 0 0 13 0.005"]);
        assert!(cards(&d, "GD").is_empty());
        assert_eq!(cards(&d, "RP"), ["0 1 361 1000 67 0 0 1"]);
    }

    // The MININEC-type ground: perfect for the currents, real for the far
    // field. NEC-2 also needs RP 3 to use the second medium.
    m.ground = mininec();
    let d = deck(&m, Engine::Nec2);
    assert_eq!(
        (cards(&d, "GN"), cards(&d, "GD")),
        (vec!["1".to_string()], vec!["0 0 0 0 13 0.005 0 0".to_string()])
    );
    assert_eq!(cards(&d, "RP"), ["3 1 361 1000 67 0 0 1"]);
    for engine in [Engine::Nec4, Engine::Nec5] {
        let d = deck(&m, engine);
        assert_eq!(
            (cards(&d, "GN"), cards(&d, "GD")),
            (vec!["1".to_string()], vec!["2 0 0 0 13 0.005 0 0".to_string()])
        );
        assert_eq!(cards(&d, "RP"), ["0 1 361 1000 67 0 0 1"]);
        assert!(d.notes.iter().any(|n| n.contains("MININEC-type ground")));
    }

    // Two media, as EZNEC exports them: GD with the second medium, its R
    // Coord and the height with its sign changed.
    if let Ground::Real { media, .. } = &mut m.ground {
        media[0] = Medium {
            conductivity_s_per_m: 0.0123,
            rel_permittivity: 17.0,
            r_coord_m: 0.0,
            height_m: 0.0,
        };
        media.push(Medium {
            conductivity_s_per_m: 0.0045,
            rel_permittivity: 17.0,
            r_coord_m: 0.0,
            height_m: -0.6096,
        });
    }
    let d = deck(&m, Engine::Nec4);
    assert_eq!(cards(&d, "GD"), ["2 0 0 0 17 0.0045 0 0.6096"]);
    assert!(d.notes.iter().any(|n| n.contains("two ground media")));
}

#[test]
fn buried_wires_use_ge_minus_one() {
    let mut m = base();
    m.ground = Ground::Real {
        model: RealGroundModel::HighAccuracy,
        media: vec![Medium {
            conductivity_s_per_m: 0.005,
            rel_permittivity: 13.0,
            r_coord_m: 0.0,
            height_m: 0.0,
        }],
    };
    m.wires = vec![wire([0.0, 0.0, -1.0], [0.0, 0.0, 30.0], 31)];
    assert_eq!(cards(&deck(&m, Engine::Nec4), "GE"), ["-1"]);
    m.wires = vec![wire([0.0, 0.0, 0.0], [0.0, 0.0, 30.0], 30)];
    assert_eq!(cards(&deck(&m, Engine::Nec4), "GE"), ["1"]);
}

#[test]
fn patterns() {
    let mut m = base();
    m.ground = mininec();
    // An elevation cut at azimuth 90: over the top from one horizon to the other.
    m.plot.kind = PlotKind::Elevation;
    m.plot.angle_deg = 90.0;
    assert_eq!(cards(&deck(&m, Engine::Nec4), "RP"), ["0 181 1 1000 90 90 -1 0"]);
    // In free space, all the way round.
    m.ground = Ground::FreeSpace;
    assert_eq!(cards(&deck(&m, Engine::Nec4), "RP"), ["0 361 1 1000 180 90 -1 0"]);
    // A 3 degree step.
    m.plot.kind = PlotKind::Azimuth;
    m.plot.angle_deg = 17.0;
    m.plot.step_deg = 3.0;
    assert_eq!(cards(&deck(&m, Engine::Nec4), "RP"), ["0 1 121 1000 73 0 0 3"]);
    // 3D: 5 degree steps, the upper half over ground.
    m.plot.kind = PlotKind::ThreeD;
    assert_eq!(cards(&deck(&m, Engine::Nec4), "RP"), ["0 37 73 1001 0 0 5 5"]);
    m.ground = mininec();
    assert_eq!(cards(&deck(&m, Engine::Nec2), "RP"), ["3 19 73 1001 0 0 5 5"]);
    // Code 2 follows the 2D cut.
    m.plot.kind = PlotKind::Code2;
    m.plot.cut = PlotCut::Azimuth;
    m.plot.step_deg = 1.0;
    assert_eq!(cards(&deck(&m, Engine::Nec4), "RP"), ["0 1 361 1000 73 0 0 1"]);
    // Ground wave, as EZNEC exports it.
    m.plot.kind = PlotKind::Azimuth;
    m.plot.angle_deg = 0.0;
    m.plot.ground_wave_distance_m = 3762.756;
    assert_eq!(cards(&deck(&m, Engine::Nec4), "RP"), ["1 1 361 1000 0 0 0 1 3762.756"]);
}

#[test]
fn nec5_attaches_at_segment_ends() {
    // An even segment count: the centre is junction 10, the far end of segment 10.
    let mut m = base();
    m.wires[0].segments = 20;
    let d = deck(&m, Engine::Nec5);
    assert_eq!(cards(&d, "EX"), ["0 1 10 0 1.414214 0"]);
    assert!(d.notes.iter().any(|n| n.contains("not confirmed")));
    assert_eq!(cards(&deck(&m, Engine::Nec4), "EX"), ["0 1 11 0 1.414214 0"]);

    // A source at an open end moves to the first junction inside.
    m.sources[0].position_pct = 0.0;
    assert_eq!(cards(&deck(&m, Engine::Nec5), "EX"), ["0 1 1 0 1.414214 0"]);

    // At the base of a grounded monopole it stays on the ground connection.
    let mut m = base();
    m.ground = Ground::Perfect;
    m.wires = vec![wire([0.0, 0.0, 0.0], [0.0, 0.0, 30.0], 30)];
    m.sources[0].position_pct = 0.0;
    assert_eq!(cards(&deck(&m, Engine::Nec5), "EX"), ["0 1 -1 0 1.414214 0"]);
    assert_eq!(cards(&deck(&m, Engine::Nec4), "EX"), ["0 1 1 0 1.414214 0"]);
}

#[test]
fn nec5_one_segment_feed_wire() {
    // The usual EZNEC feed point: a tiny one-segment wire joined to the
    // elements by transmission lines.
    let mut m = two_wires();
    m.wires.push(wire([0.0, -0.1, 100.0], [0.0, 0.1, 100.0], 1));
    m.sources = vec![source(3, 50.0, SourceKind::Voltage)];
    m.transmission_lines = vec![TransmissionLine {
        wire_a: 3,
        position_a_pct: 50.0,
        end2: TlEnd2::Wire {
            wire: 1,
            position_pct: 50.0,
        },
        z0_ohm: 50.0,
        length_m: Some(2.0),
        velocity_factor: 1.0,
        loss_db_per_m: 0.0,
        loss_second: 0.0,
    }];
    let d = deck(&m, Engine::Nec4);
    assert!(cards(&d, "GW")[2].starts_with("3 1 "));
    assert_eq!(cards(&d, "EX"), ["0 3 1 0 1.414214 0"]);
    assert_eq!(cards(&d, "TL"), ["3 1 1 11 50 2 0 0 0 0"]);

    // NEC-5 can't use a one-segment wire, so it gets two and is fed at the middle.
    let d = deck(&m, Engine::Nec5);
    assert!(cards(&d, "GW")[2].starts_with("3 2 "));
    assert_eq!(cards(&d, "EX"), ["0 3 1 0 1.414214 0"]);
    assert_eq!(cards(&d, "TL"), ["3 1 1 11 50 2 0 0 0 0"]);
    assert!(d.notes.iter().any(|n| n.contains("two segments")));
    // Wires that carry nothing keep their segment counts.
    assert!(cards(&d, "GW")[1].starts_with("2 21 "));
}

#[test]
fn notes_are_in_the_deck_and_lines_fit_the_engine() {
    let mut m = base();
    m.ground = mininec();
    m.sweep = Some(Sweep {
        start_mhz: 14.0,
        stop_mhz: 14.3,
        step_mhz: 0.05,
    });
    m.wires[0].insulation = Some(Insulation {
        rel_permittivity: 2.3,
        thickness_m: 0.0005,
        third: 0.0,
    });
    for (engine, limit) in [(Engine::Nec2, 80), (Engine::Nec4, 120), (Engine::Nec5, 220)] {
        let d = deck(&m, engine);
        assert!(d.notes.iter().any(|n| n.contains("sweep")));
        assert!(
            d.text
                .lines()
                .any(|l| l.starts_with("CM ez2nec: ") && l.contains("sweep"))
        );
        for l in d.text.lines() {
            assert!(l.len() <= limit, "{} columns: {l}", l.len());
        }
    }
}

#[test]
fn a_model_with_no_wires_is_refused() {
    let mut m = base();
    m.wires.clear();
    m.sources.clear();
    assert!(
        write_deck(
            &m,
            &Options {
                engine: Engine::Nec4,
                source_name: None
            }
        )
        .is_err()
    );
}
