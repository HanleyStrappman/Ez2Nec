//! EZNEC's rules for placing things on segments, as established from its own
//! exports and computed results.

/// The segment (1-based) that holds a point `pct` percent along a wire of
/// `segments` segments. Sources, loads, transmission line ends and L network
/// ports all use it: floor(pct·N/100) + 1, capped at N.
pub fn segment_at(pct: f32, segments: u16) -> u16 {
    let x = pct as f64 * segments as f64 / 100.0;
    let s = x.floor().max(0.0) as u64 + 1;
    s.min(segments as u64) as u16
}

/// The segment junction (0 to N, counted along the wire) a split source uses:
/// the junction nearest the point, with a half rounded up. EZNEC feeds the
/// segments on both sides of it.
pub fn split_junction(pct: f32, segments: u16) -> u16 {
    let x = pct as f64 * segments as f64 / 100.0;
    let j = (x + 0.5).floor().max(0.0) as u64;
    j.min(segments as u64) as u16
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn floor_rule_matches_eznec_exports() {
        assert_eq!(segment_at(50.0, 21), 11);
        assert_eq!(segment_at(0.0, 21), 1);
        assert_eq!(segment_at(100.0, 21), 21);
        assert_eq!(segment_at(50.0, 19), 10); // dl6wu12
        assert_eq!(segment_at(50.0, 15), 8); // 10nb
        assert_eq!(segment_at(50.0, 3), 2); // 20wb4l-taper
        assert_eq!(segment_at(30.0, 9), 3); // lnet-two network 2 port 1
        assert_eq!(segment_at(70.0, 21), 15); // lnet-two network 2 port 2
    }

    #[test]
    fn split_junction_rounds_half_up() {
        assert_eq!(split_junction(30.0, 21), 6); // split-30pct: segs 6 and 7
        assert_eq!(split_junction(50.0, 20), 10); // split-20seg: segs 10 and 11
        assert_eq!(split_junction(50.0, 21), 11); // src-split: segs 11 and 12
        assert_eq!(split_junction(93.75, 8), 8); // mod21-6: the wire's end
        assert_eq!(split_junction(100.0, 11), 11); // MOD16-3
        assert_eq!(split_junction(0.0, 21), 0); // split-start
    }
}
