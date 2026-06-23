use construct_config::{Agent, Config};
use construct_core::model::ModelProvider;
use construct_core::store::Store;
use construct_core::tool::Tool;
use construct_engine::events::{emit, EventKind, EventSender};
use construct_engine::orchestrator::Orchestrator;
use construct_engine::pipelines::PipelineKind;
use construct_engine::rules::known_tags;
use construct_engine::triggers::TriggerEvent;
use construct_model_ollama::OllamaProvider;
use construct_obsidian::watcher::{watch, VaultEvent};
use construct_store::SqliteStore;
use construct_tools::{WebFetch, WebSearch};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use tokio::sync::mpsc;
use tokio::sync::Mutex as AsyncMutex;

/// Resolve the `construct.db` SQLite URL relative to the config file's directory, so
/// the run database is stable regardless of the process's current working directory.
pub fn resolve_db_url(base_dir: &Path) -> String {
    let dir = std::fs::canonicalize(base_dir).unwrap_or_else(|_| base_dir.to_path_buf());
    format!("sqlite://{}", dir.join("construct.db").display())
}

/// Load an agent's system prompt, resolving a relative `system_prompt_file` against
/// the config directory. Warns (rather than silently using the generic fallback) when
/// a configured prompt file can't be read — a common "launched from the wrong dir" trap.
fn load_system_prompt(base_dir: &Path, agent: &Agent, fallback: &str) -> String {
    match &agent.system_prompt_file {
        None => fallback.to_string(),
        Some(p) => {
            let resolved = if Path::new(p).is_absolute() {
                PathBuf::from(p)
            } else {
                base_dir.join(p)
            };
            match std::fs::read_to_string(&resolved) {
                Ok(s) => s,
                Err(e) => {
                    tracing::warn!(
                        "could not read system_prompt_file {} ({e}); using default prompt",
                        resolved.display()
                    );
                    fallback.to_string()
                }
            }
        }
    }
}

/// How a trigger event should be routed to orchestrators.
#[derive(Debug, PartialEq)]
enum RouteTarget {
    /// Route to the single orchestrator whose rule matches this tag.
    Tag(String),
    /// Broadcast to all orchestrators (status decisions — any may own the run).
    Broadcast,
    /// Route to the single Inbox orchestrator (idle note processing).
    Inbox,
    /// Route to the daily orchestrator (brief events).
    Daily,
    /// No consumer wired yet (schedule lands in Plan 3).
    Unhandled,
}

fn route_key(ev: &TriggerEvent) -> RouteTarget {
    match ev {
        TriggerEvent::Tagged { tag, .. } => RouteTarget::Tag(tag.clone()),
        TriggerEvent::StatusChanged { .. } => RouteTarget::Broadcast,
        TriggerEvent::IdleNote { .. } => RouteTarget::Inbox,
        TriggerEvent::Scheduled { .. } => RouteTarget::Unhandled,
        TriggerEvent::Brief { .. } => RouteTarget::Daily,
    }
}

/// Per-note serialization map: each note path maps to its own async lock so two
/// actions on the SAME note never run concurrently while cross-note actions
/// stay parallel.
type NoteLocks = Arc<std::sync::Mutex<HashMap<String, Arc<AsyncMutex<()>>>>>;

/// Acquire (or create) the async lock for a given note path. To bound memory on a
/// long-running daemon, prune idle entries (those only the map still references) once
/// the map grows past a threshold.
fn lock_for(locks: &NoteLocks, path: &std::path::Path) -> Arc<AsyncMutex<()>> {
    let key = path.to_string_lossy().to_string();
    let mut map = locks.lock().unwrap();
    if map.len() > 256 {
        map.retain(|_, lock| Arc::strong_count(lock) > 1);
    }
    map.entry(key)
        .or_insert_with(|| Arc::new(AsyncMutex::new(())))
        .clone()
}

