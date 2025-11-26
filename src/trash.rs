// src/trash.rs - Core trash implementation with symlink safety and configurable auto-clean policies
use crate::fs_utils::{copy_recursively, remove_recursively};
use anyhow::{anyhow, Context, Result};
use chrono::{DateTime, Duration, Local, Utc, SecondsFormat};
use nanoid::nanoid;
use std::env;
use std::fs;
use std::io::{self, BufRead, Write};
use std::path::PathBuf;
use urlencoding::{decode, encode};

#[derive(Clone, Debug)]
pub struct TrashItem {
    pub original_path: PathBuf,
    pub deletion_date: DateTime<Utc>,
    pub trashed_name: String,
    pub info_path: PathBuf,
}

#[derive(Debug, Clone, PartialEq)]
enum KeepPolicy {
    Never,
    Days(i64),
    AskBeforeDelete,
}

static mut KEEP_POLICY: KeepPolicy = KeepPolicy::Days(30);

pub fn set_keep_policy(policy: &str) -> Result<()> {
    let p = policy.trim().to_lowercase();
    unsafe {
        KEEP_POLICY = match p.as_str() {
            "never" => KeepPolicy::Never,
            "ask" => KeepPolicy::AskBeforeDelete,
            s if s.ends_with('d') => {
                let days = s.trim_end_matches('d')
                    .parse::<i64>()
                    .map_err(|_| anyhow!("Invalid day count: {s}"))?;
                if days <= 0 {
                    KeepPolicy::Never
                } else {
                    KeepPolicy::Days(days)
                }
            }
            _ => return Err(anyhow!("Valid policies: ask | never | 30d | 90d | ...")),
        };
        println!("Auto-clean policy set to: {KEEP_POLICY:#?}");
    }
    Ok(())
}

pub fn show_keep_policy() -> Result<()> {
    unsafe {
        println!("Current auto-clean policy: {KEEP_POLICY:#?}");
    }
    Ok(())
}

fn confirm(prompt: &str) -> bool {
    print!("{prompt}");
    let _ = io::stdout().flush();
    io::stdin().lock().lines()
        .next()
        .and_then(|l| l.ok())
        .map(|s| s.trim().to_lowercase() == "y")
        .unwrap_or(false)
}

fn cleanup_old_trash() -> Result<()> {
    let items = load_trash_items()?;
    if items.is_empty() {
        return Ok(());
    }

    unsafe {
        match KEEP_POLICY {
            KeepPolicy::Never => {}
            KeepPolicy::Days(days) => {
                let cutoff = Utc::now() - Duration::days(days);
                let mut deleted = 0;
                for item in &items {
                    if item.deletion_date < cutoff {
                        let trash = find_trash_dir()?;
                        let _ = fs::remove_file(trash.join("files").join(&item.trashed_name));
                        let _ = fs::remove_file(&item.info_path);
                        deleted += 1;
                    }
                }
                if deleted > 0 {
                    println!("Auto-cleaned {deleted} items older than {days} days");
                }
            }
            KeepPolicy::AskBeforeDelete => {
                let cutoff = Utc::now() - Duration::days(30);
                let old: Vec<_> = items.iter().filter(|i| i.deletion_date < cutoff).collect();
                if !old.is_empty() && confirm(&format!("{old_len} old items found. Permanently delete them? [y/N] ", old_len = old.len())) {
                    for item in &old {
                        let trash = find_trash_dir()?;
                        let _ = fs::remove_file(trash.join("files").join(&item.trashed_name));
                        let _ = fs::remove_file(&item.info_path);
                    }
                    println!("Permanently deleted {old_len} old items.", old_len = old.len());
                }
            }
        }
    }
    Ok(())
}

pub fn find_trash_dir() -> Result<PathBuf> {
    let trash = if let Ok(xdg) = env::var("XDG_DATA_HOME") {
        if !xdg.is_empty() {
            PathBuf::from(xdg).join("Trash")
        } else {
            dirs_next::home_dir().unwrap().join(".local/share/Trash")
        }
    } else {
        dirs_next::home_dir().unwrap().join(".local/share/Trash")
    };
    let _ = fs::create_dir_all(&trash);
    let _ = fs::create_dir_all(trash.join("files"));
    let _ = fs::create_dir_all(trash.join("info"));
    Ok(trash)
}

fn generate_unique_name(original: &std::path::Path) -> String {
    let stem = original.file_stem().and_then(|s| s.to_str()).unwrap_or("file");
    let ext = original.extension().and_then(|s| s.to_str()).unwrap_or("");
    let id = nanoid!(10);
    if ext.is_empty() {
        format!("{stem}_{id}")
    } else {
        format!("{stem}_{id}.{ext}")
    }
}

