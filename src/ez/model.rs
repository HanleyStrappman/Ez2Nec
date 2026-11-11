//! The decoded content of an `.ez` file, as EZNEC stores it.
//!
//! Lengths are in metres whatever the display unit, as the file stores them.
//! Positions along a wire are percentages from end 1. Source magnitudes are RMS,
//! as EZNEC shows them. No NEC-specific choices are made here.

use super::bytes::{Unclaimed, UnknownField};

/// A decoded file and what the decoder couldn't account for.
#[derive(Debug, Clone, PartialEq)]
pub struct EzFile {
    pub model: EzModel,
    pub report: DecodeReport,
}

#[derive(Debug, Clone, PartialEq, Default)]
pub struct DecodeReport {
    /// Fields whose place is known but whose meaning is not decoded.
    pub unknown_fields: Vec<UnknownField>,
    /// Non-zero bytes that no decoder step accounted for.
    pub unclaimed: Vec<Unclaimed>,
    /// Inconsistencies between copies of the same value, and similar remarks.
    pub notes: Vec<String>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct EzModel {
    pub title: String,
    pub frequency_mhz: f32,
    pub units: LengthUnit,
    pub wires: Vec<Wire>,
    pub sources: Vec<Source>,
    pub loads: Vec<Load>,
    pub transmission_lines: Vec<TransmissionLine>,
    pub l_networks: Vec<LNetwork>,
    pub ground: Ground,
    pub plot: Plot,
    /// Frequency sweep settings, when any are set (chunk `0x0b`).
    pub sweep: Option<Sweep>,
    /// Ref Level, dBi (header 0x5E).
    pub ref_level_dbi: f32,
    /// Alt SWR Z0, ohms (header 0x6E).
    pub alt_swr_z0: f32,
    /// Which program saved the file, when it says.
    pub saved_by: SavedBy,
    /// The calculating engine selected when the file was saved (chunk `0x66`).
    pub engine: Option<CalcEngine>,
}

/// The display unit (header 0x41). The file stores every length in metres.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LengthUnit {
    Feet,
    Inches,
    Metres,
    Millimetres,
    Wavelengths,
}

impl LengthUnit {
    /// Metres per unit; wavelengths depend on the frequency.
    pub fn metres_per_unit(self, frequency_mhz: f64) -> f64 {
        match self {
            LengthUnit::Feet => 0.3048,
            LengthUnit::Inches => 0.0254,
            LengthUnit::Metres => 1.0,
            LengthUnit::Millimetres => 0.001,
            LengthUnit::Wavelengths => SPEED_OF_LIGHT_M_PER_US / frequency_mhz,
        }
    }

    pub fn symbol(self) -> &'static str {
        match self {
            LengthUnit::Feet => "ft",
            LengthUnit::Inches => "in",
            LengthUnit::Metres => "m",
            LengthUnit::Millimetres => "mm",
            LengthUnit::Wavelengths => "wl",
        }
    }
}

/// Metres per microsecond, so that c / f(MHz) is a wavelength in metres.
pub const SPEED_OF_LIGHT_M_PER_US: f64 = 299.792458;

#[derive(Debug, Clone, PartialEq)]
pub struct Wire {
    pub end1: [f32; 3],
    pub end2: [f32; 3],
    pub diameter_m: f32,
    /// AWG gauge when the wire was given as a gauge; `None` when the diameter was typed.
    pub awg: Option<i16>,
    pub segments: u16,
    /// Resistivity, Ω·m; 0 is lossless. From chunk `0x0d` when present, else header 0x62.
    pub resistivity_ohm_m: f32,
    /// Stored with each resistivity. It is 1.0 in every file seen, perhaps relative permeability.
    pub loss_factor: f32,
    pub insulation: Option<Insulation>,
}

