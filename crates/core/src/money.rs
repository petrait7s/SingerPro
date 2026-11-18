//! Exact money: signed integer paisa (1 BDT = 100 paisa). No floating point anywhere.

use crate::{CoreError, CoreResult};
use serde::{Deserialize, Serialize};
use std::fmt;

#[derive(
    Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Default, Serialize, Deserialize,
)]
#[serde(transparent)]
pub struct Money(i64);

impl Money {
    pub const ZERO: Money = Money(0);

    pub const fn from_paisa(p: i64) -> Self {
        Self(p)
    }

    pub fn from_taka(taka: i64) -> CoreResult<Self> {
        taka.checked_mul(100).map(Self).ok_or(CoreError::Overflow)
    }

    pub const fn paisa(self) -> i64 {
        self.0
    }

    pub const fn is_zero(self) -> bool {
        self.0 == 0
    }

    pub const fn is_negative(self) -> bool {
        self.0 < 0
    }

    pub fn checked_add(self, o: Money) -> CoreResult<Money> {
        self.0
            .checked_add(o.0)
            .map(Money)
            .ok_or(CoreError::Overflow)
    }

    pub fn checked_sub(self, o: Money) -> CoreResult<Money> {
        self.0
            .checked_sub(o.0)
            .map(Money)
            .ok_or(CoreError::Overflow)
    }

    pub fn checked_neg(self) -> CoreResult<Money> {
        self.0.checked_neg().map(Money).ok_or(CoreError::Overflow)
    }

    /// Multiply by an integer quantity.
    pub fn checked_mul_qty(self, qty: i64) -> CoreResult<Money> {
        self.0
            .checked_mul(qty)
            .map(Money)
            .ok_or(CoreError::Overflow)
    }

    /// Apply a percentage given in basis points (1% = 100 bp), rounding half away from zero.
    pub fn percent_bp(self, bp: i64) -> CoreResult<Money> {
        let num = (self.0 as i128) * (bp as i128);
        Ok(Money(div_round_half_away(num, 10_000)?))
    }

    /// Split into `n` parts as evenly as possible; the whole remainder goes to the LAST part
    /// (installment rule). Parts always sum exactly to `self`.
    pub fn split_even_remainder_last(self, n: u32) -> CoreResult<Vec<Money>> {
        if n == 0 {
            return Err(CoreError::Validation("cannot split into 0 parts".into()));
        }
        let n = n as i64;
        let base = self.0.div_euclid(n);
        let rem = self.0.rem_euclid(n);
        let mut v = vec![Money(base); n as usize];
        v[(n - 1) as usize] = Money(base + rem);
        Ok(v)
    }

    /// Allocate proportionally to non-negative integer weights (largest-remainder method,
    /// ties to earlier index). Parts always sum exactly to `self`. Used for landed-cost allocation.
    pub fn allocate_by_weights(self, weights: &[i64]) -> CoreResult<Vec<Money>> {
        if weights.is_empty() || weights.iter().any(|w| *w < 0) {
            return Err(CoreError::Validation(
                "weights must be non-empty and non-negative".into(),
            ));
        }
        let total: i128 = weights.iter().map(|w| *w as i128).sum();
        if total == 0 {
            return Err(CoreError::Validation("weights sum to zero".into()));
        }
        let sign: i128 = if self.0 < 0 { -1 } else { 1 };
        let amt = (self.0 as i128).abs();
        let mut parts: Vec<(i128, i128)> = weights
            .iter()
            .map(|w| {
                let x = amt * (*w as i128);
                (x / total, x % total)
            })
            .collect();
        let assigned: i128 = parts.iter().map(|p| p.0).sum();
        let mut left = amt - assigned;
        let mut order: Vec<usize> = (0..parts.len()).collect();
        order.sort_by(|a, b| parts[*b].1.cmp(&parts[*a].1).then(a.cmp(b)));
        for i in order {
            if left == 0 {
                break;
            }
            parts[i].0 += 1;
            left -= 1;
        }
        parts
            .into_iter()
            .map(|(q, _)| {
                i64::try_from(q * sign)
                    .map(Money)
                    .map_err(|_| CoreError::Overflow)
            })
            .collect()
    }

    /// Parse user input such as `1250`, `1,250.5`, `৳ 1,25,000.00`. Max 2 decimals.
    pub fn parse(input: &str) -> CoreResult<Money> {
        let s: String = input
            .trim()
            .trim_start_matches('৳')
            .chars()
            .filter(|c| !c.is_whitespace() && *c != ',')
            .collect();
        let (neg, s) = match s.strip_prefix('-') {
            Some(r) => (true, r.to_string()),
            None => (false, s),
        };
        let bad = || CoreError::Validation(format!("invalid amount: {input}"));
        let (whole, frac) = match s.split_once('.') {
            Some((w, f)) => (w, f),
            None => (s.as_str(), ""),
        };
        if whole.is_empty() && frac.is_empty() {
            return Err(bad());
        }
        if frac.len() > 2
            || !whole.chars().all(|c| c.is_ascii_digit())
            || !frac.chars().all(|c| c.is_ascii_digit())
        {
            return Err(bad());
        }
        let w: i64 = if whole.is_empty() {
            0
        } else {
            whole.parse().map_err(|_| bad())?
        };
        let f: i64 = match frac.len() {
            0 => 0,
            1 => frac.parse::<i64>().map_err(|_| bad())? * 10,
            _ => frac.parse().map_err(|_| bad())?,
        };
        let p = w
            .checked_mul(100)
            .and_then(|x| x.checked_add(f))
            .ok_or(CoreError::Overflow)?;
        Ok(Money(if neg { -p } else { p }))
    }
}

