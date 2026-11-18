//! `anysola hooks`: lists every configured hook with its source and whether it runs; `trust`
//! approves the current project's hooks after showing their commands, `deny` refuses them.

use std::io::{BufRead, IsTerminal, Write};

use anysola_core::config::{self, LoadOptions};
use anysola_core::hooks::{self, HookDocument, HookInfo, HookScope};
use anysola_core::paths::Paths;
use anysola_core::storage::Db;

use crate::cli::HooksCommand;

pub fn hooks(command: Option<HooksCommand>) -> i32 {
    let paths = Paths::from_env();
    let directory = match std::env::current_dir() {
        Ok(dir) => dir,
        Err(error) => return crate::fail(&error.to_string()),
    };
    let config = config::load(&paths, &directory, &LoadOptions::default());
    let db = match Db::open(&paths.database()) {
        Ok(db) => db,
        Err(error) => return crate::fail(&error.to_string()),
    };
    match command {
        None => list(
            &hooks::list(&config.hooks, &db),
            &hooks::documents(&config.hooks, &db),
        ),
        Some(HooksCommand::Trust { yes }) => trust(&db, hooks::documents(&config.hooks, &db), yes),
        Some(HooksCommand::Deny) => deny(&db, hooks::documents(&config.hooks, &db)),
    }
}

/// `  PreToolUse (Bash): ./guard.sh`
fn line(hook: &HookInfo) -> String {
    match hook
        .matcher
        .as_deref()
        .filter(|m| !m.is_empty() && *m != "*")
    {
        Some(matcher) => format!("  {} ({matcher}): {}", hook.event, hook.command),
        None => format!("  {}: {}", hook.event, hook.command),
    }
}

fn status(hook: &HookInfo, decision: Option<Option<bool>>) -> &'static str {
    match (hook.scope, decision.flatten(), hook.active) {
        (HookScope::Project, Some(true), true) => "approved",
        (HookScope::Project, Some(true), false) => "approved, turned off",
        (HookScope::Project, Some(false), _) => "refused",
        (HookScope::Project, None, _) => "not approved",
        (_, _, true) => "runs",
        (_, _, false) => "turned off",
    }
}

fn list(all: &[HookInfo], documents: &[HookDocument]) -> i32 {
    if all.is_empty() {
        println!("No hooks configured.");
        return 0;
    }
    let mut out = std::io::stdout().lock();
    let mut source: Option<&str> = None;
    for hook in all {
        if source != Some(hook.source.as_str()) {
            if source.is_some() {
                let _ = writeln!(out);
            }
            source = Some(&hook.source);
            let decision = documents
                .iter()
                .find(|d| d.path == hook.source)
                .map(|d| d.decision);
            let _ = writeln!(
                out,
                "{} ({}) · {}",
                hook.source,
                hook.scope.as_str(),
                status(hook, decision)
            );
        }
        let _ = writeln!(out, "{}", line(hook));
    }
    if documents.iter().any(|d| d.decision.is_none()) {
        let _ = writeln!(
            out,
            "\nThis project's hooks run only once approved. Run `anysola hooks trust` to review them."
        );
    }
    0
}

fn show(documents: &[&HookDocument]) {
    for document in documents {
        println!("{}", document.path);
        for hook in &document.hooks {
            println!("{}", line(hook));
        }
    }
}

fn trust(db: &Db, documents: Vec<HookDocument>, yes: bool) -> i32 {
    let pending: Vec<&HookDocument> = documents
        .iter()
        .filter(|d| d.decision != Some(true))
        .collect();
    if pending.is_empty() {
        println!("No project hooks wait for approval.");
        return 0;
    }
    show(&pending);
    if !yes {
        if !std::io::stdin().is_terminal() {
            return crate::fail("Pass --yes to approve these hooks without a terminal.");
        }
        eprint!("\nRun these commands as hooks in this project? [y/N] ");
        let _ = std::io::stderr().flush();
        let mut answer = String::new();
        let _ = std::io::stdin().lock().read_line(&mut answer);
        if !matches!(answer.trim().to_ascii_lowercase().as_str(), "y" | "yes") {
            println!("Nothing approved.");
            return 0;
        }
    }
    for document in pending {
        if let Err(error) = hooks::trust(db, &document.path, &document.digest, true) {
            return crate::fail(&error.to_string());
        }
        println!("Approved the hooks of {}", document.path);
    }
    0
}

fn deny(db: &Db, documents: Vec<HookDocument>) -> i32 {
    let pending: Vec<&HookDocument> = documents
        .iter()
        .filter(|d| d.decision != Some(false))
        .collect();
    if pending.is_empty() {
        println!("No project hooks to refuse.");
        return 0;
    }
    for document in pending {
        if let Err(error) = hooks::trust(db, &document.path, &document.digest, false) {
            return crate::fail(&error.to_string());
        }
        println!("Refused the hooks of {}", document.path);
    }
    0
}
