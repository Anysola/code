//! What the config sources produce for a process: the merged config, agents, commands, models,
//! skills and fetched instruction URLs. Loaded at start; loaded again before a step once a watched
//! file or the remote config document changed.

use std::path::Path;
use std::sync::atomic::Ordering;
use std::sync::{Arc, Weak};
use std::time::Duration;

use crate::agent::Agents;
use crate::command::Commands;
use crate::config::{Config, LoadOptions};
use crate::gateway::Gateway;
use crate::gateway::config::Remote;
use crate::instructions::{InstructionFile, RemoteStatus};
use crate::model::Model;
use crate::paths::Paths;
use crate::skill::Skill;

use super::Shared;

pub struct Loaded {
    pub config: Arc<Config>,
    pub agents: Agents,
    pub commands: Commands,
    pub models: Arc<[Model]>,
    pub skills: Vec<Skill>,
    pub references: Vec<crate::reference::Reference>,
    /// The configured `instructions` URLs, fetched when the config loaded.
    pub remote_instructions: Vec<InstructionFile>,
    /// How fetching each configured `instructions` URL went, in config order.
    pub remote_instruction_status: Vec<RemoteStatus>,
    /// How fetching each configured `skills` URL's index went, in config order.
    pub skill_status: Vec<RemoteStatus>,
    /// The formatters edits and writes run, when config `formatter` enables them.
    pub formatters: Option<Arc<crate::formatter::Formatters>>,
}

impl Loaded {
    /// Loads every source; `previous` keeps the fetched instruction URLs when they did not change.
    /// `trust`: the decisions on project documents' MCP and formatter sections.
    pub async fn load(
        paths: &Paths,
        gateway: &Gateway,
        directory: &Path,
        remote: &Remote,
        previous: Option<&Loaded>,
        trust: crate::trust::Decisions,
    ) -> Loaded {
        let options = LoadOptions {
            remote: remote.body.clone(),
            api_url: gateway.base().to_string(),
            trust,
        };
        let config = crate::config::load(paths, directory, &options);
        for warning in &config.warnings {
            crate::log::warn(warning, &[]);
        }
        let agents = Agents::load(&config, paths);
        let commands = Commands::load(&config);
        // Fetched again only when the list changed; with the same list, only the URLs that
        // failed last time are tried again.
        let (remote_instructions, remote_instruction_status) = match previous {
            Some(previous) if previous.config.instructions == config.instructions => {
                let failed: Vec<String> = previous
                    .remote_instruction_status
                    .iter()
                    .filter(|s| s.error.is_some())
                    .map(|s| s.url.clone())
                    .collect();
                if failed.is_empty() {
                    (
                        previous.remote_instructions.clone(),
                        previous.remote_instruction_status.clone(),
                    )
                } else {
                    let (files, status) = crate::instructions::fetch_remote(gateway, &failed).await;
                    let mut all_files = previous.remote_instructions.clone();
                    all_files.extend(files);
                    let all_status = previous
                        .remote_instruction_status
                        .iter()
                        .map(|s| status.iter().find(|n| n.url == s.url).unwrap_or(s).clone())
                        .collect();
                    (all_files, all_status)
                }
            }
            _ => crate::instructions::fetch_remote(gateway, &config.instructions).await,
        };
        let (mut skills, skill_status) =
            crate::skill::discover_with_status(paths, &config, directory).await;
        let policies = config.policies();
        skills.retain(|skill| policies.allows("skill.use", &skill.id));
        let models = if config.anysola_allowed("provider.use")
            && config.anysola_allowed("integration.use")
        {
            crate::model::catalog(&config).into()
        } else {
            Arc::from([])
        };
        let references =
            crate::reference::list(&config, &paths.home, directory, &paths.repos_dir());
        let formatters = crate::formatter::Formatters::from_config(&config, paths.data.join("bin"))
            .map(Arc::new);
        Loaded {
            formatters,
            config: Arc::new(config),
            agents,
            commands,
            models,
            skills,
            references,
            remote_instructions,
            remote_instruction_status,
            skill_status,
        }
    }

    fn watch_targets(&self, paths: &Paths, directory: &Path) -> Vec<crate::watch::Target> {
        let skills = crate::skill::local_roots(paths, &self.config, directory);
        crate::watch::targets(paths, &self.config, directory, &skills)
    }
}

