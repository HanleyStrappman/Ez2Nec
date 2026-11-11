//! A small complex type for the network and load arithmetic.

use std::ops::{Add, Div, Mul, Neg, Sub};

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct C {
    pub re: f64,
    pub im: f64,
}

pub const ZERO: C = C { re: 0.0, im: 0.0 };

impl C {
    pub const fn new(re: f64, im: f64) -> C {
        C { re, im }
    }

    pub fn polar(magnitude: f64, degrees: f64) -> C {
        let r = degrees.to_radians();
        C::new(magnitude * r.cos(), magnitude * r.sin())
    }

    pub fn abs(self) -> f64 {
        self.re.hypot(self.im)
    }

    pub fn is_zero(self) -> bool {
        self.re == 0.0 && self.im == 0.0
    }

    pub fn recip(self) -> C {
        let d = self.re * self.re + self.im * self.im;
        C::new(self.re / d, -self.im / d)
    }

    pub fn sqrt(self) -> C {
        let m = self.abs();
        let re = ((m + self.re) / 2.0).max(0.0).sqrt();
        let im = ((m - self.re) / 2.0).max(0.0).sqrt();
        C::new(re, if self.im < 0.0 { -im } else { im })
    }

    pub fn exp(self) -> C {
        let e = self.re.exp();
        C::new(e * self.im.cos(), e * self.im.sin())
    }

    pub fn sinh(self) -> C {
        (self.exp() - (-self).exp()) * 0.5
    }

    pub fn cosh(self) -> C {
        (self.exp() + (-self).exp()) * 0.5
    }
}

impl Add for C {
    type Output = C;
    fn add(self, o: C) -> C {
        C::new(self.re + o.re, self.im + o.im)
    }
}

impl Sub for C {
    type Output = C;
    fn sub(self, o: C) -> C {
        C::new(self.re - o.re, self.im - o.im)
    }
}

impl Neg for C {
    type Output = C;
    fn neg(self) -> C {
        C::new(-self.re, -self.im)
    }
}

impl Mul for C {
    type Output = C;
    fn mul(self, o: C) -> C {
        C::new(self.re * o.re - self.im * o.im, self.re * o.im + self.im * o.re)
    }
}

impl Mul<f64> for C {
    type Output = C;
    fn mul(self, k: f64) -> C {
        C::new(self.re * k, self.im * k)
    }
}

impl Div for C {
    type Output = C;
    #[allow(clippy::suspicious_arithmetic_impl)]
    fn div(self, o: C) -> C {
        self * o.recip()
    }
}
