//! Timestamps are stored as UTC RFC 3339 text; business dates as `YYYY-MM-DD`.
//! Display zone is Asia/Dhaka (UTC+06:00, no DST since 2009), so a fixed offset is exact.

use chrono::{DateTime, FixedOffset, NaiveDate, SecondsFormat, Utc};

pub const DHAKA_OFFSET_SECS: i32 = 6 * 3600;

pub fn dhaka() -> FixedOffset {
    FixedOffset::east_opt(DHAKA_OFFSET_SECS).expect("valid offset")
}

pub fn now_utc_string() -> String {
    Utc::now().to_rfc3339_opts(SecondsFormat::Millis, true)
}

pub fn to_dhaka(ts: DateTime<Utc>) -> DateTime<FixedOffset> {
    ts.with_timezone(&dhaka())
}

/// Business "today" in Dhaka (day boundaries follow local midnight, not UTC).
pub fn today_dhaka() -> NaiveDate {
    to_dhaka(Utc::now()).date_naive()
}

pub fn format_dhaka(ts: DateTime<Utc>) -> String {
    to_dhaka(ts).format("%d %b %Y, %I:%M %p").to_string()
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;

    #[test]
    fn dhaka_is_utc_plus_6() {
        let ts = Utc.with_ymd_and_hms(2026, 1, 1, 20, 0, 0).unwrap();
        assert_eq!(format_dhaka(ts), "02 Jan 2026, 02:00 AM");
        assert_eq!(to_dhaka(ts).date_naive().to_string(), "2026-01-02");
    }

    #[test]
    fn now_string_is_utc_rfc3339() {
        let s = now_utc_string();
        assert!(s.ends_with('Z'));
        assert!(DateTime::parse_from_rfc3339(&s).is_ok());
    }
}
