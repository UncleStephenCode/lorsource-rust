//! Decode contract for Java/JDBC `timestamp without time zone` columns.
//!
//! PgJDBC interprets these wall-clock values in the JVM default timezone.
//! PostgreSQL's `AT TIME ZONE` operator gives Rust the corresponding instant;
//! callers bind the verified IANA name as parameter one.

#[cfg(test)]
pub const S_LEGACY_TIMESTAMP_SQL_EXPRESSION: &str = "c.edit_date AT TIME ZONE $1::text";

pub fn sLegacyJdbcTimezone(stTimezone: chrono_tz::Tz) -> &'static str {
    stTimezone.name()
}

#[cfg(test)]
mod tests {
    use chrono::{LocalResult, NaiveDate, TimeZone};

    use super::{S_LEGACY_TIMESTAMP_SQL_EXPRESSION, sLegacyJdbcTimezone};

    #[test]
    fn timezone_parameter_is_a_canonical_iana_name() {
        assert_eq!(
            sLegacyJdbcTimezone(chrono_tz::Europe::Berlin),
            "Europe/Berlin"
        );
        assert_eq!(
            S_LEGACY_TIMESTAMP_SQL_EXPRESSION,
            "c.edit_date AT TIME ZONE $1::text"
        );
    }

    #[test]
    fn dst_reference_values_are_not_equivalent_to_hardcoded_utc() {
        let stTimezone = chrono_tz::Europe::Berlin;
        let dtSummer = NaiveDate::from_ymd_opt(2026, 7, 1)
            .unwrap()
            .and_hms_opt(12, 0, 0)
            .unwrap();
        let dtWinter = NaiveDate::from_ymd_opt(2026, 1, 1)
            .unwrap()
            .and_hms_opt(12, 0, 0)
            .unwrap();
        assert_eq!(
            stTimezone
                .from_local_datetime(&dtSummer)
                .single()
                .unwrap()
                .to_utc()
                .to_rfc3339(),
            "2026-07-01T10:00:00+00:00"
        );
        assert_eq!(
            stTimezone
                .from_local_datetime(&dtWinter)
                .single()
                .unwrap()
                .to_utc()
                .to_rfc3339(),
            "2026-01-01T11:00:00+00:00"
        );

        let dtOverlap = NaiveDate::from_ymd_opt(2026, 10, 25)
            .unwrap()
            .and_hms_opt(2, 30, 0)
            .unwrap();
        assert!(matches!(
            stTimezone.from_local_datetime(&dtOverlap),
            LocalResult::Ambiguous(_, _)
        ));

        let dtGap = NaiveDate::from_ymd_opt(2026, 3, 29)
            .unwrap()
            .and_hms_opt(2, 30, 0)
            .unwrap();
        assert!(matches!(
            stTimezone.from_local_datetime(&dtGap),
            LocalResult::None
        ));
    }
}
