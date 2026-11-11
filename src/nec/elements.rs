//! Impedances and admittances of EZNEC's lumped elements at one frequency,
//! computed the way EZNEC does. Each rule here was checked against a value
//! EZNEC stored in a file or wrote to an export.

use std::f64::consts::TAU;

use super::complex::{C, ZERO};
use crate::ez::{Connection, LNetwork, LPort, RlcForm, SPEED_OF_LIGHT_M_PER_US};

/// The admittance EZNEC writes for a short circuit in a network: 0.001 + j0.001 ohm.
pub const SHORT_Y: C = C::new(500.0, -500.0);

fn omega(f_mhz: f64) -> f64 {
    TAU * f_mhz * 1e6
}

/// R at frequency `f_mhz` when it is given at `r_freq_mhz`: it scales as √f.
/// An R Freq of 0 means R is fixed.
pub fn r_at(r: f64, r_freq_mhz: f64, f_mhz: f64) -> f64 {
    if r_freq_mhz > 0.0 {
        r * (f_mhz / r_freq_mhz).sqrt()
    } else {
        r
    }
}

/// The impedance of an RLC load or branch. `None` is an open circuit.
///
/// A zero element is a short in the series form and is absent in the others.
pub fn rlc(form: RlcForm, r: f64, l: f64, c: f64, r_freq_mhz: f64, f_mhz: f64) -> Option<C> {
    let w = omega(f_mhz);
    let r = r_at(r, r_freq_mhz, f_mhz);
    let zl = C::new(0.0, w * l);
    let yc = C::new(0.0, w * c);
    match form {
        RlcForm::Series => {
            let zc = if c != 0.0 { yc.recip() } else { ZERO };
            Some(C::new(r, 0.0) + zl + zc)
        }
        RlcForm::Parallel => {
            let mut y = ZERO;
            if r != 0.0 {
                y = y + C::new(1.0 / r, 0.0);
            }
            if l != 0.0 {
                y = y + zl.recip();
            }
            if c != 0.0 {
                y = y + yc;
            }
            (!y.is_zero()).then(|| y.recip())
        }
        RlcForm::Trap => {
            let zrl = C::new(r, 0.0) + zl;
            if c == 0.0 {
                Some(zrl)
            } else if zrl.is_zero() {
                Some(ZERO)
            } else {
                Some((zrl.recip() + yc).recip())
            }
        }
    }
}

/// A Laplace load: the ratio of two polynomials in s = jω (rad/s), s^0 first.
pub fn laplace(numerator: &[f32; 6], denominator: &[f32; 6], f_mhz: f64) -> C {
    let s = C::new(0.0, omega(f_mhz));
    let poly = |k: &[f32; 6]| k.iter().rev().fold(ZERO, |acc, &a| acc * s + C::new(a as f64, 0.0));
    poly(numerator) / poly(denominator)
}

/// Y11 (= Y22) and Y12 of a transmission line with loss.
///
/// EZNEC treats the loss as conductor loss only: series R = 2·α·Z0 per metre
/// and no shunt G, with the exact line equations. This matches its export of
/// a 75 ohm, 1.5 dB/100 ft line to every printed digit.
pub fn lossy_line(z0: f64, length_m: f64, vf: f64, loss_db_per_m: f64, f_mhz: f64) -> (C, C) {
    let (gamma, zc) = line_constants(z0, vf, loss_db_per_m, f_mhz);
    let gl = gamma * length_m;
    let y11 = gl.cosh() / (zc * gl.sinh());
    let y12 = -(zc * gl.sinh()).recip();
    (y11, y12)
}

/// The input admittance of a line with loss whose far end is shorted.
pub fn lossy_shorted_stub(z0: f64, length_m: f64, vf: f64, loss_db_per_m: f64, f_mhz: f64) -> C {
    lossy_line(z0, length_m, vf, loss_db_per_m, f_mhz).0
}

/// The input admittance of a line with loss whose far end is open.
pub fn lossy_open_stub(z0: f64, length_m: f64, vf: f64, loss_db_per_m: f64, f_mhz: f64) -> C {
    let (y11, y12) = lossy_line(z0, length_m, vf, loss_db_per_m, f_mhz);
    y11 - y12 * y12 / y11
}

fn line_constants(z0: f64, vf: f64, loss_db_per_m: f64, f_mhz: f64) -> (C, C) {
    let w = omega(f_mhz);
    let v = SPEED_OF_LIGHT_M_PER_US * 1e6 * vf;
    let alpha = loss_db_per_m / (20.0 / std::f64::consts::LN_10);
    let series = C::new(2.0 * alpha * z0, w * z0 / v);
    let shunt = C::new(0.0, w / (z0 * v));
    ((series * shunt).sqrt(), (series / shunt).sqrt())
}

fn port_admittance(p: &LPort, f_mhz: f64) -> C {
    let form = match p.connection {
        Connection::Series => RlcForm::Series,
        Connection::Parallel => RlcForm::Parallel,
    };
    match rlc(
        form,
        p.r_ohm as f64,
        p.l_h as f64,
        p.c_f as f64,
        p.r_freq_mhz as f64,
        f_mhz,
    ) {
        None => ZERO,
        Some(z) if z.is_zero() => SHORT_Y,
        Some(z) => z.recip(),
    }
}