/// Wire insulation (chunk `0x0c`).
#[derive(Debug, Clone, PartialEq)]
pub struct Insulation {
    pub rel_permittivity: f32,
    pub thickness_m: f32,
    /// Third value in each row; 0 in every file seen.
    pub third: f32,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Source {
    /// 1-based wire number.
    pub wire: u16,
    pub position_pct: f32,
    /// RMS volts or amps.
    pub magnitude: f32,
    pub phase_deg: f32,
    pub kind: SourceKind,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SourceKind {
    Voltage,
    Current,
    SplitVoltage,
    SplitCurrent,
}

impl SourceKind {
    pub fn is_split(self) -> bool {
        matches!(self, SourceKind::SplitVoltage | SourceKind::SplitCurrent)
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct Load {
    pub wire: u16,
    pub position_pct: f32,
    pub value: LoadValue,
    /// R and X as EZNEC computed them at the design frequency. For an R + jX
    /// load, these are the entered values.
    pub z_at_design: (f32, f32),
}

#[derive(Debug, Clone, PartialEq)]
pub enum LoadValue {
    Impedance {
        r: f32,
        x: f32,
    },
    /// Ohms, henries, farads. R applies at `r_freq_mhz` and scales as √f from
    /// it; 0 means a fixed R.
    Rlc {
        form: RlcForm,
        r: f32,
        l_h: f32,
        c_f: f32,
        r_freq_mhz: f32,
    },
    /// Coefficients of s^0 to s^5.
    Laplace {
        numerator: [f32; 6],
        denominator: [f32; 6],
    },
}

/// How an RLC load's three elements are connected (record +0x86).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RlcForm {
    /// `S`: R, L and C in series.
    Series,
    /// `P`: R, L and C in parallel. The stored impedances in course models
    /// mod19-4a and mod19-4b match this to 7 digits.
    Parallel,
    /// `T`: R in series with L, and C across both. Course model mod19-5's
    /// stored impedance matches this to 5 digits once R is scaled from its R Freq.
    Trap,
}

/// Where a transmission line's second end goes.
#[derive(Debug, Clone, PartialEq)]
pub enum TlEnd2 {
    Wire {
        wire: u16,
        position_pct: f32,
    },
    /// Wire number −1: a stub whose far end is a short circuit.
    ShortedStub,
    /// Wire number −2: a stub whose far end is open.
    OpenStub,
}

#[derive(Debug, Clone, PartialEq)]
pub struct TransmissionLine {
    pub wire_a: u16,
    pub position_a_pct: f32,
    pub end2: TlEnd2,
    /// Characteristic impedance, ohms, as stored: a negative value means reversed.
    pub z0_ohm: f32,
    /// Electrical length in metres; `None` means the actual distance between the ends.
    pub length_m: Option<f32>,
    pub velocity_factor: f32,
    /// Loss, dB per metre (chunk `0x0e`); 0 when lossless or absent.
    pub loss_db_per_m: f32,
    /// Second value stored with the loss; 0 in every file seen.
    pub loss_second: f32,
}

impl TransmissionLine {
    pub fn reversed(&self) -> bool {
        self.z0_ohm.is_sign_negative()
    }
}

/// One L network (chunk `0x11`), a two-port with a branch at each port.
#[derive(Debug, Clone, PartialEq)]
pub struct LNetwork {
    pub ports: [LPort; 2],
}

#[derive(Debug, Clone, PartialEq)]
pub struct LPort {
    pub wire: u16,
    pub position_pct: f32,
    /// Ohms, henries, farads; 0 is a short for each element.
    pub r_ohm: f32,
    pub l_h: f32,
    pub c_f: f32,
    /// Frequency at which R applies, MHz; R scales as √f from it. 0 means fixed R.
    pub r_freq_mhz: f32,
    pub connection: Connection,
    /// R and X as EZNEC computed them at the design frequency.
    pub z_at_design: (f32, f32),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Connection {
    Series,
    Parallel,
}

#[derive(Debug, Clone, PartialEq)]
pub enum Ground {
    FreeSpace,
    Perfect,
    Real { model: RealGroundModel, media: Vec<Medium> },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RealGroundModel {
    Mininec,
    HighAccuracy,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Medium {
    pub conductivity_s_per_m: f32,
    pub rel_permittivity: f32,
    /// Radial distance where this medium begins, metres.
    pub r_coord_m: f32,
    pub height_m: f32,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Plot {
    pub kind: PlotKind,
    /// The 2D cut (header 0x09): with a 3D plot this is `Elevation`; with code `2` it is `Azimuth`.
    pub cut: PlotCut,
    /// Elevation angle of an azimuth plot, or azimuth of an elevation plot, degrees.
    pub angle_deg: f32,
    pub step_deg: f32,
    pub range_deg: f32,
    /// Ground-wave distance, metres; 0 when not used.
    pub ground_wave_distance_m: f32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PlotKind {
    Azimuth,
    Elevation,
    ThreeD,
    /// Code `2` (header 0x7A). EZNEC v7 exports it as the 2D cut in header 0x09.
    Code2,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PlotCut {
    Azimuth,
    Elevation,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Sweep {
    pub start_mhz: f32,
    pub stop_mhz: f32,
    pub step_mhz: f32,
}

#[derive(Debug, Clone, PartialEq, Default)]
pub struct SavedBy {
    /// Header 0x90, e.g. 70000004 for 7.0.4; 0 in most files before v6.
    pub version_code: u32,
    /// Chunk `0x65`: program name, version text and OLE save date.
    pub program: Option<String>,
    pub version: Option<String>,
    pub ole_date: Option<f64>,
    /// Chunk `0x65`'s free text, such as "Calc Engine = …".
    pub text: Option<String>,
}

impl SavedBy {
    /// The version as text, from the header code when the chunk doesn't give one.
    pub fn version_text(&self) -> Option<String> {
        if let Some(v) = &self.version {
            return Some(v.clone());
        }
        let c = self.version_code;
        (c != 0).then(|| format!("{}.{}.{}", c / 10_000_000, (c / 100_000) % 100, c % 100_000))
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct CalcEngine {
    pub kind: EngineKind,
    pub name: String,
    pub path: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EngineKind {
    EzCalcD,
    ExtNec4,
    ExtNec5,
    Other(u32),
}

impl EngineKind {
    pub fn from_code(code: u32) -> Self {
        match code {
            2 => EngineKind::EzCalcD,
            6 => EngineKind::ExtNec4,
            8 => EngineKind::ExtNec5,
            other => EngineKind::Other(other),
        }
    }
}
