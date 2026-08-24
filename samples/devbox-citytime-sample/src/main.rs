// citytime: the same idea as samples/flox-clap-sample and
// samples/nix-flake-sample's own citytime CLIs, adapted rather than
// copied verbatim — this one uses std only, no clap, no chrono. See
// README's "Why no clap, no chrono" for why: this sample's point is the
// devbox provider underneath, not the CLI, and a dependency-free binary
// keeps it that way without needing to solve a problem the other two
// samples don't have (see the README).

use std::env;
use std::time::{SystemTime, UNIX_EPOCH};

/// City name -> fixed UTC offset in whole hours. A real CLI would resolve
/// this against a timezone database (the other two citytime samples use
/// `chrono_tz` for exactly that); a fixed table with no DST handling is a
/// deliberate simplification here, not an oversight — it's what keeps
/// this sample dependency-free.
const CITY_UTC_OFFSETS: &[(&str, i64)] = &[
    ("bucharest", 2),
    ("london", 0),
    ("paris", 1),
    ("berlin", 1),
    ("moscow", 3),
    ("tokyo", 9),
    ("beijing", 8),
    ("mumbai", 5), // rounded from UTC+5:30
    ("dubai", 4),
    ("sydney", 10),
    ("new york", -5),
    ("los angeles", -8),
    ("chicago", -6),
    ("sao paulo", -3),
    ("cairo", 2),
];

fn main() {
    let args: Vec<String> = env::args().skip(1).collect();
    match args.first().map(String::as_str) {
        Some("time") => match args.get(1) {
            Some(city) => match lookup(city) {
                Some(offset) => println!("{city}: {}", format_local(offset)),
                None => {
                    eprintln!("citytime: unknown city {city:?}");
                    eprintln!("known cities:");
                    for (name, _) in CITY_UTC_OFFSETS {
                        eprintln!("  {name}");
                    }
                    std::process::exit(1);
                }
            },
            None => {
                eprintln!("usage: citytime time <city>");
                std::process::exit(2);
            }
        },
        Some("version") => println!(env!("CARGO_PKG_VERSION")),
        _ => {
            eprintln!("usage: citytime <time <city>|version>");
            std::process::exit(2);
        }
    }
}

fn lookup(city: &str) -> Option<i64> {
    let needle = city.trim().to_lowercase();
    CITY_UTC_OFFSETS
        .iter()
        .find(|(name, _)| *name == needle)
        .map(|(_, offset)| *offset)
}

/// Formats the current time at a fixed UTC offset without pulling in a
/// date/time crate: civil-from-days, the same well-known algorithm
/// `std::time` itself has no built-in calendar conversion for.
fn format_local(utc_offset_hours: i64) -> String {
    let now_secs = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_secs() as i64;
    let local_secs = now_secs + utc_offset_hours * 3600;

    let days = local_secs.div_euclid(86400);
    let secs_of_day = local_secs.rem_euclid(86400);
    let (hour, min, sec) = (
        secs_of_day / 3600,
        (secs_of_day / 60) % 60,
        secs_of_day % 60,
    );
    let (year, month, day) = civil_from_days(days);

    format!(
        "{year:04}-{month:02}-{day:02} {hour:02}:{min:02}:{sec:02} UTC{:+03}:00",
        utc_offset_hours
    )
}

/// Howard Hinnant's `civil_from_days`: days since the Unix epoch to a
/// proleptic-Gregorian (year, month, day). Public-domain algorithm, no
/// crate needed for what `chrono` would otherwise provide here.
fn civil_from_days(z: i64) -> (i64, u32, u32) {
    let z = z + 719468;
    let era = if z >= 0 { z } else { z - 146096 } / 146097;
    let doe = (z - era * 146097) as u64;
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146096) / 365;
    let y = yoe as i64 + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = (doy - (153 * mp + 2) / 5 + 1) as u32;
    let m = if mp < 10 { mp + 3 } else { mp - 9 } as u32;
    (if m <= 2 { y + 1 } else { y }, m, d)
}