/// Build one orchestrator per configured rule + start the watcher; process
/// events forever. `NoteTagged` is routed to the orchestrator whose rule
/// matches the tag; `StatusChanged` is broadcast to all (any may own the run).
///
/// `quiet` suppresses the startup banner `println!`s (use under a TUI where
/// alternate-screen mode is active); the same information is also logged via
/// `tracing::info!` unconditionally.
pub async fn run_watch(
    cfg: Config,
    base_dir: PathBuf,
    events: EventSender,
    paused: Arc<std::sync::atomic::AtomicBool>,
    quiet: bool,
) -> anyhow::Result<()> {
    if cfg.rules.is_empty() {
        return Err(anyhow::anyhow!("config has no rules"));
    }

    let db_url = resolve_db_url(&base_dir);
    let store: Arc<dyn Store> = Arc::new(SqliteStore::connect(&db_url).await?);

    // Build one orchestrator per rule, keyed by the rule's trigger tag.
    use std::collections::HashMap as Map;
    let mut orchestrators: Map<String, Arc<Orchestrator>> = Map::new();
    for rule in &cfg.rules {
        let agent = cfg
            .agent(&rule.agent)
            .ok_or_else(|| anyhow::anyhow!("rule references unknown agent"))?
            .clone();
        let provider: Arc<dyn ModelProvider> =
            Arc::new(OllamaProvider::new(agent.base_url.clone()));

        let mut tools: HashMap<String, Arc<dyn Tool>> = HashMap::new();
        if agent.tools.iter().any(|t| t == "web_search") {
            if let Some(ws) = &cfg.tools.web_search {
                let key = std::env::var(&ws.api_key_env).unwrap_or_default();
                tools.insert("web_search".into(), Arc::new(WebSearch::tavily(key)));
            }
        }
        if agent.tools.iter().any(|t| t == "web_fetch") {
            tools.insert("web_fetch".into(), Arc::new(WebFetch::new()));
        }

        let fallback = format!("You are {}, a meticulous web research agent. Always answer with strict JSON: {{summary, findings[], sources[{{title,url}}]}}.", agent.name);
        let system_prompt = load_system_prompt(&base_dir, &agent, &fallback);

        let kind = PipelineKind::from_name(&rule.pipeline)
            .ok_or_else(|| anyhow::anyhow!("unknown pipeline {}", rule.pipeline))?;

        let orchestrator = Arc::new(Orchestrator {
            store: store.clone(),
            provider,
            tools,
            model: agent.model.clone(),
            agent: agent.name.clone(),
            rule: rule.pipeline.clone(),
            pipeline: kind,
            system_prompt,
            max_iterations: 8,
            done_tag: Some("theconstruct/done".into()),
            vault_path: shellexpand_tilde(&cfg.vault.path).into(),
            max_tags: cfg.actions.tag.max_tags,
            exclude_dirs: cfg.actions.organize.exclude_dirs.clone(),
            prompt_dir: Some(base_dir.join("prompts")),
            briefs_folder: cfg.briefs.as_ref().map(|b| b.folder.clone()),
        });
        orchestrators.insert(rule.match_tag.clone(), orchestrator);
    }

    // Build the Inbox orchestrator (Feature A) when [inbox] is configured.
    let inbox_orch: Option<Arc<Orchestrator>> = if let Some(inbox_cfg) = &cfg.inbox {
        // Resolve which agent runs the inbox.
        let agent = inbox_cfg
            .agent
            .as_deref()
            .and_then(|name| cfg.agent(name))
            .or_else(|| {
                cfg.rules
                    .iter()
                    .find(|r| r.pipeline == "tag" || r.pipeline == "summarize")
                    .and_then(|r| cfg.agent(&r.agent))
            })
            .or_else(|| cfg.agents.first());
        match agent {
            None => {
                tracing::warn!("[inbox] configured but no agent available; skipping inbox");
                None
            }
            Some(agent) => {
                let provider: Arc<dyn ModelProvider> =
                    Arc::new(OllamaProvider::new(agent.base_url.clone()));
                // Inbox always needs web_fetch for URL enrichment.
                let mut tools: HashMap<String, Arc<dyn Tool>> = HashMap::new();
                tools.insert("web_fetch".into(), Arc::new(WebFetch::new()));
                if agent.tools.iter().any(|t| t == "web_search") {
                    if let Some(ws) = &cfg.tools.web_search {
                        let key = std::env::var(&ws.api_key_env).unwrap_or_default();
                        tools.insert("web_search".into(), Arc::new(WebSearch::tavily(key)));
                    }
                }
                let fallback = format!(
                    "You are {}, organizing inbox notes. Always answer with strict JSON.",
                    agent.name
                );
                let system_prompt = load_system_prompt(&base_dir, agent, &fallback);
                Some(Arc::new(Orchestrator {
                    store: store.clone(),
                    provider,
                    tools,
                    model: agent.model.clone(),
                    agent: agent.name.clone(),
                    rule: "inbox".into(),
                    pipeline: PipelineKind::Inbox,
                    system_prompt,
                    max_iterations: 8,
                    done_tag: None,
                    vault_path: shellexpand_tilde(&cfg.vault.path).into(),
                    max_tags: cfg.actions.tag.max_tags,
                    exclude_dirs: cfg.actions.organize.exclude_dirs.clone(),
                    prompt_dir: Some(base_dir.join("prompts")),
                    briefs_folder: cfg.briefs.as_ref().map(|b| b.folder.clone()),
                }))
            }
        }
    } else {
        None
    };

    // Build the DailySummary orchestrator (Feature B) when [schedule] or [briefs] is configured.
    let daily_orch: Option<Arc<Orchestrator>> = if cfg.schedule.is_some() || cfg.briefs.is_some() {
        let agent = cfg
            .rules
            .iter()
            .find(|r| r.pipeline == "summarize" || r.pipeline == "tag")
            .and_then(|r| cfg.agent(&r.agent))
            .or_else(|| cfg.agents.first());
        match agent {
            None => {
                tracing::warn!(
                    "[schedule] configured but no agent available; skipping daily summary"
                );
                None
            }
            Some(agent) => {
                let provider: Arc<dyn ModelProvider> =
                    Arc::new(OllamaProvider::new(agent.base_url.clone()));
                let fallback = format!("You are {}, writing a concise daily journal recap. Always answer with strict JSON.", agent.name);
                let system_prompt = load_system_prompt(&base_dir, agent, &fallback);
                Some(Arc::new(Orchestrator {
                    store: store.clone(),
                    provider,
                    tools: HashMap::new(),
                    model: agent.model.clone(),
                    agent: agent.name.clone(),
                    rule: "daily_summary".into(),
                    pipeline: PipelineKind::DailySummary,
                    system_prompt,
                    max_iterations: 8,
                    done_tag: None,
                    vault_path: shellexpand_tilde(&cfg.vault.path).into(),
                    max_tags: cfg.actions.tag.max_tags,
                    exclude_dirs: cfg.actions.organize.exclude_dirs.clone(),
                    prompt_dir: Some(base_dir.join("prompts")),
                    briefs_folder: cfg.briefs.as_ref().map(|b| b.folder.clone()),
                }))
            }
        }
    } else {
        None
    };

    // Crash recovery: re-trigger any run left mid-flight before we start watching.
    for o in orchestrators.values() {
        if let Err(e) = o.reconcile().await {
            tracing::warn!("reconcile failed: {e}");
        }
    }
    if let Some(o) = &inbox_orch {
        if let Err(e) = o.reconcile().await {
            tracing::warn!("inbox reconcile failed: {e}");
        }
    }

    // Per-note serialization: two actions on the SAME note never run
    // concurrently. Each note path maps to its own async lock; the spawned
    // handler acquires it before running. Cross-note actions stay parallel.
    // Initialized here (before any spawned tasks) so both the scheduler task
    // and the brief handler can share the same lock map.
    let note_locks: NoteLocks = Arc::new(std::sync::Mutex::new(HashMap::new()));

    // The watch loop runs on a single `TriggerEvent` channel end-to-end. The
    // vault watcher sends `VaultEvent`s on its own channel; a forwarder task
    // maps those into `TriggerEvent`s. The idle poller sends `TriggerEvent`s
    // (IdleNote) directly on the same `tx`.
    let (tx, mut rx) = mpsc::unbounded_channel::<TriggerEvent>();
    let (tx_vault, mut rx_vault) = mpsc::unbounded_channel::<VaultEvent>();
    let vault = shellexpand_tilde(&cfg.vault.path);
    let briefs_dir: Option<PathBuf> = cfg
        .briefs
        .as_ref()
        .map(|b| PathBuf::from(shellexpand_tilde(&cfg.vault.path)).join(&b.folder));
    let _debouncer = watch(
        vault.into(),
        known_tags(&cfg),
        "construct_status".into(),
        briefs_dir,
        tx_vault,
    );

    // Forwarder: map watcher VaultEvents into TriggerEvents on the shared channel.
    // Wrapped in catch_unwind so a panic logs and the loop survives — if this task
    // died silently the watcher would stop receiving file events while looking healthy.
    {
        let tx_fwd = tx.clone();
        tokio::spawn(async move {
            use futures::FutureExt;
            loop {
                let step = std::panic::AssertUnwindSafe(async {
                    match rx_vault.recv().await {
                        Some(ev) => {
                            let _ = tx_fwd.send(ev.into());
                            true
                        }
                        None => false, // watcher channel closed
                    }
                })
                .catch_unwind()
                .await;
                match step {
                    Ok(true) => {}
                    Ok(false) => break,
                    Err(_) => tracing::error!("event forwarder panicked; continuing"),
                }
            }
        });
    }

    // Idle poller: scan top-level Inbox files on an interval; emit IdleNote events.
    if let Some(inbox_cfg) = cfg.inbox.clone() {
        let tx_idle = tx.clone();
        let vault_root: std::path::PathBuf = shellexpand_tilde(&cfg.vault.path).into();
        let inbox_dir = vault_root.join(&inbox_cfg.folder);
        let journal_folder = cfg
            .journal
            .as_ref()
            .map(|j| j.folder.clone())
            .unwrap_or_else(|| "journal".to_string());
        let managed = cfg.vault.managed_folder.clone();
        let idle_minutes = inbox_cfg.idle_minutes;
        tokio::spawn(async move {
            use construct_core::clock::{Clock, SystemClock};
            use futures::FutureExt;
            let clock = SystemClock;
            let mut ticker = tokio::time::interval(std::time::Duration::from_secs(60));
            // After the machine wakes from sleep, don't fire a backlog of missed ticks
            // all at once — one tick per period is enough.
            ticker.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
            loop {
                ticker.tick().await;
                let step = std::panic::AssertUnwindSafe(async {
                    let now = clock.now_local();
                    let ready = construct_engine::triggers::idle::scan_inbox(
                        &inbox_dir,
                        &vault_root,
                        &journal_folder,
                        managed.as_deref(),
                        now,
                        idle_minutes,
                    );
                    for path in ready {
                        let _ = tx_idle.send(TriggerEvent::IdleNote { path });
                    }
                })
                .catch_unwind()
                .await;
                if step.is_err() {
                    tracing::error!("idle poller iteration panicked; continuing");
                }
            }
        });
    }

    // Daily-summary scheduler: poll every 60s; fire run_daily_summary when due
    // (with catch-up via the persisted last_run). All times local.
    if let (Some(sched_cfg), Some(daily)) = (cfg.schedule.clone(), daily_orch.clone()) {
        let store_sched = store.clone();
        let cfg_journal_folder = cfg
            .journal
            .as_ref()
            .map(|j| j.folder.clone())
            .unwrap_or_else(|| "journal".to_string());
        let vault_path_sched: std::path::PathBuf = shellexpand_tilde(&cfg.vault.path).into();
        let note_locks_sched = note_locks.clone();
        let events_sched = events.clone();
        tokio::spawn(async move {
            use construct_core::clock::{Clock, SystemClock};
            use construct_engine::pipelines::daily::journal_day_path;
            use construct_engine::triggers::schedule;
            use futures::FutureExt;
            let clock = SystemClock;
            let Some(daily_time) = schedule::parse_hhmm(&sched_cfg.daily_time) else {
                tracing::warn!(
                    "invalid schedule.daily_time '{}'; daily summary disabled",
                    sched_cfg.daily_time
                );
                return;
            };
            let journal_folder = cfg_journal_folder; // captured String
            let mut ticker = tokio::time::interval(std::time::Duration::from_secs(60));
            // Don't replay a burst of missed ticks after sleep; `due()` + last_run
            // already handle catch-up on the next single tick.
            ticker.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
            loop {
                ticker.tick().await;
                let step = std::panic::AssertUnwindSafe(async {
                    let now = clock.now_local();
                    let last_run = store_sched
                        .get_last_run("daily_summary")
                        .await
                        .ok()
                        .flatten()
                        .and_then(|s| chrono::DateTime::parse_from_rfc3339(&s).ok())
                        .map(|dt| dt.with_timezone(&chrono::Local));
                    if schedule::due(last_run, daily_time, now) {
                        let date = now.date_naive();
                        let day_note_path =
                            vault_path_sched.join(journal_day_path(&journal_folder, date));
                        let lock = lock_for(&note_locks_sched, &day_note_path);
                        let _guard = lock.lock().await;
                        emit(
                            &events_sched,
                            construct_engine::events::EventKind::Daily,
                            format!("daily summary for {date}"),
                        );
                        match daily.run_daily_summary(date, &journal_folder).await {
                            Ok(()) => {
                                // Record the COMPLETION time (not the pre-run `now`): the
                                // LLM call can span minutes, and a stale timestamp can fall
                                // before today's firing instant and double-fire next tick.
                                let completed = clock.now_local();
                                let _ = store_sched
                                    .set_last_run("daily_summary", &completed.to_rfc3339())
                                    .await;
                                emit(
                                    &events_sched,
                                    construct_engine::events::EventKind::Daily,
                                    "daily summary done",
                                );
                            }
                            Err(e) => {
                                tracing::error!("daily summary failed: {e}");
                                emit(
                                    &events_sched,
                                    construct_engine::events::EventKind::Daily,
                                    format!("daily summary: {e}"),
                                );
                            }
                        }
                    }
                })
                .catch_unwind()
                .await;
                if step.is_err() {
                    tracing::error!("daily scheduler iteration panicked; continuing");
                }
            }
        });
    }

    let journal_folder_for_briefs = cfg
        .journal
        .as_ref()
        .map(|j| j.folder.clone())
        .unwrap_or_else(|| "journal".to_string());
    let vault_path_for_briefs: std::path::PathBuf = shellexpand_tilde(&cfg.vault.path).into();

    let tags: Vec<String> = orchestrators.keys().cloned().collect();
    // Startup banner: surface what's actually active so a misconfigured run is obvious.
    // Under dashboard mode (quiet=true) the alternate screen is already live, so we
    // log via tracing only — printing here would race the TUI and corrupt the display.
    let db_path = db_url.strip_prefix("sqlite://").unwrap_or(&db_url);
    let vault_display = shellexpand_tilde(&cfg.vault.path);
    let tags_display = tags
        .iter()
        .map(|t| format!("#{t}"))
        .collect::<Vec<_>>()
        .join(" or ");
    tracing::info!(
        "The Construct is watching. vault={vault_display} db={db_path} tags={tags_display}"
    );
    if !quiet {
        println!("The Construct is watching.");
        println!("  vault:   {vault_display}");
        println!("  db:      {db_path}");
        println!("  tags:    {tags_display}");
        if let Some(ib) = &cfg.inbox {
            println!("  inbox:   {} (idle {}m)", ib.folder, ib.idle_minutes);
        }
        if let Some(sc) = &cfg.schedule {
            let jf = cfg
                .journal
                .as_ref()
                .map(|j| j.folder.as_str())
                .unwrap_or("journal");
            println!("  daily:   {} → {}/", sc.daily_time, jf);
        }
        if let Some(b) = &cfg.briefs {
            println!("  briefs:  {}/ (event-driven)", b.folder);
        }
        if inbox_orch.is_none() && cfg.inbox.is_some() {
            println!("  (inbox configured but disabled — no agent available)");
        }
        if daily_orch.is_none() && (cfg.schedule.is_some() || cfg.briefs.is_some()) {
            println!("  (daily/briefs configured but disabled — no agent available)");
        }
    }
    emit(&events, EventKind::Info, "watching started");

    loop {
        let event = tokio::select! {
            maybe = rx.recv() => match maybe {
                Some(ev) => ev,
                None => break, // all senders dropped
            },
            _ = tokio::signal::ctrl_c() => {
                println!("\nShutting down. In-flight work resumes via reconcile on next start.");
                break;
            }
        };
        if paused.load(std::sync::atomic::Ordering::Relaxed) {
            emit(
                &events,
                EventKind::Info,
                "paused — event skipped (idle notes re-trigger automatically)",
            );
            continue;
        }
        match route_key(&event) {
            RouteTarget::Tag(tag) => {
                if let (Some(o), TriggerEvent::Tagged { path, tag: t }) =
                    (orchestrators.get(&tag), event.clone())
                {
                    let o = o.clone();
                    // Mark deterministic handlers distinctly in the activity log —
                    // "handled without a model" is the product's whole claim.
                    let kind = if o.pipeline.is_deterministic() {
                        EventKind::Deterministic
                    } else {
                        EventKind::Run
                    };
                    let lock = lock_for(&note_locks, &path);
                    let ev_tx = events.clone();
                    tokio::spawn(async move {
                        let _guard = lock.lock().await;
                        let name = path
                            .file_name()
                            .map(|n| n.to_string_lossy().to_string())
                            .unwrap_or_default();
                        emit(&ev_tx, kind, format!("{name} → #{t}"));
                        match o.handle(VaultEvent::NoteTagged { path, tag: t }).await {
                            Ok(()) => {
                                let done = if kind == EventKind::Deterministic {
                                    format!("{name} done (no model call)")
                                } else {
                                    format!("{name} done")
                                };
                                emit(&ev_tx, kind, done)
                            }
                            Err(e) => {
                                tracing::error!("handler error: {e}");
                                emit(&ev_tx, EventKind::Error, format!("{name}: {e}"));
                            }
                        }
                    });
                }
            }
            RouteTarget::Broadcast => {
                if let TriggerEvent::StatusChanged { path, status } = event {
                    for o in orchestrators.values() {
                        let o = o.clone();
                        let (p, s) = (path.clone(), status.clone());
                        tokio::spawn(async move {
                            if let Err(e) = o
                                .handle(VaultEvent::StatusChanged { path: p, status: s })
                                .await
                            {
                                tracing::error!("decision handler error: {e}");
                            }
                        });
                    }
                }
            }
            RouteTarget::Inbox => {
                if let (Some(o), TriggerEvent::IdleNote { path }) = (inbox_orch.as_ref(), event) {
                    let o = o.clone();
                    let lock = lock_for(&note_locks, &path);
                    let ev_tx = events.clone();
                    tokio::spawn(async move {
                        let _guard = lock.lock().await;
                        let name = path
                            .file_name()
                            .map(|n| n.to_string_lossy().to_string())
                            .unwrap_or_default();
                        emit(&ev_tx, EventKind::Inbox, format!("{name} processing"));
                        match o.handle_idle(&path).await {
                            Ok(()) => emit(&ev_tx, EventKind::Inbox, format!("{name} done")),
                            Err(e) => {
                                tracing::error!("inbox handler error: {e}");
                                emit(&ev_tx, EventKind::Error, format!("{name}: {e}"));
                            }
                        }
                    });
                }
            }
            RouteTarget::Daily => {
                if let (Some(o), TriggerEvent::Brief { path, date }) = (daily_orch.as_ref(), event)
                {
                    let o = o.clone();
                    let journal_folder = journal_folder_for_briefs.clone();
                    // Lock on the DAY-NOTE path (same file run_brief and run_daily_summary
                    // both write), not on the brief file path, so scheduler and brief events
                    // serialize correctly on the same target note.
                    let day_note_path = vault_path_for_briefs.join(
                        construct_engine::pipelines::daily::journal_day_path(&journal_folder, date),
                    );
                    let lock = lock_for(&note_locks, &day_note_path);
                    let ev_tx = events.clone();
                    tokio::spawn(async move {
                        let _guard = lock.lock().await;
                        let name = path
                            .file_name()
                            .map(|n| n.to_string_lossy().to_string())
                            .unwrap_or_default();
                        emit(&ev_tx, EventKind::Brief, format!("{name} processing"));
                        match o.run_brief(&path, date, &journal_folder).await {
                            Ok(()) => emit(&ev_tx, EventKind::Brief, format!("{name} done")),
                            Err(e) => {
                                tracing::error!("brief handler error: {e}");
                                emit(&ev_tx, EventKind::Error, format!("{name}: {e}"));
                            }
                        }
                    });
                }
            }
            RouteTarget::Unhandled => {
                tracing::debug!("unhandled trigger event: {event:?}");
            }
        }
    }
    Ok(())
}

