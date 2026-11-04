//! What the config sources produce for a process: the merged config, agents, commands, models,
//! skills and fetched instruction URLs. Loaded at start; loaded again before a step once a watched
//! file or the remote config document changed.

use std::path::Path;
use std::sync::atomic::Ordering;
use std::sync::{Arc, Weak};

use crate::agent::Agents;
use crate::command::Commands;
use crate::config::{Config, LoadOptions};
use crate::gateway::Gateway;
use crate::gateway::config::Remote;
use crate::instructions::InstructionFile;
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
    /// The configured `instructions` URLs, fetched when the config loaded.
    pub remote_instructions: Vec<InstructionFile>,
}

impl Loaded {
    /// Loads every source; `previous` keeps the fetched instruction URLs when they did not change.
    pub async fn load(
        paths: &Paths,
        gateway: &Gateway,
        directory: &Path,
        remote: &Remote,
        previous: Option<&Loaded>,
    ) -> Loaded {
        let options = LoadOptions {
            remote: remote.body.clone(),
            api_url: gateway.base().to_string(),
        };
        let config = crate::config::load(paths, directory, &options);
        for warning in &config.warnings {
            crate::log::warn(warning, &[]);
        }
        let agents = Agents::load(&config, paths);
        let commands = Commands::load(&config);
        let remote_instructions = match previous {
            Some(previous) if previous.config.instructions == config.instructions => {
                previous.remote_instructions.clone()
            }
            _ => crate::instructions::fetch_remote(gateway, &config.instructions).await,
        };
        let mut skills = crate::skill::discover(paths, &config, directory).await;
        let policies = config.policies();
        skills.retain(|skill| policies.allows("skill.use", &skill.id));
        let models = crate::model::catalog(&config).into();
        Loaded {
            config: Arc::new(config),
            agents,
            commands,
            models,
            skills,
            remote_instructions,
        }
    }

    fn watch_targets(&self, paths: &Paths, directory: &Path) -> Vec<crate::watch::Target> {
        let skills = crate::skill::local_roots(paths, &self.config, directory);
        crate::watch::targets(paths, &self.config, directory, &skills)
    }
}

impl Shared {
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
        )
        .await;
        self.arm_watch(&next);
        *self.loaded.write().unwrap_or_else(|e| e.into_inner()) = Arc::new(next);
        self.applied.store(generation, Ordering::SeqCst);
    }

    pub(super) fn arm_watch(&self, loaded: &Loaded) {
        self.watch.arm(
            loaded.watch_targets(&self.paths, &self.directory),
            crate::watch::ignore_set(loaded.config.get("watcher")),
            self.changes.clone(),
        );
    }

    pub(super) fn remote(&self) -> Remote {
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

/// Fetches `/config` every 10 minutes while the process holds a credential and lives.
pub(super) async fn poll_remote(shared: Weak<Shared>) {
    loop {
        tokio::time::sleep(crate::gateway::config::REFRESH_EVERY).await;
        let Some(shared) = shared.upgrade() else {
            return;
        };
        shared.refresh_remote().await;
    }
}
