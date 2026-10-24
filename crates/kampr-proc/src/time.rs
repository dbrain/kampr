/// `strftime("%c")` in the C locale at UTC, which is what `LC_ALL=C TZ=UTC ps -o lstart=` prints:
/// `Thu Jan  1 00:00:00 1970`, the day padded with a space.
#[cfg_attr(not(target_os = "macos"), allow(dead_code))]
pub(crate) fn lstart(unix: i64) -> String {
    const DAYS: [&str; 7] = ["Thu", "Fri", "Sat", "Sun", "Mon", "Tue", "Wed"];
    const MONTHS: [&str; 12] = [
        "Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec",
    ];
    let (days, secs) = (unix.div_euclid(86_400), unix.rem_euclid(86_400));
    let (year, month, day) = civil(days);
    format!(
        "{} {} {day:>2} {:02}:{:02}:{:02} {year}",
        DAYS[days.rem_euclid(7) as usize],
        MONTHS[month as usize - 1],
        secs / 3600,
        secs / 60 % 60,
        secs % 60,
    )
}

/// Howard Hinnant's `civil_from_days`.
fn civil(days: i64) -> (i64, u32, u32) {
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = (doy - (153 * mp + 2) / 5 + 1) as u32;
    let month = if mp < 10 { mp + 3 } else { mp - 9 } as u32;
    let year = yoe + era * 400 + i64::from(month <= 2);
    (year, month, day)
}

#[cfg(test)]
mod tests {
    use super::lstart;

    #[test]
    fn a_start_is_written_as_ps_writes_lstart_in_the_c_locale_at_utc() {
        for (unix, want) in [
            (0, "Thu Jan  1 00:00:00 1970"),
            (951_782_400, "Tue Feb 29 00:00:00 2000"),
            (1_792_803_825, "Sat Oct 24 01:03:45 2026"),
            (1_735_689_599, "Tue Dec 31 23:59:59 2024"),
        ] {
            assert_eq!(lstart(unix), want, "{unix}");
        }
    }
}