/// Public wrapper so commands.rs can resolve the display/vault path identically.
pub fn expand_vault_path(p: &str) -> String {
    shellexpand_tilde(p)
}

/// Minimal `~` expansion (avoids an extra dependency).
fn shellexpand_tilde(p: &str) -> String {
    if let Some(rest) = p.strip_prefix("~/") {
        if let Ok(home) = std::env::var("HOME") {
            return format!("{home}/{rest}");
        }
    }
    p.to_string()
}

#[cfg(test)]
mod tests {
    use super::*;
    use construct_engine::triggers::TriggerEvent;
    use std::path::PathBuf;

    #[test]
    fn expands_tilde() {
        std::env::set_var("HOME", "/home/x");
        assert_eq!(shellexpand_tilde("~/vault"), "/home/x/vault");
        assert_eq!(shellexpand_tilde("/abs"), "/abs");
    }

    #[test]
    fn classify_route_tagged_goes_to_matching_orchestrator_key() {
        // route_key returns Some(tag) for Tagged, None for broadcast/unhandled.
        let ev = TriggerEvent::Tagged {
            path: PathBuf::from("/v/a.md"),
            tag: "t/x".into(),
        };
        assert_eq!(route_key(&ev), RouteTarget::Tag("t/x".to_string()));
        let ev = TriggerEvent::StatusChanged {
            path: PathBuf::from("/v/a.md"),
            status: "accepted".into(),
        };
        assert_eq!(route_key(&ev), RouteTarget::Broadcast);
        // Scheduled has no consumer yet → Unhandled.
        assert_eq!(
            route_key(&TriggerEvent::Scheduled {
                job: "daily_summary".into()
            }),
            RouteTarget::Unhandled
        );
    }

    #[test]
    fn route_key_for_idle_note_is_inbox() {
        let ev = TriggerEvent::IdleNote {
            path: PathBuf::from("/v/Inbox/a.md"),
        };
        assert_eq!(route_key(&ev), RouteTarget::Inbox);
    }

    #[test]
    fn route_key_for_brief_is_daily() {
        let ev = TriggerEvent::Brief {
            path: PathBuf::from("/v/AI/DailyBriefs/2026-06-09.md"),
            date: chrono::NaiveDate::from_ymd_opt(2026, 6, 9).unwrap(),
        };
        assert_eq!(route_key(&ev), RouteTarget::Daily);
    }
}
