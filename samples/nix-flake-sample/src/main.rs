// citytime: identical to samples/flox-clap-sample's own CLI, on purpose —
// this sample's point is the environment provider underneath (nix flakes
// instead of flox), not the CLI itself. See that sample's README for why
// clap's derive API and a fixed city table were chosen.

use chrono::Utc;
use chrono_tz::Tz;
use clap::{Parser, Subcommand};

#[derive(Parser)]
#[command(
    name = "citytime",
    version,
    about = "Look up the current time in a city"
)]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Print the current time in CITY
    Time {
        /// City name, e.g. "Bucharest", "Tokyo", "New York"
        city: String,
    },
    /// Print the CLI version
    Version,
}

/// City name -> IANA timezone. A real CLI would resolve this against a
/// geocoding/timezone service; kept as a small fixed table here so the
/// sample needs no network access at runtime, session or otherwise.
const CITY_TIMEZONES: &[(&str, Tz)] = &[
    ("bucharest", chrono_tz::Europe::Bucharest),
    ("london", chrono_tz::Europe::London),
    ("paris", chrono_tz::Europe::Paris),
    ("berlin", chrono_tz::Europe::Berlin),
    ("moscow", chrono_tz::Europe::Moscow),
    ("tokyo", chrono_tz::Asia::Tokyo),
    ("beijing", chrono_tz::Asia::Shanghai),
    ("mumbai", chrono_tz::Asia::Kolkata),
    ("dubai", chrono_tz::Asia::Dubai),
    ("sydney", chrono_tz::Australia::Sydney),
    ("new york", chrono_tz::America::New_York),
    ("los angeles", chrono_tz::America::Los_Angeles),
    ("chicago", chrono_tz::America::Chicago),
    ("sao paulo", chrono_tz::America::Sao_Paulo),
    ("cairo", chrono_tz::Africa::Cairo),
];

fn main() {
    let cli = Cli::parse();

    match cli.command {
        Command::Time { city } => match lookup(&city) {
            Some(tz) => {
                let now = Utc::now().with_timezone(&tz);
                println!("{city}: {}", now.format("%Y-%m-%d %H:%M:%S %Z"));
            }
            None => {
                eprintln!("citytime: unknown city {city:?}");
                eprintln!("known cities:");
                for (name, _) in CITY_TIMEZONES {
                    eprintln!("  {name}");
                }
                std::process::exit(1);
            }
        },
        Command::Version => {
            println!(env!("CARGO_PKG_VERSION"));
        }
    }
}

fn lookup(city: &str) -> Option<Tz> {
    let needle = city.trim().to_lowercase();
    CITY_TIMEZONES
        .iter()
        .find(|(name, _)| *name == needle)
        .map(|(_, tz)| *tz)
}
