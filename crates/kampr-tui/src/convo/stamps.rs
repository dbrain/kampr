//! Stamps. `at` is RFC 3339 **with an explicit zone** — every adapter in tree writes a `Z` and
//! says so (#285) — which is what lets a clock face be drawn in the reader's own zone. A stamp
//! that names no zone is a floating local time whose only honest reading is an elapsed one, and
//! UTC must not be assumed for it.

use std::sync::OnceLock;

pub fn when(at: &str) -> Option<String> {
    let (civil, zone) = split_zone(at)?;
    let (year, month, day, hour, minute, second) = civil;
    let stamp =
        days_from_civil(year, month, day) * 86_400 + hour as i64 * 3600 + minute as i64 * 60 + second as i64;
    let Some(zone) = zone else {
        return Some(elapsed(now().saturating_sub(stamp)));
    };
    let instant = stamp - zone as i64 * 60;
    let local = local_offset()?;
    let face = instant + local as i64 * 60;
    let (y, m, d, hh, mm) = civil_from(face);
    let (ty, tm, td, _, _) = civil_from(now() + local as i64 * 60);
    Some(match (y, m, d) == (ty, tm, td) {
        true => format!("{hh:02}:{mm:02}"),
        false => format!("{d} {} {hh:02}:{mm:02}", MONTHS[(m - 1) as usize]),
    })
}

const MONTHS: [&str; 12] = [
    "Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec",
];

fn elapsed(seconds: i64) -> String {
    match seconds {
        s if s < 90 => "just now".to_string(),
        s if s < 5400 => format!("{}m ago", s / 60),
        s if s < 172_800 => format!("{}h ago", s / 3600),
        s => format!("{}d ago", s / 86_400),
    }
}

fn now() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or_default()
}

/// The offset this machine is at, which std cannot answer and this crate has no date library to
/// ask. One process, once, cached for the run.
fn local_offset() -> Option<i32> {
    static CACHED: OnceLock<Option<i32>> = OnceLock::new();
    *CACHED.get_or_init(|| {
        let out = std::process::Command::new("date").arg("+%z").output().ok()?;
        offset_minutes(String::from_utf8(out.stdout).ok()?.trim())
    })
}

type Civil = (i64, u32, u32, u32, u32, u32);

fn split_zone(at: &str) -> Option<(Civil, Option<i32>)> {
    let at = at.trim();
    let (date, rest) = at.split_once(['T', 't', ' '])?;
    let mut date = date.split('-');
    let year: i64 = date.next()?.parse().ok()?;
    let month: u32 = date.next()?.parse().ok()?;
    let day: u32 = date.next()?.parse().ok()?;
    let cut = rest
        .char_indices()
        .find(|(i, c)| matches!(c, 'Z' | 'z' | '+') || (*c == '-' && *i > 0))
        .map(|(i, _)| i);
    let (clock, zone) = match cut {
        Some(cut) => (&rest[..cut], Some(&rest[cut..])),
        None => (rest, None),
    };
    let clock = clock.split('.').next().unwrap_or(clock);
    let mut clock = clock.split(':');
    let hour: u32 = clock.next()?.parse().ok()?;
    let minute: u32 = clock.next().unwrap_or("0").parse().ok()?;
    let second: u32 = clock.next().unwrap_or("0").parse().ok()?;
    let zone = zone.and_then(offset_minutes);
    Some(((year, month, day, hour, minute, second), zone))
}

fn offset_minutes(text: &str) -> Option<i32> {
    let text = text.trim();
    if matches!(text, "Z" | "z") {
        return Some(0);
    }
    let sign = match text.chars().next()? {
        '+' => 1,
        '-' => -1,
        _ => return None,
    };
    let digits: String = text[1..].chars().filter(char::is_ascii_digit).collect();
    let (hours, minutes) = match digits.len() {
        2 => (digits.parse::<i32>().ok()?, 0),
        4 => (digits[..2].parse::<i32>().ok()?, digits[2..].parse::<i32>().ok()?),
        _ => return None,
    };
    Some(sign * (hours * 60 + minutes))
}

fn days_from_civil(year: i64, month: u32, day: u32) -> i64 {
    let year = year - i64::from(month <= 2);
    let era = if year >= 0 { year } else { year - 399 } / 400;
    let yoe = year - era * 400;
    let mp = ((month + 9) % 12) as i64;
    let doy = (153 * mp + 2) / 5 + day as i64 - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146_097 + doe - 719_468
}

fn civil_from(seconds: i64) -> (i64, u32, u32, u32, u32) {
    let days = seconds.div_euclid(86_400);
    let rest = seconds.rem_euclid(86_400);
    let z = days + 719_468;
    let era = if z >= 0 { z } else { z - 146_096 } / 146_097;
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let year = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = (doy - (153 * mp + 2) / 5 + 1) as u32;
    let month = match mp < 10 {
        true => mp + 3,
        false => mp - 9,
    } as u32;
    let year = year + i64::from(month <= 2);
    (
        year,
        month,
        day,
        (rest / 3600) as u32,
        ((rest % 3600) / 60) as u32,
    )
}
