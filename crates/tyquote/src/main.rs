use std::fs::{self, File, OpenOptions};
use std::io::{Read, Write};
use std::path::{Path, PathBuf};

use anyhow::{bail, Context, Result};
use chrono::{DateTime, Utc};
use clap::{Parser, Subcommand};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

#[derive(Parser)]
#[command(version, about = "A personal, syncable quote collection")]
struct Cli {
    /// Config file (default: ./tyquote.yaml, then ~/.config/tyquote/config.yaml).
    #[arg(short, long, global = true)]
    config: Option<PathBuf>,
    /// Quote folder; overrides config (default: $XDG_DATA_HOME/tyquote/quotes).
    #[arg(long, global = true)]
    directory: Option<PathBuf>,
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Add a quote; use - to read its text from stdin.
    Add {
        text: String,
        #[arg(long)]
        author: Option<String>,
    },
    /// Print a randomly selected quote.
    Random,
    /// Print the most recently added quote.
    Latest,
    /// Print the same quote for the same local date and collection on every machine.
    Today,
    /// Validate settings without reading or writing the quote collection.
    ValidateConfig,
}

#[derive(Serialize, Deserialize)]
struct Quote {
    text: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    author: Option<String>,
    created_at: DateTime<Utc>,
}

fn random_bytes() -> Result<[u8; 16]> {
    let mut bytes = [0; 16];
    File::open("/dev/urandom")?.read_exact(&mut bytes)?;
    Ok(bytes)
}

fn load(directory: &Path) -> Result<Vec<(String, Quote)>> {
    let entries = match fs::read_dir(directory) {
        Ok(entries) => entries,
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(err) => return Err(err.into()),
    };
    let mut quotes = Vec::new();
    for entry in entries {
        let path = entry?.path();
        if path.extension().is_none_or(|ext| ext != "json") || !path.is_file() {
            continue;
        }
        let quote: Quote = serde_json::from_slice(&fs::read(&path)?)
            .with_context(|| format!("invalid quote {}", path.display()))?;
        if quote.text.trim().is_empty() {
            bail!("empty quote in {}", path.display());
        }
        quotes.push((
            path.file_name().unwrap().to_string_lossy().into_owned(),
            quote,
        ));
    }
    quotes.sort_by(|a, b| a.0.cmp(&b.0));
    Ok(quotes)
}

// Rendezvous hashing avoids dependence on filesystem enumeration order and minimizes
// changes to today's selection when a newly synced quote arrives.
fn today<'a>(quotes: &'a [(String, Quote)], date: &str) -> Option<&'a Quote> {
    quotes
        .iter()
        .max_by_key(|(id, _)| Sha256::digest(format!("{date}\0{id}").as_bytes()))
        .map(|(_, quote)| quote)
}

fn run(cli: Cli) -> Result<()> {
    let loaded = ty_common::config::load::<config::Config>("tyquote", cli.config)?;
    let config = loaded.config;
    config.validate()?;
    if matches!(cli.command, Command::ValidateConfig) {
        match loaded.path {
            Some(path) => println!("Config is valid: {}", path.display()),
            None => println!("No config file found; defaults are valid"),
        }
        return Ok(());
    }
    let directory = config.directory(cli.directory)?;
    if let Command::Add { mut text, author } = cli.command {
        if text == "-" {
            text.clear();
            std::io::stdin().read_to_string(&mut text)?;
        }
        let text = text.trim().to_owned();
        if text.is_empty() {
            bail!("quote text cannot be empty");
        }
        let quote = Quote {
            text,
            author: author
                .or_else(|| config.default_author.clone())
                .map(|s| s.trim().to_owned())
                .filter(|s| !s.is_empty()),
            created_at: Utc::now(),
        };
        fs::create_dir_all(&directory)?;
        let id = u128::from_le_bytes(random_bytes()?);
        let path = directory.join(format!("{id:032x}.json"));
        let temporary = directory.join(format!(".tyquote-{id:032x}.tmp"));
        let result = (|| -> Result<()> {
            let mut file = OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(&temporary)?;
            file.write_all(&serde_json::to_vec_pretty(&quote)?)?;
            file.write_all(b"\n")?;
            file.sync_all()?;
            // Publish atomically without replacing any existing quote.
            fs::hard_link(&temporary, &path)?;
            Ok(())
        })();
        let _ = fs::remove_file(&temporary);
        result?;
        println!("{}", path.display());
        return Ok(());
    }
    let quotes = load(&directory)?;
    if quotes.is_empty() {
        bail!("no quotes in {}; use tyquote add", directory.display());
    }
    let quote = match cli.command {
        Command::Random => {
            &quotes[(u128::from_le_bytes(random_bytes()?) % quotes.len() as u128) as usize].1
        }
        Command::Latest => {
            &quotes
                .iter()
                .max_by(|a, b| a.1.created_at.cmp(&b.1.created_at).then(a.0.cmp(&b.0)))
                .unwrap()
                .1
        }
        Command::Today => today(&quotes, &config.date(Utc::now())).unwrap(),
        Command::Add { .. } | Command::ValidateConfig => unreachable!(),
    };
    println!("{}", config.display(&quote.text, quote.author.as_deref()));
    Ok(())
}

fn main() {
    if let Err(err) = run(Cli::parse()) {
        eprintln!("tyquote: {err:#}");
        std::process::exit(1);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn daily_selection_is_independent_of_directory_order() {
        let mut quotes: Vec<_> = (0..20)
            .map(|i| {
                (
                    format!("{i}.json"),
                    Quote {
                        text: i.to_string(),
                        author: None,
                        created_at: Utc::now(),
                    },
                )
            })
            .collect();
        let selected = today(&quotes, "2026-11-12").unwrap().text.clone();
        quotes.reverse();
        assert_eq!(today(&quotes, "2026-11-12").unwrap().text, selected);
        assert!(today(&[], "2026-11-12").is_none());
        assert!((1..=30)
            .any(|day| today(&quotes, &format!("2026-11-{day:02}")).unwrap().text != selected));
    }
}
mod config;