/// Y11, Y12 and Y22 of an L network: port 1's branch is in series between the
/// ports and port 2's branch is across port 2.
pub fn l_network(n: &LNetwork, f_mhz: f64) -> (C, C, C) {
    let y1 = port_admittance(&n.ports[0], f_mhz);
    let y2 = port_admittance(&n.ports[1], f_mhz);
    (y1, -y1, y1 + y2)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn close(a: C, re: f64, im: f64, rel: f64) {
        let scale = re.hypot(im);
        assert!((a - C::new(re, im)).abs() <= rel * scale, "{a:?} against {re} {im}");
    }

    #[test]
    fn rlc_forms_match_eznec_stored_values() {
        // load-rlc-ser: 1.5 ohm, 2.25 uH, 33 pF at 14.2 MHz.
        close(
            rlc(RlcForm::Series, 1.5, 2.25e-6, 33e-12, 0.0, 14.2).unwrap(),
            1.5,
            -138.89146,
            1e-6,
        );
        // Course model mod19-4b, parallel, at 14.1 MHz.
        let z = rlc(
            RlcForm::Parallel,
            56296.6015625,
            3.299999889350147e-6,
            1.7404999316794623e-11,
            0.0,
            14.100000381469727,
        );
        close(z.unwrap(), 5.033233165740967, 532.2861938476562, 1e-6);
        // Course model mod19-5, a trap whose R is given at 21.0 MHz, at 21.1 MHz.
        let z = rlc(
            RlcForm::Trap,
            1.8528499603271484,
            3.299999889350147e-6,
            1.7404999316794623e-11,
            21.0,
            21.100000381469727,
        );
        close(z.unwrap(), 17055.919921875, -38299.765625, 3e-5);
    }

    #[test]
    fn zero_elements() {
        assert_eq!(rlc(RlcForm::Series, 0.0, 0.0, 0.0, 0.0, 10.0), Some(ZERO));
        assert_eq!(rlc(RlcForm::Parallel, 0.0, 0.0, 0.0, 0.0, 10.0), None);
        close(
            rlc(RlcForm::Parallel, 50.0, 0.0, 0.0, 0.0, 10.0).unwrap(),
            50.0,
            0.0,
            1e-12,
        );
        close(
            rlc(RlcForm::Trap, 2.0, 1e-6, 0.0, 0.0, 10.0).unwrap(),
            2.0,
            62.83185307,
            1e-9,
        );
    }

    #[test]
    fn r_scales_as_root_frequency() {
        // lnet-2: 4.7 ohm at 7 MHz is 6.694 at 14.2 MHz.
        assert!((r_at(4.7, 7.0, 14.2) - 6.694).abs() < 1e-3);
        assert_eq!(r_at(4.7, 0.0, 14.2), 4.7);
    }

    #[test]
    fn laplace_matches_eznec() {
        // load-laplace: EZNEC stored 0.9166667 + j1.5566798e-10 at 14.2 MHz.
        let z = laplace(
            &[1.0, 3.0, 5.0, 7.0, 9.0, 11.0],
            &[2.0, 4.0, 6.0, 8.0, 10.0, 12.0],
            14.2,
        );
        assert!((z.re - 0.9166667).abs() < 1e-7);
        assert!((z.im - 1.5566798e-10).abs() < 1e-15);
    }

    #[test]
    fn open_stub_is_the_dual_of_a_shorted_one() {
        // Without loss: -j cot(bl)/Z0 shorted, +j tan(bl)/Z0 open.
        let bl = omega(14.2) * 1.524 / (SPEED_OF_LIGHT_M_PER_US * 1e6 * 0.66);
        close(
            lossy_shorted_stub(75.0, 1.524, 0.66, 0.0, 14.2),
            0.0,
            -1.0 / (75.0 * bl.tan()),
            1e-9,
        );
        close(
            lossy_open_stub(75.0, 1.524, 0.66, 0.0, 14.2),
            0.0,
            bl.tan() / 75.0,
            1e-9,
        );
    }

    #[test]
    fn lossy_line_matches_eznec_export() {
        // tl-loss.nec: NT 1,11,2,11,2.6686E-4,-1.647E-3,-1.99E-4,1.3429E-2,...
        let (y11, y12) = lossy_line(75.0, 3.21, 0.66, 0.049212597, 14.2);
        // The export prints 4 or 5 digits.
        close(y11, 2.6686e-4, -1.647e-3, 3e-4);
        close(y12, -1.99e-4, 1.3429e-2, 3e-4);
    }

    #[test]
    fn l_network_matches_eznec_export() {
        let port = |r, l, c, connection| LPort {
            wire: 1,
            position_pct: 50.0,
            r_ohm: r,
            l_h: l,
            c_f: c,
            r_freq_mhz: 0.0,
            connection,
            z_at_design: (0.0, 0.0),
        };
        // lnet-two.nec network 1: 2.4336E-4,7.1916E-3,-2.434E-4,-7.192E-3,2.4344E-4,1.6114E-2
        let n = LNetwork {
            ports: [
                port(4.7, 2.25e-6, 33e-12, Connection::Series),
                port(0.0, 0.0, 1e-10, Connection::Series),
            ],
        };
        let (y11, y12, y22) = l_network(&n, 14.2);
        close(y11, 2.4336e-4, 7.1916e-3, 1e-4);
        close(y12, -2.4336e-4, -7.1916e-3, 1e-4);
        close(y22, 2.4344e-4, 1.6114e-2, 1e-4);
        // Network 2: an all-short port 2 gives 500.0002, -499.9904.
        let n = LNetwork {
            ports: [
                port(2.2, 1.5e-6, 47e-12, Connection::Series),
                port(0.0, 0.0, 0.0, Connection::Series),
            ],
        };
        let (y11, _, y22) = l_network(&n, 14.2);
        close(y11, 2.0084e-4, 9.5525e-3, 1e-4);
        close(y22, 500.0002, -499.9904, 1e-6);
    }
}
