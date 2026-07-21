//! Native deterministic match runner and wall-clock smoke harness.

use std::process::ExitCode;
use std::time::Instant;

use zoomieball_controller::ZoomieBackend;
use zoomieball_core::{Match, MatchConfig, Playbook, TICK_HZ};

fn main() -> ExitCode {
    match run() {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("zoomieball-headless: {error}");
            ExitCode::FAILURE
        }
    }
}

fn run() -> Result<(), String> {
    let mut arguments = std::env::args().skip(1);
    let active_per_team = parse_or(arguments.next(), 10usize, "roster")?;
    let ticks = parse_or(arguments.next(), 256u32, "ticks")?;
    let hashes = match arguments.next().as_deref() {
        None => false,
        Some("--hashes") => true,
        Some(argument) => return Err(format!("unexpected argument: {argument}")),
    };
    if arguments.next().is_some() {
        return Err("usage: zoomieball-headless [10|100] [ticks] [--hashes]".to_owned());
    }
    if !matches!(active_per_team, 10 | 100) {
        return Err("roster must be 10 or 100".to_owned());
    }
    let playbook = Playbook::compile_ron(include_str!("../../../assets/default-playbook.ron"))
        .map_err(|error| error.to_string())?;
    let controller = ZoomieBackend::new(active_per_team, 0x005a_001e_ba11);
    let mut game = Match::new(
        MatchConfig {
            active_per_team,
            ..MatchConfig::default()
        },
        playbook,
        controller,
    );
    let start = Instant::now();
    for tick in 0..ticks {
        let hash = game.tick();
        if hashes {
            println!(
                "tick={} world={:016x} controller={:016x} learning={:016x} combined={:016x}",
                tick + 1,
                hash.world,
                hash.controller,
                hash.learning,
                hash.combined,
            );
        }
    }
    if hashes {
        println!(
            "final ticks={} hash={:016x} score={:?}",
            ticks,
            game.last_hash().combined,
            game.world().scores(),
        );
        return Ok(());
    }
    let elapsed = start.elapsed();
    let simulated = f64::from(ticks) / f64::from(TICK_HZ);
    println!(
        "roster={}v{} ticks={} hash={:016x} score={:?} wall={:.3}s realtime={:.2}x",
        active_per_team,
        active_per_team,
        ticks,
        game.last_hash().combined,
        game.world().scores(),
        elapsed.as_secs_f64(),
        simulated / elapsed.as_secs_f64(),
    );
    Ok(())
}

fn parse_or<T: std::str::FromStr>(
    value: Option<String>,
    default: T,
    name: &str,
) -> Result<T, String> {
    value.map_or(Ok(default), |value| {
        value
            .parse()
            .map_err(|_| format!("invalid {name}: {value}"))
    })
}
