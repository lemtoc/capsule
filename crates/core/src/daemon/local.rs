//! Local prompt engine for `capsule connect --local`.
//!
//! This keeps the daemon request pipeline but hosts it inside the shell
//! coprocess, avoiding the long-lived socket daemon.

use std::{path::PathBuf, sync::Arc};

use capsule_protocol::{Message, Request};
use tokio::{
    sync::{Mutex, mpsc},
    task::JoinSet,
};

use super::{ConfigSource, DaemonError, ReloadableConfig, SharedState, request, stats};
use crate::module::GitProvider;

/// Prompt engine hosted inside a single shell coprocess.
///
/// The local engine has the same fast/slow/cache/update behavior as the
/// daemon server, but its state lives only as long as the `capsule connect
/// --local` process.
pub struct LocalEngine<G> {
    engine: request::PromptEngine<G>,
    worker_tasks: Arc<Mutex<JoinSet<()>>>,
}

impl<G: GitProvider + Clone + Send + Sync + 'static> LocalEngine<G> {
    /// Creates a local prompt engine.
    #[must_use]
    pub fn new(home_dir: PathBuf, git_provider: G, config_source: ConfigSource) -> Self {
        let worker_tasks = Arc::new(Mutex::new(JoinSet::new()));
        let engine = request::PromptEngine::new(request::PromptEngineParts {
            state: Arc::new(Mutex::new(SharedState::new())),
            home_dir: Arc::new(home_dir),
            git_provider,
            config: Arc::new(Mutex::new(ReloadableConfig::new(
                config_source.config,
                config_source.path,
            ))),
            stats: Arc::new(stats::DaemonStats::new()),
            worker_tasks: Arc::clone(&worker_tasks),
        });
        Self {
            engine,
            worker_tasks,
        }
    }

    /// Returns shell environment variables required by configured modules.
    pub async fn env_var_names(&self) -> Vec<String> {
        self.engine.env_var_names().await
    }

    /// Handles one prompt request and sends render/update messages to `responses`.
    ///
    /// # Errors
    ///
    /// Returns [`DaemonError`] when request handling fails.
    pub async fn handle_request(
        &self,
        request: Request,
        responses: mpsc::Sender<Message>,
        update_tasks: &mut JoinSet<()>,
    ) -> Result<(), DaemonError> {
        self.engine
            .handle_request(request, responses, update_tasks)
            .await
    }

    /// Aborts background slow-module workers.
    pub async fn shutdown(&self) {
        let mut tasks = self.worker_tasks.lock().await;
        tasks.abort_all();
        while tasks.join_next().await.is_some() {}
    }
}