impl Shared {
    /// A spent budget stops this session's model calls: the front end asked for budgets to
    /// stop turns ([`crate::api::Core::stop_at_budget`]) and its root's budget is used up.
    pub(crate) fn budget_spent(&self, session_id: &str) -> bool {
        self.budget_stops.load(std::sync::atomic::Ordering::Relaxed) && self.usage.spent(session_id)
    }

    /// The current config and what was built from it.
    pub fn loaded(&self) -> Arc<Loaded> {
        self.loaded
            .read()
            .unwrap_or_else(|e| e.into_inner())
            .clone()
    }

    /// Reloads every source when a watched file or the remote config changed since the last load
    /// (once the change is 100 ms old). The agent loop calls this before each step.
    pub async fn refresh(&self) {
        let applied = || self.applied.load(Ordering::SeqCst);
        if self.changes.generation() == applied() {
            return;
        }
        let _reloading = self.reloading.lock().await;
        self.changes.settled().await;
        let generation = self.changes.generation();
        if generation == applied() {
            return;
        }
        let remote = self.remote();
        let previous = self.loaded();
        let next = Loaded::load(
            &self.paths,
            &self.gateway,
            &self.directory,
            &remote,
            Some(&previous),
            crate::trust::Decisions::read(&self.db),
        )
        .await;
        self.arm_watch(&next);
        if next.references != previous.references {
            tokio::spawn(crate::reference::materialize(
                self.db.clone(),
                self.paths.repos_dir(),
                next.references.clone(),
            ));
        }
        let config = next.config.clone();
        let asked = |config: &crate::config::Config| {
            crate::hooks::undecided_with(&config.hooks, &self.db, &self.hooks.approved)
        };
        let hooks_changed = asked(&previous.config) != asked(&config);
        let waiting = |config: &crate::config::Config| -> Vec<(String, String, String)> {
            crate::trust::items_with(config, &self.db, &self.hooks.approved)
                .into_iter()
                .filter(|item| item.decision.is_none())
                .map(|item| (item.path, item.kind.as_str().to_string(), item.digest))
                .collect()
        };
        let trust_changed = waiting(&previous.config) != waiting(&config);
        *self.loaded.write().unwrap_or_else(|e| e.into_inner()) = Arc::new(next);
        self.applied.store(generation, Ordering::SeqCst);
        if hooks_changed {
            let _ = self.events.send(crate::events::Event::HooksChanged);
        }
        if trust_changed {
            let _ = self.events.send(crate::events::Event::TrustChanged);
        }
        if let Some(mcp) = self.mcp.get() {
            mcp.reconcile(config);
        }
    }

    pub(super) fn arm_watch(&self, loaded: &Loaded) {
        self.watch.arm(
            loaded.watch_targets(&self.paths, &self.directory),
            crate::watch::ignore_set(loaded.config.get("watcher")),
            self.changes.clone(),
        );
    }

    pub(crate) fn remote(&self) -> Remote {
        self.remote
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .clone()
    }

    /// Fetches `/config` again; a different body is reloaded before the next step.
    pub async fn refresh_remote(&self) {
        let next = crate::gateway::config::fetch(&self.gateway, &self.db).await;
        let mut remote = self.remote.lock().unwrap_or_else(|e| e.into_inner());
        let changed = remote.body != next.body;
        *remote = next;
        drop(remote);
        if changed {
            self.changes.mark();
        }
    }
}

/// Fetches `/config` every 10 minutes while the process holds a credential and lives. While
/// there is no document at all (the first fetch failed offline, nothing cached) it retries
/// sooner, from 15 s doubling up to the regular interval, so models appear once the network is
/// back instead of ten minutes later.
pub(super) async fn poll_remote(shared: Weak<Shared>) {
    const RETRY_SOON: Duration = Duration::from_secs(15);
    let mut backoff = RETRY_SOON;
    loop {
        let missing = match shared.upgrade() {
            Some(shared) => shared.gateway.token().is_some() && shared.remote().body.is_none(),
            None => return,
        };
        let delay = if missing {
            backoff
        } else {
            crate::gateway::config::REFRESH_EVERY
        };
        tokio::time::sleep(delay).await;
        let Some(shared) = shared.upgrade() else {
            return;
        };
        shared.refresh_remote().await;
        backoff = if missing {
            (backoff * 2).min(crate::gateway::config::REFRESH_EVERY)
        } else {
            RETRY_SOON
        };
    }
}