fn div_round_half_away(num: i128, den: i128) -> CoreResult<i64> {
    let sign = if num < 0 { -1 } else { 1 };
    let q = (num.abs() * 2 + den) / (den * 2);
    i64::try_from(q * sign).map_err(|_| CoreError::Overflow)
}

/// Display as `৳1,25,000.50` using the South-Asian (lakh/crore) grouping used in Bangladesh.
impl fmt::Display for Money {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let abs = self.0.unsigned_abs();
        let (whole, frac) = (abs / 100, abs % 100);
        let digits = whole.to_string();
        let grouped = if digits.len() <= 3 {
            digits
        } else {
            let (head, tail) = digits.split_at(digits.len() - 3);
            let mut parts: Vec<&str> = Vec::new();
            let mut h = head;
            while h.len() > 2 {
                let (a, b) = h.split_at(h.len() - 2);
                parts.push(b);
                h = a;
            }
            if !h.is_empty() {
                parts.push(h);
            }
            parts.reverse();
            format!("{},{}", parts.join(","), tail)
        };
        write!(
            f,
            "{}৳{}.{:02}",
            if self.0 < 0 { "-" } else { "" },
            grouped,
            frac
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn no_float_drift() {
        // 0.1 + 0.2 style trap, in paisa.
        let a = Money::parse("0.10").unwrap();
        let b = Money::parse("0.20").unwrap();
        assert_eq!(a.checked_add(b).unwrap(), Money::parse("0.30").unwrap());
    }

    #[test]
    fn parse_variants() {
        assert_eq!(Money::parse("1250").unwrap().paisa(), 125_000);
        assert_eq!(Money::parse("৳ 1,25,000.5").unwrap().paisa(), 12_500_050);
        assert_eq!(Money::parse("-3.07").unwrap().paisa(), -307);
        assert_eq!(Money::parse(".5").unwrap().paisa(), 50);
        for bad in ["", "abc", "1.234", "1..2", "--1", "1e3", "."] {
            assert!(Money::parse(bad).is_err(), "{bad} should fail");
        }
    }

    #[test]
    fn display_lakh_grouping() {
        assert_eq!(Money::from_paisa(0).to_string(), "৳0.00");
        assert_eq!(Money::from_paisa(99_900).to_string(), "৳999.00");
        assert_eq!(Money::from_paisa(100_000).to_string(), "৳1,000.00");
        assert_eq!(Money::from_paisa(12_500_050).to_string(), "৳1,25,000.50");
        assert_eq!(
            Money::from_paisa(1_234_567_800).to_string(),
            "৳1,23,45,678.00"
        );
        assert_eq!(Money::from_paisa(-5).to_string(), "-৳0.05");
    }

    #[test]
    fn overflow_is_an_error() {
        assert!(Money::from_paisa(i64::MAX)
            .checked_add(Money::from_paisa(1))
            .is_err());
        assert!(Money::from_taka(i64::MAX).is_err());
        assert!(Money::from_paisa(i64::MIN).checked_neg().is_err());
    }

    #[test]
    fn percent_rounds_half_away() {
        // 5% of ৳0.30 = 1.5 paisa -> 2
        assert_eq!(Money::from_paisa(30).percent_bp(500).unwrap().paisa(), 2);
        assert_eq!(Money::from_paisa(-30).percent_bp(500).unwrap().paisa(), -2);
        assert_eq!(
            Money::from_paisa(10_000).percent_bp(750).unwrap().paisa(),
            750
        );
        assert_eq!(Money::from_paisa(10_000).percent_bp(0).unwrap().paisa(), 0);
    }

    #[test]
    fn installment_split_remainder_goes_last() {
        let parts = Money::from_paisa(100_000)
            .split_even_remainder_last(3)
            .unwrap();
        assert_eq!(
            parts.iter().map(|m| m.paisa()).collect::<Vec<_>>(),
            vec![33_333, 33_333, 33_334]
        );
        let sum: i64 = parts.iter().map(|m| m.paisa()).sum();
        assert_eq!(sum, 100_000);
        assert!(Money::ZERO.split_even_remainder_last(0).is_err());
    }

    #[test]
    fn split_always_reconciles() {
        for total in [0i64, 1, 7, 99, 100_001, 123_456_789] {
            for n in 1..=37u32 {
                let p = Money::from_paisa(total)
                    .split_even_remainder_last(n)
                    .unwrap();
                assert_eq!(p.len(), n as usize);
                assert_eq!(p.iter().map(|m| m.paisa()).sum::<i64>(), total);
            }
        }
    }

    #[test]
    fn weighted_allocation_reconciles_and_is_proportional() {
        // landed cost 1000.00 over lines worth 3:1 -> 750 / 250
        let p = Money::from_paisa(100_000)
            .allocate_by_weights(&[3, 1])
            .unwrap();
        assert_eq!(
            p,
            vec![Money::from_paisa(75_000), Money::from_paisa(25_000)]
        );
        // uneven: 1 paisa over three equal weights -> first gets it
        let p = Money::from_paisa(1)
            .allocate_by_weights(&[1, 1, 1])
            .unwrap();
        assert_eq!(
            p.iter().map(|m| m.paisa()).collect::<Vec<_>>(),
            vec![1, 0, 0]
        );
        for total in [-1001i64, 0, 5, 100_003] {
            let p = Money::from_paisa(total)
                .allocate_by_weights(&[7, 13, 29, 1])
                .unwrap();
            assert_eq!(p.iter().map(|m| m.paisa()).sum::<i64>(), total);
        }
        assert!(Money::from_paisa(1).allocate_by_weights(&[0, 0]).is_err());
        assert!(Money::from_paisa(1).allocate_by_weights(&[]).is_err());
        assert!(Money::from_paisa(1).allocate_by_weights(&[-1, 2]).is_err());
    }
}