pub fn move_to_trash(path_str: &str) -> Result<()> {
    let _ = cleanup_old_trash();
    let original_path = std::path::Path::new(path_str);
    let metadata = original_path
        .symlink_metadata()
        .with_context(|| format!("No such file or directory: {path_str}"))?;
    let original_absolute = if original_path.is_absolute() {
        original_path.to_path_buf()
    } else {
        env::current_dir()?.join(original_path)
    };

    let trash = find_trash_dir()?;
    let files_dir = trash.join("files");
    let info_dir = trash.join("info");
    let trashed_name = generate_unique_name(original_path);
    let dest_file = files_dir.join(&trashed_name);
    let info_file = info_dir.join(format!("{trashed_name}.trashinfo"));
    let deletion_date = Utc::now().to_rfc3339_opts(SecondsFormat::Secs, true);
    let encoded_path = encode(
        original_absolute
            .to_str()
            .context("non-UTF8 path")?
    ).to_string();

    let moved = if metadata.file_type().is_symlink() {
        let _ = fs::remove_file(&dest_file);
        if let Ok(target) = fs::read_link(original_path) {
            std::os::unix::fs::symlink(target, &dest_file).is_ok()
        } else {
            let _ = std::os::unix::fs::symlink("/RIP_BROKEN_LINK", &dest_file);
            true
        }
    } else if metadata.is_dir() {
        copy_recursively(original_path, &dest_file)?;
        remove_recursively(original_path)?;
        true
    } else if fs::rename(original_path, &dest_file).is_ok() {
        true
    } else {
        fs::copy(original_path, &dest_file)?;
        fs::remove_file(original_path)?;
        true
    };

    if moved {
        fs::write(&info_file, format!("[Trash Info]\nPath={encoded_path}\nDeletionDate={deletion_date}\n"))?;
        Ok(())
    } else {
        Err(anyhow!("Failed to move '{path_str}' to trash"))
    }
}

fn load_trash_items() -> Result<Vec<TrashItem>> {
    let trash = find_trash_dir()?;
    let info_dir = trash.join("info");
    let files_dir = trash.join("files");
    let mut items = Vec::new();

    let Ok(entries) = fs::read_dir(&info_dir) else {
        return Ok(items);
    };

    for entry in entries.flatten() {
        let info_path = entry.path();
        if info_path.extension().and_then(|s| s.to_str()) != Some("trashinfo") {
            continue;
        }
        let content = match fs::read_to_string(&info_path) {
            Ok(c) => c,
            Err(_) => {
                let _ = fs::remove_file(&info_path);
                continue;
            }
        };

        let mut path_val = None;
        let mut date_val = None;
        for line in content.lines() {
            if let Some(v) = line.strip_prefix("Path=") {
                path_val = Some(v.to_owned());
            }
            if let Some(v) = line.strip_prefix("DeletionDate=") {
                date_val = Some(v.to_owned());
            }
        }

        let Some(path_val) = path_val else { continue };
        let Some(date_str) = date_val else { continue };

        let original_path = match decode(&path_val) {
            Ok(p) => PathBuf::from(p.into_owned()),
            Err(_) => continue,
        };

        let deletion_date = DateTime::parse_from_rfc3339(&date_str)
            .or_else(|_| DateTime::parse_from_rfc3339(&format!("{date_str}Z")))
            .map(|dt| dt.with_timezone(&Utc))
            .unwrap_or_else(|_| Utc::now());

        let trashed_name = info_path
            .file_stem()
            .and_then(|s| s.to_str())
            .unwrap_or("unknown")
            .to_owned();

        if files_dir.join(&trashed_name).exists() {
            items.push(TrashItem {
                original_path,
                deletion_date,
                trashed_name,
                info_path,
            });
        } else {
            let _ = fs::remove_file(&info_path);
        }
    }

    items.sort_by_key(|i| std::cmp::Reverse(i.deletion_date));
    Ok(items)
}

pub fn list_trash() -> Result<()> {
    let items = load_trash_items()?;
    if items.is_empty() {
        println!("Trash is empty");
        return Ok(());
    }
    println!(" # Date & Time                 Original Path");
    println!("────────────────────────────────────────────────────────────────");
    for (i, item) in items.iter().enumerate() {
        println!(
            "{:>3} {}  {}",
            i + 1,
            item.deletion_date
                .with_timezone(&Local)
                .format("%Y-%m-%d %H:%M:%S"),
            item.original_path.display()
        );
    }
    Ok(())
}

pub fn restore_nth(n: usize) -> Result<()> {
    let items = load_trash_items()?;
    let item = items.get(n - 1).context("No such item")?.clone();
    let trash = find_trash_dir()?;
    let src = trash.join("files").join(&item.trashed_name);
    let mut target = item.original_path.clone();

    if target.exists() {
        let stem = target.file_stem().and_then(|s| s.to_str()).unwrap_or("restored");
        let ext = target.extension().and_then(|s| s.to_str()).unwrap_or("");
        let date = Local::now().format("%Y-%m-%d");
        let mut p = target.parent().unwrap_or(std::path::Path::new(".")).to_path_buf();
        p.push(format!("{stem} (restored {date})"));
        if !ext.is_empty() {
            p.set_extension(ext);
        }
        target = p;
    }

    fs::rename(&src, &target)?;
    fs::remove_file(&item.info_path)?;
    println!("Restored: {}", target.display());
    Ok(())
}

pub fn empty_trash() -> Result<()> {
    let trash = find_trash_dir()?;
    for sub in ["files", "info"] {
        let p = trash.join(sub);
        if p.exists() {
            fs::remove_dir_all(&p)?;
            fs::create_dir(&p)?;
        }
    }
    println!("Trash emptied");
    Ok(())
}