use anyhow::{Context, Result};
use clap::{Parser, Subcommand};
use datatool_core::dataset::Dataset;
use datatool_core::report;
use std::path::PathBuf;

#[derive(Parser)]
#[command(
    name = "datatool",
    version,
    about = "Understand a dataset in seconds: load, inspect and profile CSV, Excel, Parquet and JSON files"
)]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Show the first rows of a dataset
    Head {
        file: PathBuf,
        /// Number of rows to show
        #[arg(short = 'n', long, default_value_t = 10)]
        rows: usize,
    },
    /// Show each column's detected type and what was cleaned while loading
    Schema { file: PathBuf },
    /// Profile a dataset: statistics, data quality issues and a plain-language summary
    Profile {
        file: PathBuf,
        /// Also write a standalone HTML report to this path
        #[arg(long, value_name = "PATH")]
        html: Option<PathBuf>,
        /// Print the full profile as JSON instead of the text summary
        #[arg(long)]
        json: bool,
    },
}

fn main() -> Result<()> {
    match Cli::parse().command {
        Command::Head { file, rows } => {
            let ds = load(&file)?;
            println!("{}", ds.df.head(Some(rows)));
        }
        Command::Schema { file } => {
            let ds = load(&file)?;
            print_schema(&ds);
        }
        Command::Profile { file, html, json } => {
            let ds = load(&file)?;
            let profile = datatool_core::profile(&ds)
                .with_context(|| format!("failed to profile {}", file.display()))?;
            if json {
                println!("{}", serde_json::to_string_pretty(&profile)?);
            } else {
                print!("{}", report::text::render(&profile));
            }
            if let Some(path) = html {
                std::fs::write(&path, report::html::render(&profile))
                    .with_context(|| format!("failed to write {}", path.display()))?;
                eprintln!("HTML report written to {}", path.display());
            }
        }
    }
    Ok(())
}

fn load(file: &PathBuf) -> Result<Dataset> {
    datatool_core::load(file).with_context(|| format!("failed to load {}", file.display()))
}

fn print_schema(ds: &Dataset) {
    println!(
        "{} — {}, {} rows × {} columns\n",
        ds.name,
        ds.format,
        ds.rows(),
        ds.cols()
    );
    let width = ds
        .columns
        .iter()
        .map(|c| c.name.len())
        .max()
        .unwrap_or(4)
        .max(6);
    println!(
        "{:<width$}  {:<12}  {:>8}  notes",
        "column", "type", "missing"
    );
    for (meta, column) in ds.columns.iter().zip(ds.df.columns()) {
        let info = &meta.inference;
        let mut notes = Vec::new();
        if let Some(original) = &meta.original_name {
            notes.push(format!("renamed from {:?}", original));
        }
        if info.placeholder_nulls > 0 {
            notes.push(format!(
                "{} placeholder(s) like {:?} read as missing",
                info.placeholder_nulls,
                info.placeholder_examples
                    .first()
                    .map(String::as_str)
                    .unwrap_or("")
            ));
        }
        if info.unparsed > 0 {
            notes.push(format!(
                "{} value(s) not {} (e.g. {:?})",
                info.unparsed,
                info.inferred.label(),
                info.unparsed_examples
                    .first()
                    .map(String::as_str)
                    .unwrap_or("")
            ));
        }
        if info.whitespace_padded > 0 {
            notes.push(format!("{} value(s) trimmed", info.whitespace_padded));
        }
        if info.date_formats.len() > 1 {
            notes.push(format!("{} date formats", info.date_formats.len()));
        }
        println!(
            "{:<width$}  {:<12}  {:>8}  {}",
            meta.name,
            info.inferred.label(),
            column.null_count(),
            notes.join("; ")
        );
    }
}
