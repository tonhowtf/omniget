use crate::byte_reader::Reader;
use core::str::FromStr;

/// A date time.
#[derive(Debug, Copy, Clone, Eq, PartialEq, Hash)]
pub struct DateTime {
    /// The year.
    pub year: u16,
    /// The year.
    pub month: u8,
    /// The day.
    pub day: u8,
    /// The hour.
    pub hour: u8,
    /// The minute.
    pub minute: u8,
    /// The second.
    pub second: u8,
    /// The offset in hours from UTC.
    pub utc_offset_hour: i8,
    /// The offset in minutes from UTC.
    pub utc_offset_minute: u8,
}

impl DateTime {
    pub(crate) fn from_bytes(bytes: &[u8]) -> Option<Self> {
        let mut reader = Reader::new(bytes);

        reader.forward_tag(b"D:")?;

        let read_num = |reader: &mut Reader<'_>, bytes: u8, min: u16, max: u16| -> Option<u16> {
            if matches!(reader.peek_byte()?, b'-' | b'+' | b'Z') {
                return None;
            }

            let num = u16::from_str(core::str::from_utf8(reader.read_bytes(bytes as usize)?).ok()?)
                .ok()?;

            if num < min || num > max {
                return None;
            }

            Some(num)
        };

        let year = read_num(&mut reader, 4, 0, 9999)?;
        let month = read_num(&mut reader, 2, 1, 12)
            .map(|n| n as u8)
            .unwrap_or(1);
        let day = read_num(&mut reader, 2, 1, 31)
            .map(|n| n as u8)
            .unwrap_or(1);
        let hour = read_num(&mut reader, 2, 0, 23)
            .map(|n| n as u8)
            .unwrap_or(0);
        let minute = read_num(&mut reader, 2, 0, 59)
            .map(|n| n as u8)
            .unwrap_or(0);
        let second = read_num(&mut reader, 2, 0, 59)
            .map(|n| n as u8)
            .unwrap_or(0);

        let (utc_offset_hour, utc_offset_minute) = if !reader.at_end() {
            let multiplier = match reader.read_byte()? {
                b'-' => -1,
                _ => 1,
            };

            let hour = multiplier
                * read_num(&mut reader, 2, 0, 23)
                    .map(|n| n as i8)
                    .unwrap_or(0);
            reader.forward_tag(b"\'");
            let minute = read_num(&mut reader, 2, 0, 59)
                .map(|n| n as u8)
                .unwrap_or(0);

            (hour, minute)
        } else {
            (0, 0)
        };

        Some(Self {
            year,
            month,
            day,
            hour,
            minute,
            second,
            utc_offset_hour,
            utc_offset_minute,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::DateTime;

    #[allow(clippy::too_many_arguments)]
    fn dt(
        year: u16,
        month: u8,
        day: u8,
        hour: u8,
        minute: u8,
        second: u8,
        utc_hour: i8,
        utc_minute: u8,
    ) -> DateTime {
        DateTime {
            year,
            month,
            day,
            hour,
            minute,
            second,
            utc_offset_hour: utc_hour,
            utc_offset_minute: utc_minute,
        }
    }

    fn parse(str: &str) -> DateTime {
        DateTime::from_bytes(str.as_bytes()).unwrap()
    }

    #[test]
    fn year_only_defaults() {
        assert_eq!(parse("D:2023"), dt(2023, 1, 1, 0, 0, 0, 0, 0));
    }

    #[test]
    fn year_month_defaults() {
        assert_eq!(parse("D:202312"), dt(2023, 12, 1, 0, 0, 0, 0, 0));
    }

    #[test]
    fn year_month_day_defaults() {
        assert_eq!(parse("D:20231225"), dt(2023, 12, 25, 0, 0, 0, 0, 0));
    }

    #[test]
    fn ymdh() {
        assert_eq!(parse("D:2023122514"), dt(2023, 12, 25, 14, 0, 0, 0, 0));
    }

    #[test]
    fn ymdhm() {
        assert_eq!(parse("D:202312251430"), dt(2023, 12, 25, 14, 30, 0, 0, 0));
    }

    #[test]
    fn full_local_time() {
        assert_eq!(
            parse("D:20231225143015"),
            dt(2023, 12, 25, 14, 30, 15, 0, 0)
        );
    }

    #[test]
    fn example_from_spec() {
        assert_eq!(
            parse("D:199812231952-08'00"),
            dt(1998, 12, 23, 19, 52, 0, -8, 0)
        );
    }

    #[test]
    fn positive_offset_with_minutes() {
        assert_eq!(
            parse("D:20230701120000+05'30"),
            dt(2023, 7, 1, 12, 0, 0, 5, 30)
        );
    }

    #[test]
    fn utc_z() {
        assert_eq!(parse("D:20230701120000Z"), dt(2023, 7, 1, 12, 0, 0, 0, 0));
    }

    #[test]
    fn utc_z_with_zero_offsets() {
        assert_eq!(
            parse("D:20230701120000Z00'00"),
            dt(2023, 7, 1, 12, 0, 0, 0, 0)
        );
    }

    #[test]
    fn negative_offset_with_minutes() {
        assert_eq!(
            parse("D:20230701120000-03'15"),
            dt(2023, 7, 1, 12, 0, 0, -3, 15)
        );
    }

    #[test]
    fn leap_year() {
        assert_eq!(
            parse("D:20000229010203+01'00"),
            dt(2000, 2, 29, 1, 2, 3, 1, 0)
        );
    }

    #[test]
    fn max_values() {
        assert_eq!(
            parse("D:99991231235959+14'00"),
            dt(9999, 12, 31, 23, 59, 59, 14, 0)
        );
    }

    #[test]
    fn min_values() {
        assert_eq!(parse("D:00000101000000+00'00"), dt(0, 1, 1, 0, 0, 0, 0, 0));
    }

    #[test]
    fn offset_hour_only() {
        assert_eq!(parse("D:202307011200+02"), dt(2023, 7, 1, 12, 0, 0, 2, 0));
    }

    #[test]
    fn offset_negative_zero_hour() {
        assert_eq!(
            parse("D:202307011200-00'45"),
            dt(2023, 7, 1, 12, 0, 0, 0, 45)
        );
    }
}
