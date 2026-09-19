use std::collections::BTreeMap;
use std::fs;
use std::io;
use std::path::PathBuf;

use clap::{Parser, Subcommand};

use envdrift::diff::{diff, DriftReport};
use envdrift::dotenv::parse;

#[derive(Parser)]
#[command(
    name = "envdrift",
    about = "Finds drift between .env.example and your real .env: variables missing, left empty, or undocumented",
    version
)]
struct Cli {
    #[command(subcommand)]
    command: Option<Command>,
}

#[derive(Subcommand)]
enum Command {
    /// Compare .env.example against .env and report missing/unfilled/extra
    /// variables. This is what runs if no subcommand is given.
    Check {
        /// The documentation file (source of truth for what's required).
        #[arg(long, default_value = ".env.example")]
        example: PathBuf,
        /// The real, gitignored file with actual values.
        #[arg(long, default_value = ".env")]
        env: PathBuf,
    },
    /// Append variables found in .env but undocumented in .env.example.
    /// Only key names are ever written — never real values from .env.
    Sync {
        #[arg(long, default_value = ".env.example")]
        example: PathBuf,
        #[arg(long, default_value = ".env")]
        env: PathBuf,
        /// Show what would be appended without writing the file.
        #[arg(long)]
        dry_run: bool,
    },
}

/// Reads and parses a `.env`-style file. A missing file is a hard error —
/// used for `.env.example`, which is expected to always exist and be the
/// source of truth.
fn read_vars_required(path: &PathBuf) -> anyhow::Result<BTreeMap<String, String>> {
    let text =
        fs::read_to_string(path).map_err(|e| anyhow::anyhow!("reading {}: {e}", path.display()))?;
    Ok(parse(&text))
}

/// Reads and parses a `.env`-style file, tolerating it not existing yet (a
/// real, common state: a fresh clone has `.env.example` but no `.env` until
/// someone creates one). Returns the parsed vars and whether the file
/// existed.
fn read_vars_optional(path: &PathBuf) -> anyhow::Result<(BTreeMap<String, String>, bool)> {
    match fs::read_to_string(path) {
        Ok(text) => Ok((parse(&text), true)),
        Err(e) if e.kind() == io::ErrorKind::NotFound => Ok((BTreeMap::new(), false)),
        Err(e) => Err(anyhow::anyhow!("reading {}: {e}", path.display())),
    }
}

fn run_check(example: &PathBuf, env: &PathBuf) -> anyhow::Result<bool> {
    let example_vars = read_vars_required(example)?;
    let (env_vars, env_existed) = read_vars_optional(env)?;

    println!(
        "envdrift: comparing {} -> {}",
        example.display(),
        env.display()
    );
    if !env_existed {
        println!(
            "note: {} does not exist yet — treating it as empty (every documented variable will show as missing)",
            env.display()
        );
    }
    println!();

    let report: DriftReport = diff(&example_vars, &env_vars);

    if !report.has_drift() {
        println!(
            "no drift: {} variable(s) match between {} and {}",
            report.matched.len(),
            example.display(),
            env.display()
        );
        return Ok(false);
    }

    if !report.missing.is_empty() {
        println!(
            "missing ({}): in {} but not in {} — will break at runtime",
            report.missing.len(),
            example.display(),
            env.display()
        );
        for k in &report.missing {
            println!("  {k}");
        }
        println!();
    }

    if !report.unfilled.is_empty() {
        println!(
            "unfilled ({}): present in {} but empty — {} suggests a real value is needed",
            report.unfilled.len(),
            env.display(),
            example.display()
        );
        for k in &report.unfilled {
            println!("  {k}");
        }
        println!();
    }

    if !report.extra.is_empty() {
        println!(
            "extra ({}): in {} but not documented in {} — onboarding gap",
            report.extra.len(),
            env.display(),
            example.display()
        );
        for k in &report.extra {
            println!("  {k}");
        }
        println!();
    }

    println!(
        "{} problem(s) found ({} matched cleanly)",
        report.problem_count(),
        report.matched.len()
    );

    Ok(true)
}

fn run_sync(example: &PathBuf, env: &PathBuf, dry_run: bool) -> anyhow::Result<()> {
    let (example_vars, _) = read_vars_optional(example)?;
    let (env_vars, env_existed) = read_vars_optional(env)?;

    if !env_existed {
        anyhow::bail!("{} does not exist — nothing to sync from", env.display());
    }

    let report = diff(&example_vars, &env_vars);

    if report.extra.is_empty() {
        println!(
            "envdrift: {} already documents every variable in {}, nothing to sync",
            example.display(),
            env.display()
        );
        return Ok(());
    }

    if dry_run {
        println!(
            "envdrift: would add {} key(s) to {} (dry run, nothing written):",
            report.extra.len(),
            example.display()
        );
        for k in &report.extra {
            println!("  {k}=");
        }
        return Ok(());
    }

    let existing = fs::read_to_string(example).unwrap_or_default();
    let mut out = existing;
    if !out.is_empty() && !out.ends_with('\n') {
        out.push('\n');
    }
    for k in &report.extra {
        out.push_str(k);
        out.push_str("=\n");
    }

    fs::write(example, out).map_err(|e| anyhow::anyhow!("writing {}: {e}", example.display()))?;

    println!(
        "envdrift: added {} key(s) to {}: {}",
        report.extra.len(),
        example.display(),
        report.extra.join(", ")
    );
    println!("(values left empty on purpose — never copied from .env)");

    Ok(())
}

fn main() -> anyhow::Result<()> {
    let cli = Cli::parse();

    let command = cli.command.unwrap_or(Command::Check {
        example: PathBuf::from(".env.example"),
        env: PathBuf::from(".env"),
    });

    match command {
        Command::Check { example, env } => {
            let has_drift = run_check(&example, &env)?;
            if has_drift {
                std::process::exit(1);
            }
            Ok(())
        }
        Command::Sync {
            example,
            env,
            dry_run,
        } => run_sync(&example, &env, dry_run),
    }
}
