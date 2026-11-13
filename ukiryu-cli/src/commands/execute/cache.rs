//! `ukiryu cache info|stats|clear` (`CacheCommand`, a Thor subcommand).
//!
//! The source reads `Definition::DefinitionCache.instance`; the handler
//! takes the caller's [`DefinitionCache`] (in the CLI a fresh, empty cache:
//! nothing in a CLI process populates the singleton before the command
//! runs).
//!
//! Output is Thor's `say`, not `BaseCommand#say`: text is coloured only when
//! stdout is a terminal, `TERM` is not `dumb` and `NO_COLOR` is unset or
//! empty. Thor 1.5.0 has no `DIM` colour, so a `:dim` line on a colour
//! terminal raises `NameError` (`uninitialized constant
//! Thor::Shell::Color::DIM`) after the lines before it were written.
//!
//! `--verbose` is accepted and unused, as in the source.
use super::{ExecuteContext, ExecuteError};
use crate::commands::CommandOutcome;
use ukiryu::definition::cache::{CachedDefinition, DefinitionCache};
use ukiryu::render::OutputWrite;

/// The cache action.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CacheAction {
    Info,
    Stats,
    Clear,
}

/// `ukiryu cache ACTION` options, as given.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct CacheInput {
    pub action: CacheAction,
    /// `--dry-run` (class option, default false).
    pub dry_run: bool,
    /// `--verbose` (class option, default false; unused).
    pub verbose: bool,
}

/// Thor colour names `CacheCommand` uses.
#[derive(Clone, Copy)]
enum ThorColor {
    Cyan,
    Clear,
    Green,
    White,
    Dim,
}

/// Ordered Thor `say` writes.
struct ThorOut {
    writes: Vec<OutputWrite>,
    colors: bool,
}

impl ThorOut {
    fn new(ctx: &ExecuteContext) -> ThorOut {
        let env = |k: &str| ctx.host.env.get(k);
        let supported = ctx.stdout_tty && env("TERM") != Some("dumb");
        let disabled = env("NO_COLOR").is_some_and(|v| !v.is_empty());
        ThorOut {
            writes: Vec::new(),
            colors: supported && !disabled,
        }
    }

    /// `say(message, color)`: `set_color` when colours display, then a
    /// newline unless the message ends in a space, tab or newline.
    fn say(&mut self, message: &str, color: ThorColor) -> Result<(), ExecuteError> {
        let mut text = if self.colors {
            let code = match color {
                ThorColor::Cyan => "\x1b[36m",
                ThorColor::Clear => "\x1b[0m",
                ThorColor::Green => "\x1b[32m",
                ThorColor::White => "\x1b[37m",
                ThorColor::Dim => {
                    return Err(ExecuteError::Raised {
                        class: "NameError".to_owned(),
                        message: "uninitialized constant Thor::Shell::Color::DIM".to_owned(),
                        partial: std::mem::take(&mut self.writes),
                    })
                }
            };
            format!("{code}{message}\x1b[0m")
        } else {
            message.to_owned()
        };
        if !(message.ends_with(' ') || message.ends_with('\t') || message.ends_with('\n')) {
            text.push('\n');
        }
        self.writes.push(OutputWrite::Stdout(text));
        Ok(())
    }

    fn done(self) -> CommandOutcome {
        CommandOutcome {
            writes: self.writes,
            exit_code: 0,
        }
    }
}

/// `CacheCommand#info`, `#stats` and `#clear`.
pub fn cache<D: CachedDefinition>(
    input: CacheInput,
    ctx: &ExecuteContext,
    cache: &DefinitionCache<D>,
) -> Result<CommandOutcome, ExecuteError> {
    if ctx.host.env.get("THOR_SHELL").is_some() {
        return Err(ExecuteError::Unsupported {
            feature: "THOR_SHELL (Thor shell selection)".to_owned(),
        });
    }
    let mut out = ThorOut::new(ctx);
    match input.action {
        CacheAction::Info => {
            let stats = cache.stats();
            out.say("Definition Cache:", ThorColor::Cyan)?;
            out.say("", ThorColor::Clear)?;
            if stats.size > 0 {
                out.say("Status: Active", ThorColor::Green)?;
            } else {
                out.say("Status: Empty", ThorColor::Dim)?;
            }
            out.say(&format!("Entries: {}", stats.size), ThorColor::White)?;
            out.say(&format!("TTL: {} seconds", stats.ttl), ThorColor::White)?;
            out.say(
                &format!("Refresh Strategy: {}", stats.refresh_strategy.as_str()),
                ThorColor::White,
            )?;
            if !stats.entries.is_empty() {
                out.say("", ThorColor::Clear)?;
                out.say("Cached Definitions:", ThorColor::White)?;
                for key in &stats.entries {
                    out.say(&format!("  - {key}"), ThorColor::Dim)?;
                }
            }
        }
        CacheAction::Stats => {
            let stats = cache.stats();
            out.say("Cache Statistics:", ThorColor::Cyan)?;
            out.say("", ThorColor::Clear)?;
            out.say(&format!("Total Entries: {}", stats.size), ThorColor::White)?;
            // `(ttl / 60).round(1)`: Integer division, then Integer#round.
            out.say(
                &format!(
                    "TTL: {} seconds ({} minutes)",
                    stats.ttl,
                    stats.ttl.div_euclid(60)
                ),
                ThorColor::White,
            )?;
            out.say(
                &format!("Refresh Strategy: {}", stats.refresh_strategy.as_str()),
                ThorColor::White,
            )?;
            out.say("", ThorColor::Clear)?;
            if !stats.entries.is_empty() {
                // `cache.get(key)` returns the definition, which has no
                // `loaded_at`: the source raises NoMethodError naming the
                // definition's class.
                return Err(ExecuteError::Unsupported {
                    feature: "cache stats with cached entries (the source raises NoMethodError on the definition)".to_owned(),
                });
            }
        }
        CacheAction::Clear => {
            if input.dry_run {
                out.say("Dry run: Would clear cache", ThorColor::Cyan)?;
                out.say(
                    &format!("Entries to be cleared: {}", cache.stats().size),
                    ThorColor::White,
                )?;
            } else {
                let count = cache.clear();
                out.say(
                    &format!("✓ Cache cleared ({count} entries)"),
                    ThorColor::Green,
                )?;
            }
        }
    }
    let _ = input.verbose;
    Ok(out.done())
}
