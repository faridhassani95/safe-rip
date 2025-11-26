// src/main.rs - rip: safe rm that moves files to trash instead of deleting them permanently
mod fs_utils;
mod trash;

use anyhow::Result;
use clap::{CommandFactory, Parser};
use crate::trash::*;

#[derive(Parser, Debug)]
#[command(
    name = "rip",
    about = "A safe alternative to rm — moves files to trash instead of deleting permanently",
    version = "1.0.0",
    author = "Farid",
    long_about = None
)]
struct Cli {
    #[arg(
        long,
        value_name = "POLICY",
        help = "Show current or set auto-clean policy: ask, never, 30d, ... (no value = show current)"
    )]
    keep: Option<Option<String>>,

    #[arg(long, help = "List items currently in trash")]
    list: bool,

    #[arg(long, help = "Permanently empty the trash")]
    empty: bool,

    #[arg(long, value_name = "N", help = "Restore the Nth item from trash (1 = newest)")]
    restore: Option<usize>,

    #[arg(value_name = "FILE", trailing_var_arg = true, help = "Files, directories or symlinks to move to trash")]
    files: Vec<String>,
}

fn main() -> Result<()> {
    let cli = Cli::parse();

    if let Some(policy_opt) = cli.keep {
        match policy_opt {
            Some(policy) => { set_keep_policy(&policy)?; }
            None => { show_keep_policy()?; }
        }
    } else if cli.list {
        list_trash()?;
    } else if cli.empty {
        empty_trash()?;
    } else if let Some(n) = cli.restore {
        restore_nth(n)?;
    } else if cli.files.is_empty() {
        Cli::command().print_help()?;
    } else {
        let mut had_error = false;
        for path in &cli.files {
            if let Err(e) = move_to_trash(path) {
                eprintln!("rip: {path}: {e}");
                had_error = true;
            }
        }
        if had_error {
            std::process::exit(1);
        }
    }
    Ok(())
}