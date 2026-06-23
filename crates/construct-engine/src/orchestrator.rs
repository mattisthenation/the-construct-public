use crate::agent_loop::{run_loop, LoopConfig};
use crate::gate;
use crate::pipeline::{apply_accept, apply_claim, apply_reject, apply_write_back};
use construct_core::model::{ChatMessage, ModelProvider};
use construct_core::store::{RunRecord, Store};
use construct_core::tool::Tool;
use construct_core::types::{RunId, RunStatus};
use construct_obsidian::frontmatter::Note;
use construct_obsidian::watcher::VaultEvent;
use std::collections::HashMap;
use std::path::Path;
use std::sync::Arc;

/// Write file contents atomically: write a sibling temp file then rename over the
/// target, so a crash mid-write can never truncate/corrupt an existing note.
fn write_atomic(path: &Path, contents: &str) -> std::io::Result<()> {
    match path.file_name() {
        Some(name) => {
            let tmp = path.with_file_name(format!(".{}.tmp", name.to_string_lossy()));
            std::fs::write(&tmp, contents)?;
            std::fs::rename(&tmp, path)
        }
        None => std::fs::write(path, contents), // pathological path; best-effort
    }
}

pub struct Orchestrator {
    pub store: Arc<dyn Store>,
    pub provider: Arc<dyn ModelProvider>,
    pub tools: HashMap<String, Arc<dyn Tool>>,
    pub model: String,
    pub agent: String,
    pub rule: String,
    pub pipeline: crate::pipelines::PipelineKind,
    pub system_prompt: String,
    pub max_iterations: usize,
    pub done_tag: Option<String>,
    pub vault_path: std::path::PathBuf,
    pub max_tags: usize,
    pub exclude_dirs: Vec<String>,
    /// Directory holding prompt templates (e.g. ~/.theconstruct/prompts);
    /// None → embedded defaults.
    pub prompt_dir: Option<std::path::PathBuf>,
    /// Vault-relative Daily Briefs folder ([briefs].folder), None when off.
    pub briefs_folder: Option<String>,
    /// Deterministic file-this routing rules ([actions.file_this].rules).
    pub file_rules: Vec<construct_config::FileRule>,
}

impl Orchestrator {
    /// Handle one classified vault event end-to-end.
    pub async fn handle(&self, event: VaultEvent) -> anyhow::Result<()> {
        match event {
            VaultEvent::NoteTagged { path, .. } => self.handle_tagged(&path).await,
            VaultEvent::StatusChanged { path, status } => {
                self.handle_decision(&path, &status).await
            }
            // Brief events are routed via TriggerEvent::Brief → Unhandled in Task 8;
            // the orchestrator never receives them directly.
            VaultEvent::BriefChanged { .. } => Ok(()),
        }
    }

    /// Crash recovery: re-trigger any run left mid-flight (queued/running/researching)
    /// after a restart. Runs parked in `review` are left alone (awaiting human).
    /// Only runs owned by this orchestrator's rule are reconciled, so a restart
    /// does not re-dispatch another pipeline's stale run through the wrong pipeline.
    /// Call once on startup before beginning to watch.
    pub async fn reconcile(&self) -> anyhow::Result<()> {
        // Daily-summary runs are scheduled, not note-claim based; they recover on the
        // next schedule fire (with catch-up), never via note re-claim. Skip them here
        // so reconcile never stamps a claim onto a journal day note.
        if self.pipeline == crate::pipelines::PipelineKind::DailySummary {
            return Ok(());
        }
        for status in [
            RunStatus::Queued,
            RunStatus::Running,
            RunStatus::Researching,
        ] {
            for run in self.store.runs_with_status(status).await? {
                // Only reconcile runs owned by THIS orchestrator's pipeline/rule.
                if run.rule != self.rule {
                    continue;
                }
                self.store
                    .update_status(
                        &run.id,
                        RunStatus::Error,
                        Some("reconciled after restart".into()),
                    )
                    .await?;
                self.store
                    .append_event(&run.id, "reconcile", "restarted", serde_json::json!({}))
                    .await?;
                self.handle(VaultEvent::NoteTagged {
                    path: run.note_path.clone().into(),
                    tag: String::new(),
                })
                .await?;
            }
        }
        Ok(())
    }

    async fn handle_tagged(&self, path: &Path) -> anyhow::Result<()> {
        let note_path = path.to_string_lossy().to_string();

        // Idempotency: skip if a non-terminal run already exists for this note.
        if let Some(existing) = self.store.run_for_note(&note_path).await? {
            if !matches!(
                existing.status,
                RunStatus::Done | RunStatus::Rejected | RunStatus::Error
            ) {
                return Ok(());
            }
        }

        let run_id = RunId::new();
        let original = std::fs::read_to_string(path)?;

        // 1. claim
        self.store
            .create_run(&RunRecord {
                id: run_id.clone(),
                rule: self.rule.clone(),
                agent: self.agent.clone(),
                note_path: note_path.clone(),
                status: RunStatus::Queued,
                error: None,
            })
            .await?;
        write_atomic(path, &apply_claim(&original, &run_id.0))?;
        self.store
            .append_event(&run_id, "claim", "queued", serde_json::json!({}))
            .await?;

        // Dispatch by pipeline.
        use crate::pipelines::PipelineKind;
        match self.pipeline {
            PipelineKind::RemindMe => self.run_remind(&run_id, path, &original).await,
            PipelineKind::FileThis => self.run_file_this(&run_id, path, &original).await,
            PipelineKind::Research => self.run_research(&run_id, path, &original).await,
            PipelineKind::Summarize => self.run_summarize(&run_id, path, &original).await,
            PipelineKind::Tag => self.run_tag(&run_id, path, &original).await,
            PipelineKind::Organize => self.run_organize(&run_id, path, &original).await,
            PipelineKind::Inbox => self.run_inbox(&run_id, path, &original).await,
            PipelineKind::DailySummary => {
                self.fail(
                    &run_id,
                    path,
                    "daily summary is scheduled, not tag-triggered",
                )
                .await
            }
        }
    }

    /// remind-me pipeline: fully deterministic. Parse the reminder, write it back,
    /// done. **No model is ever invoked** — `self.provider` is untouched here. This
    /// is the handler that proves the deterministic-first thesis.
    async fn run_remind(&self, run_id: &RunId, path: &Path, original: &str) -> anyhow::Result<()> {
        use construct_core::clock::{Clock, SystemClock};
        self.store
            .update_status(run_id, RunStatus::Running, None)
            .await?;
        let now = SystemClock.now_local();
        let note = Note::parse(original);
        let Some(reminder) = crate::pipelines::remind::parse_reminder(&note.body, now) else {
            return self
                .fail(
                    run_id,
                    path,
                    "no \"remind me to …\" instruction found in note",
                )
                .await;
        };
        let current = std::fs::read_to_string(path)?;
        let applied = crate::pipelines::remind::apply_reminder(
            &current,
            &reminder,
            now.date_naive(),
            self.done_tag.as_deref(),
        );
        write_atomic(path, &applied)?;
        self.store
            .update_status(run_id, RunStatus::Done, None)
            .await?;
        self.store
            .append_event(
                run_id,
                "remind",
                "done",
                serde_json::json!({
                    "deterministic": true,
                    "task": reminder.task,
                    "due": reminder.due.map(|d| d.to_rfc3339()),
                }),
            )
            .await?;
        Ok(())
    }

    /// file-this pipeline: Priori decides. If a deterministic keyword rule matches,
    /// propose that folder with NO model call; otherwise escalate to the organize
    /// (model) flow. Either way the move is applied only on human accept.
    async fn run_file_this(
        &self,
        run_id: &RunId,
        path: &Path,
        original: &str,
    ) -> anyhow::Result<()> {
        self.store
            .update_status(run_id, RunStatus::Running, None)
            .await?;
        let note = Note::parse(original);
        match crate::priori::judge(
            crate::pipelines::PipelineKind::FileThis,
            &note.body,
            &self.file_rules,
        ) {
            crate::priori::Decision::Deterministic(reason) => {
                // Safe: a Deterministic verdict for FileThis implies a classify match.
                let folder = crate::pipelines::file_this::classify(&note.body, &self.file_rules)
                    .map(|(f, _)| f.to_string())
                    .unwrap_or_default();
                let current = std::fs::read_to_string(path)?;
                write_atomic(
                    path,
                    &crate::pipelines::organize::apply_propose(&current, &folder, &reason),
                )?;
                self.store
                    .update_status(run_id, RunStatus::Review, None)
                    .await?;
                self.store
                    .append_event(
                        run_id,
                        "file-this",
                        "review",
                        serde_json::json!({"deterministic": true, "destination": folder, "reason": reason}),
                    )
                    .await?;
                Ok(())
            }
            crate::priori::Decision::Escalate(_) => {
                // No rule matched — fall back to the model-driven organize flow.
                self.run_organize(run_id, path, original).await
            }
        }
    }

    /// Organize pipeline: agent loop → gate → propose a move for human review.
    /// No file is moved here; the move happens on accept in `handle_decision`.
    async fn run_organize(
        &self,
        run_id: &RunId,
        path: &Path,
        original: &str,
    ) -> anyhow::Result<()> {
        self.store
            .update_status(run_id, RunStatus::Running, None)
            .await?;
        let note = Note::parse(original);
        let folders = construct_obsidian::vault::list_folders(&self.vault_path, &self.exclude_dirs);
        let user_prompt = format!(
            "Pick the single best destination folder for this note from this list ONLY: {}. \
             Return STRICT JSON only: {{\"destination\": string, \"reason\": string}}.\n\nNOTE:\n{}",
            folders.join(", "),
            note.body
        );
        let messages = vec![
            ChatMessage::system(&self.system_prompt),
            ChatMessage::user(user_prompt),
        ];
        let out = match run_loop(
            self.provider.as_ref(),
            &self.tools,
            messages,
            &LoopConfig {
                model: self.model.clone(),
                max_iterations: self.max_iterations,
            },
        )
        .await
        {
            Ok(r) => r,
            Err(e) => return self.fail(run_id, path, &e.to_string()).await,
        };
        let proposal = match crate::gate::validate_organize(&out.content, &folders) {
            Ok(p) => p,
            Err(e) => return self.fail(run_id, path, &e.to_string()).await,
        };
        let current = std::fs::read_to_string(path)?;
        write_atomic(
            path,
            &crate::pipelines::organize::apply_propose(
                &current,
                &proposal.destination,
                &proposal.reason,
            ),
        )?;
        self.store
            .update_status(run_id, RunStatus::Review, None)
            .await?;
        self.store
            .append_event(
                run_id,
                "organize",
                "review",
                serde_json::json!({"destination": proposal.destination}),
            )
            .await?;
        Ok(())
    }

    /// Summarize pipeline: agent loop → gate → auto-apply summary block (no human review).
    async fn run_summarize(
        &self,
        run_id: &RunId,
        path: &Path,
        original: &str,
    ) -> anyhow::Result<()> {
        self.store
            .update_status(run_id, RunStatus::Running, None)
            .await?;
        let note = Note::parse(original);
        let user_prompt = format!(
            "Summarize the following note. Return STRICT JSON only: \
             {{\"tldr\": string, \"action_items\": [string]}}.\n\n{}",
            note.body
        );
        let messages = vec![
            ChatMessage::system(&self.system_prompt),
            ChatMessage::user(user_prompt),
        ];
        let out = match run_loop(
            self.provider.as_ref(),
            &self.tools,
            messages,
            &LoopConfig {
                model: self.model.clone(),
                max_iterations: self.max_iterations,
            },
        )
        .await
        {
            Ok(r) => r,
            Err(e) => return self.fail(run_id, path, &e.to_string()).await,
        };
        let summary = match crate::gate::validate_summary(&out.content) {
            Ok(s) => s,
            Err(e) => return self.fail(run_id, path, &e.to_string()).await,
        };
        let current = std::fs::read_to_string(path)?;
        let applied = crate::pipelines::summarize::apply_summary(
            &current,
            &summary,
            self.done_tag.as_deref(),
        );
        write_atomic(path, &applied)?;
        self.store
            .update_status(run_id, RunStatus::Done, None)
            .await?;
        self.store
            .append_event(run_id, "summarize", "done", serde_json::json!({}))
            .await?;
        Ok(())
    }

    /// Research pipeline: agent loop → gate → write back for human review.
    async fn run_research(
        &self,
        run_id: &RunId,
        path: &Path,
        original: &str,
    ) -> anyhow::Result<()> {
        // 2. research (agent) — status researching
        self.store
            .update_status(run_id, RunStatus::Researching, None)
            .await?;
        let note = Note::parse(original);
        let user_prompt = format!("Title/topic and note body follow. Research it on the web and return STRICT JSON matching {{summary, findings[], sources[{{title,url}}]}}.\n\n{}", note.body);
        let messages = vec![
            ChatMessage::system(&self.system_prompt),
            ChatMessage::user(user_prompt),
        ];
        let out = match run_loop(
            self.provider.as_ref(),
            &self.tools,
            messages,
            &LoopConfig {
                model: self.model.clone(),
                max_iterations: self.max_iterations,
            },
        )
        .await
        {
            Ok(r) => r,
            Err(e) => return self.fail(run_id, path, &e.to_string()).await,
        };

        // 3. gate (shape + source grounding against gathered evidence)
        let result = match gate::validate(&out.content, &out.evidence) {
            Ok(r) => r,
            Err(e) => return self.fail(run_id, path, &e.to_string()).await,
        };

        // 4. write_back — status review (re-read current file to preserve edits)
        let current = std::fs::read_to_string(path)?;
        write_atomic(path, &apply_write_back(&current, &result))?;
        self.store
            .update_status(run_id, RunStatus::Review, None)
            .await?;
        self.store
            .append_event(
                run_id,
                "write_back",
                "review",
                serde_json::json!({"sources": result.sources.len()}),
            )
            .await?;
        // 5. await_decision: simply return; the watcher resumes us on StatusChanged.
        Ok(())
    }

    async fn handle_decision(&self, path: &Path, status: &str) -> anyhow::Result<()> {
        let note_path = path.to_string_lossy().to_string();
        let Some(run) = self.store.run_for_note(&note_path).await? else {
            return Ok(());
        };
        // Ownership guard: StatusChanged is broadcast to every orchestrator, so a
        // non-owning orchestrator must not act on a run belonging to another rule.
        if run.rule != self.rule {
            return Ok(());
        }
        if run.status != RunStatus::Review {
            return Ok(());
        }
        let current = std::fs::read_to_string(path)?;
        match status {
            "accepted" => {
                if matches!(
                    self.pipeline,
                    crate::pipelines::PipelineKind::Organize
                        | crate::pipelines::PipelineKind::FileThis
                ) {
                    let dest = crate::pipelines::organize::proposed_destination(&current)
                        .ok_or_else(|| anyhow::anyhow!("no proposed_move on note"))?;
                    let updated = crate::pipelines::organize::apply_accept(&current, &note_path);
                    let target = self.collision_free_target(&dest, path);
                    if let Some(parent) = target.parent() {
                        std::fs::create_dir_all(parent)?;
                    }
                    // Write the finalized (status=done) content to the source first, then
                    // rename into place. A crash after the write leaves a recoverable
                    // done-stamped note at the original path; after the rename it's
                    // correctly at the target — never a truncated/empty file.
                    write_atomic(path, &updated)?;
                    std::fs::rename(path, &target)?;
                    self.store
                        .update_status(&run.id, RunStatus::Done, None)
                        .await?;
                    self.store
                        .append_event(
                            &run.id,
                            "finalize",
                            "moved",
                            serde_json::json!({"to": target.to_string_lossy()}),
                        )
                        .await?;
                } else {
                    write_atomic(path, &apply_accept(&current, self.done_tag.as_deref()))?;
                    self.store
                        .update_status(&run.id, RunStatus::Done, None)
                        .await?;
                    self.store
                        .append_event(&run.id, "finalize", "done", serde_json::json!({}))
                        .await?;
                }
            }
            "rejected" => {
                if matches!(
                    self.pipeline,
                    crate::pipelines::PipelineKind::Organize
                        | crate::pipelines::PipelineKind::FileThis
                ) {
                    write_atomic(path, &crate::pipelines::organize::apply_reject(&current))?;
                } else {
                    write_atomic(path, &apply_reject(&current))?;
                }
                self.store
                    .update_status(&run.id, RunStatus::Rejected, None)
                    .await?;
                self.store
                    .append_event(&run.id, "finalize", "rejected", serde_json::json!({}))
                    .await?;
            }
            _ => {}
        }
        Ok(())
    }

    /// Entry point for the idle (Inbox) trigger. Reuses the same claim +
    /// idempotency path as a tag trigger, then dispatches the Inbox pipeline.
    pub async fn handle_idle(&self, path: &Path) -> anyhow::Result<()> {
        self.handle_tagged(path).await
    }

    /// Compute a collision-free absolute target path for moving `path` into the
    /// vault-relative folder `dest`: `<vault>/<dest>/<name>.md`, appending
    /// ` (1)`, ` (2)`, … to the stem until the path is free.
    fn collision_free_target(&self, dest: &str, path: &Path) -> std::path::PathBuf {
        let file_name = path
            .file_name()
            .map(|n| n.to_string_lossy().to_string())
            .unwrap_or_else(|| "note.md".to_string());
        let mut target = self.vault_path.join(dest).join(&file_name);
        let mut n = 1;
        while target.exists() {
            let stem = path
                .file_stem()
                .map(|s| s.to_string_lossy().to_string())
                .unwrap_or_else(|| "note".to_string());
            target = self.vault_path.join(dest).join(format!("{stem} ({n}).md"));
            n += 1;
        }
        target
    }

    /// Inbox pipeline: enrich from URLs → summarize → tag → move-or-recommend → log.
    /// Writes the note once at the end. Auto-moves only into an existing folder;
    /// otherwise leaves the note in Inbox with a recommended destination at the top.
    async fn run_inbox(&self, run_id: &RunId, path: &Path, _original: &str) -> anyhow::Result<()> {
        use crate::pipelines::inbox;
        self.store
            .update_status(run_id, RunStatus::Running, None)
            .await?;

        // Re-read post-claim content; build the note in memory, write once at the end.
        let current = std::fs::read_to_string(path)?;
        let mut note = Note::parse(&current);

        // --- Step 1: URL enrich (skip-on-fail) ---
        let urls = inbox::extract_urls(&note.body, 5);
        let mut link_lines: Vec<String> = Vec::new();
        if let Some(fetch) = self.tools.get("web_fetch") {
            for url in &urls {
                let fetched = match fetch.call(serde_json::json!({ "url": url })).await {
                    Ok(c) => c,
                    Err(_) => continue, // dead/failing URL → skip
                };
                let messages = vec![
                    ChatMessage::system(&self.system_prompt),
                    ChatMessage::user(format!(
                        "Summarize this web page in one or two sentences. Return STRICT JSON only: \
                         {{\"tldr\": string, \"action_items\": [string]}}.\n\n{fetched}"
                    )),
                ];
                let out = match run_loop(
                    self.provider.as_ref(),
                    &self.tools,
                    messages,
                    &LoopConfig {
                        model: self.model.clone(),
                        max_iterations: self.max_iterations,
                    },
                )
                .await
                {
                    Ok(r) => r,
                    Err(_) => continue,
                };
                if let Ok(s) = crate::gate::validate_summary(&out.content) {
                    link_lines.push(format!("- {url}: {}", s.tldr));
                }
            }
        }
        if !link_lines.is_empty() {
            note.body = construct_obsidian::block::upsert_named(
                &note.body,
                "inbox-links",
                &link_lines.join("\n"),
            );
        }

        // --- Step 2: Summarize the note ---
        let summary = {
            let messages = vec![
                ChatMessage::system(&self.system_prompt),
                ChatMessage::user(format!(
                    "Summarize the following note. Return STRICT JSON only: \
                     {{\"tldr\": string, \"action_items\": [string]}}.\n\n{}",
                    note.body
                )),
            ];
            match run_loop(
                self.provider.as_ref(),
                &self.tools,
                messages,
                &LoopConfig {
                    model: self.model.clone(),
                    max_iterations: self.max_iterations,
                },
            )
            .await
            {
                Ok(r) => match crate::gate::validate_summary(&r.content) {
                    Ok(s) => s,
                    Err(e) => return self.fail(run_id, path, &e.to_string()).await,
                },
                Err(e) => return self.fail(run_id, path, &e.to_string()).await,
            }
        };
        note.body = construct_obsidian::block::upsert_named_at_top(
            &note.body,
            "summary",
            &crate::pipelines::summarize::render_summary(&summary),
        );

        // --- Step 3: Tag the note ---
        let existing = construct_obsidian::vault::existing_tags_excluding(
            &self.vault_path,
            &self.exclude_dirs,
        );
        {
            let messages = vec![
                ChatMessage::system(&self.system_prompt),
                ChatMessage::user(format!(
                    "Choose tags for this note. PREFER reusing these existing vault tags when they fit: {}. \
                     Return STRICT JSON only: {{\"tags\": [string]}}.\n\nNOTE:\n{}",
                    existing.join(", "),
                    note.body
                )),
            ];
            match run_loop(
                self.provider.as_ref(),
                &self.tools,
                messages,
                &LoopConfig {
                    model: self.model.clone(),
                    max_iterations: self.max_iterations,
                },
            )
            .await
            {
                Ok(r) => match crate::gate::validate_tags(&r.content, self.max_tags) {
                    Ok(tags) => note.merge_tags(&tags),
                    Err(e) => return self.fail(run_id, path, &e.to_string()).await,
                },
                Err(e) => return self.fail(run_id, path, &e.to_string()).await,
            }
        }

        // --- Step 4: Move decision ---
        let folders = construct_obsidian::vault::list_folders(&self.vault_path, &self.exclude_dirs);
        let proposal = {
            let messages = vec![
                ChatMessage::system(&self.system_prompt),
                ChatMessage::user(format!(
                    "Suggest the single best destination folder for this note. It may be an \
                     existing folder from this list, or a new folder you propose: {}. \
                     Return STRICT JSON only: {{\"destination\": string, \"reason\": string}}.\n\nNOTE:\n{}",
                    folders.join(", "),
                    note.body
                )),
            ];
            match run_loop(
                self.provider.as_ref(),
                &self.tools,
                messages,
                &LoopConfig {
                    model: self.model.clone(),
                    max_iterations: self.max_iterations,
                },
            )
            .await
            {
                Ok(r) => match crate::gate::validate_destination(&r.content) {
                    Ok(p) => p,
                    Err(e) => return self.fail(run_id, path, &e.to_string()).await,
                },
                Err(e) => return self.fail(run_id, path, &e.to_string()).await,
            }
        };

        let note_name = path
            .file_name()
            .map(|n| n.to_string_lossy().to_string())
            .unwrap_or_else(|| "note.md".to_string());
        // Auto-move only into an EXISTING folder that is NOT the note's current folder.
        // Guard against a self-move: if the model proposes the Inbox folder itself
        // (which is in `folders`), moving would just rename the note in place — instead
        // fall through to a recommendation so the human decides.
        let dest_dir = self.vault_path.join(&proposal.destination);
        let is_existing = folders.iter().any(|f| f == &proposal.destination)
            && path.parent() != Some(dest_dir.as_path());

        let outcome: String;
        if is_existing {
            // Auto-move into the existing folder. Finalize: status=done, drop run id.
            note.set_str(crate::pipelines::STATUS_KEY, "done");
            note.remove(crate::pipelines::RUN_KEY);
            let finalized = crate::pipelines::journal_tag::ensure_journal_tag(
                &note.to_string(),
                chrono::Local::now().date_naive(),
            );
            let target = self.collision_free_target(&proposal.destination, path);
            if let Some(parent) = target.parent() {
                std::fs::create_dir_all(parent)?;
            }
            write_atomic(path, &finalized)?;
            std::fs::rename(path, &target)?;
            self.store
                .update_status(run_id, RunStatus::Done, None)
                .await?;
            self.store
                .append_event(
                    run_id,
                    "inbox",
                    "moved",
                    serde_json::json!({"to": target.to_string_lossy()}),
                )
                .await?;
            outcome = format!("moved→{}", proposal.destination);
        } else {
            // Recommend only: write block + frontmatter review, mark the run terminal.
            let recommended = crate::pipelines::journal_tag::ensure_journal_tag(
                &inbox::apply_recommendation(
                    &note.to_string(),
                    &proposal.destination,
                    &proposal.reason,
                ),
                chrono::Local::now().date_naive(),
            );
            write_atomic(path, &recommended)?;
            self.store
                .update_status(run_id, RunStatus::Done, None)
                .await?;
            self.store
                .append_event(
                    run_id,
                    "inbox",
                    "recommended",
                    serde_json::json!({"destination": proposal.destination}),
                )
                .await?;
            outcome = format!("recommended→{}", proposal.destination);
        }

        // --- Step 5: _index log (best-effort) ---
        let moved_dest = is_existing.then(|| proposal.destination.clone());
        let when = chrono::Local::now().date_naive().to_string();
        let summary_outcome = format!(
            "enriched {} url(s), summarized, tagged, {outcome}",
            link_lines.len()
        );
        if let Some(dir) = path.parent() {
            let index_path = dir.join("_index.md");
            let cur = std::fs::read_to_string(&index_path).unwrap_or_default();
            let updated = inbox::update_index(
                &cur,
                &inbox::IndexEntry {
                    note_name: &note_name,
                    outcome: &summary_outcome,
                    destination: moved_dest.as_deref(),
                    when: &when,
                },
            );
            if let Err(e) = write_atomic(&index_path, &updated) {
                tracing::warn!("failed to update inbox _index: {e}");
            }
        }
        Ok(())
    }

    /// Tag pipeline: agent loop → gate → auto-apply tags to frontmatter (no human review).
    /// Prefers reusing tags already present in the vault.
    async fn run_tag(&self, run_id: &RunId, path: &Path, original: &str) -> anyhow::Result<()> {
        self.store
            .update_status(run_id, RunStatus::Running, None)
            .await?;
        let note = Note::parse(original);
        let existing = construct_obsidian::vault::existing_tags_excluding(
            &self.vault_path,
            &self.exclude_dirs,
        );
        let user_prompt = format!(
            "Choose tags for this note. PREFER reusing these existing vault tags when they fit: {}. \
             Return STRICT JSON only: {{\"tags\": [string]}}.\n\nNOTE:\n{}",
            existing.join(", "),
            note.body
        );
        let messages = vec![
            ChatMessage::system(&self.system_prompt),
            ChatMessage::user(user_prompt),
        ];
        let out = match run_loop(
            self.provider.as_ref(),
            &self.tools,
            messages,
            &LoopConfig {
                model: self.model.clone(),
                max_iterations: self.max_iterations,
            },
        )
        .await
        {
            Ok(r) => r,
            Err(e) => return self.fail(run_id, path, &e.to_string()).await,
        };
        let tags = match crate::gate::validate_tags(&out.content, self.max_tags) {
            Ok(t) => t,
            Err(e) => return self.fail(run_id, path, &e.to_string()).await,
        };
        let current = std::fs::read_to_string(path)?;
        write_atomic(
            path,
            &crate::pipelines::journal_tag::ensure_journal_tag(
                &crate::pipelines::tag::apply_tags(&current, &tags, self.done_tag.as_deref()),
                chrono::Local::now().date_naive(),
            ),
        )?;
        self.store
            .update_status(run_id, RunStatus::Done, None)
            .await?;
        self.store
            .append_event(
                run_id,
                "tag",
                "done",
                serde_json::json!({"count": tags.len()}),
            )
            .await?;
        Ok(())
    }

    /// Daily summary pipeline (scheduled). Standalone: creates its own store run,
    /// scans yesterday's changed notes, and writes/updates today's journal day note
    /// with four managed sections. `today` is injected (from the scheduler's clock)
    /// so this is fully testable. Writes ONLY managed blocks — no frontmatter claim.
    pub async fn run_daily_summary(
        &self,
        today: chrono::NaiveDate,
        journal_folder: &str,
    ) -> anyhow::Result<()> {
        use crate::pipelines::daily;
        let yesterday = today.pred_opt().unwrap_or(today);
        let managed = None; // managed-folder exclusion handled by exclude_dirs if set

        let day_rel = daily::journal_day_path(journal_folder, today);
        let day_note = self.vault_path.join(&day_rel);
        let note_path_str = day_note.to_string_lossy().to_string();

        // Store run for observability (no note claim).
        let run_id = RunId::new();
        self.store
            .create_run(&RunRecord {
                id: run_id.clone(),
                rule: self.rule.clone(),
                agent: self.agent.clone(),
                note_path: note_path_str.clone(),
                status: RunStatus::Running,
                error: None,
            })
            .await?;
        self.store
            .append_event(
                &run_id,
                "daily",
                "running",
                serde_json::json!({"day": today.to_string()}),
            )
            .await?;

        // 1. Scan yesterday's changed notes (loop-guarded).
        let changed = daily::changed_notes_on(
            &self.vault_path,
            &self.exclude_dirs,
            journal_folder,
            managed,
            yesterday,
        );

        // 2. Deterministic tasks: open checkboxes from changed notes + carryover.
        //    Track which notes contributed a task so "Other notes" can list only the
        //    notes NOT otherwise represented (spec: links to changed notes not captured).
        let mut note_tasks: Vec<String> = Vec::new();
        let mut contributing: std::collections::HashSet<String> = std::collections::HashSet::new();
        for p in &changed {
            if let Ok(text) = std::fs::read_to_string(p) {
                let tasks = daily::scrape_open_checkboxes(&text);
                if !tasks.is_empty() {
                    contributing.insert(p.to_string_lossy().to_string());
                }
                note_tasks.extend(tasks);
            }
        }
        // Carryover: still-unchecked items from yesterday's journal day-note.
        let yest_journal = self
            .vault_path
            .join(daily::journal_day_path(journal_folder, yesterday));
        let yesterday_open = match std::fs::read_to_string(&yest_journal) {
            Ok(t) => daily::scrape_open_checkboxes(&t),
            Err(_) => Vec::new(), // missing yesterday note → empty carryover, not an error
        };
        // Tasks block = tasks from yesterday's changed notes only; carryover block =
        // yesterday's still-open journal items not already in the task list. A task
        // appears in exactly ONE section, so tomorrow's scrape can't double-count it.
        let today_tasks = daily::dedupe_normalized(note_tasks);
        let carryover = daily::partition_carryover(&today_tasks, &yesterday_open);

        // 3. Rich recap (agentic) — skip the model entirely if nothing changed
        //    AND there is no brief to fold in.
        let brief_text = self.todays_brief_text(today);
        let summary_body = if changed.is_empty() && brief_text.is_empty() {
            daily::render_summary_section("No notes changed yesterday.", &[], &[])
        } else {
            // Inputs: per-note excerpts capped at 700 chars, 12 notes / ~8k chars total.
            let mut excerpts = String::new();
            for p in changed.iter().take(12) {
                if excerpts.chars().count() > 8_000 {
                    break;
                }
                if let Ok(text) = std::fs::read_to_string(p) {
                    let stem = p.file_stem().unwrap_or_default().to_string_lossy();
                    excerpts.push_str(&format!("### {stem}\n{}\n\n", daily::excerpt(&text, 700)));
                }
            }
            let completed = daily::scrape_checked_checkboxes(
                &std::fs::read_to_string(&yest_journal).unwrap_or_default(),
            );
            let template = self.recap_template();
            let user_prompt = daily::fill_recap_template(
                &template,
                if excerpts.is_empty() {
                    "(none)"
                } else {
                    &excerpts
                },
                &bullet_list_or_none(&completed),
                &bullet_list_or_none(&carryover),
                if brief_text.is_empty() {
                    "(none)"
                } else {
                    &brief_text
                },
            );
            let messages = vec![
                ChatMessage::system(&self.system_prompt),
                ChatMessage::user(user_prompt),
            ];
            match run_loop(
                self.provider.as_ref(),
                &self.tools,
                messages,
                &LoopConfig {
                    model: self.model.clone(),
                    max_iterations: self.max_iterations,
                },
            )
            .await
            {
                Ok(r) => match crate::gate::validate_recap(&r.content) {
                    Ok(rec) => {
                        daily::render_summary_section(&rec.tldr, &rec.highlights, &rec.action_items)
                    }
                    Err(e) => return self.fail_daily(&run_id, &e.to_string()).await,
                },
                Err(e) => return self.fail_daily(&run_id, &e.to_string()).await,
            }
        };

        // 4. Other notes: wikilinks to changed notes that contributed no task
        //    (i.e. not otherwise represented in the task list).
        let other_links: Vec<String> = changed
            .iter()
            .filter(|p| !contributing.contains(&p.to_string_lossy().to_string()))
            .map(|p| {
                format!(
                    "[[{}]]",
                    p.file_stem().unwrap_or_default().to_string_lossy()
                )
            })
            .collect();

        // 5. Ensure journal dirs exist, render + write the day note (managed blocks only).
        if let Some(parent) = day_note.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let current = std::fs::read_to_string(&day_note).unwrap_or_default();
        let updated = daily::render_day_note(
            &current,
            &today_tasks,
            &carryover,
            &summary_body,
            &other_links,
        );
        let updated = crate::pipelines::journal_tag::ensure_journal_tag(&updated, today);
        write_atomic(&day_note, &updated)?;

        self.store
            .update_status(&run_id, RunStatus::Done, None)
            .await?;
        self.store
            .append_event(
                &run_id,
                "daily",
                "done",
                serde_json::json!({"tasks": today_tasks.len(), "changed": changed.len()}),
            )
            .await?;
        Ok(())
    }

    /// Brief pipeline (event-driven). Hash-guarded: unchanged content is a
    /// no-op. Updates the `daily-brief` managed block in the brief's day note,
    /// then refreshes the recap so it can fold the brief in. Like the daily
    /// pipeline, this never writes frontmatter claims to the day note.
    pub async fn run_brief(
        &self,
        path: &Path,
        date: chrono::NaiveDate,
        journal_folder: &str,
    ) -> anyhow::Result<()> {
        use crate::pipelines::{brief, daily, journal_tag};

        let text = std::fs::read_to_string(path)?;
        let hash = brief::content_hash(&text);
        let path_str = path.to_string_lossy().to_string();
        if self.store.get_brief_hash(&path_str).await?.as_deref() == Some(hash.as_str()) {
            tracing::debug!("brief unchanged, skipping: {path_str}");
            return Ok(());
        }

        // 1. Deterministic: upsert the daily-brief block into the day note.
        let day_note = self
            .vault_path
            .join(daily::journal_day_path(journal_folder, date));
        if let Some(parent) = day_note.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let stem = path
            .file_stem()
            .map(|s| s.to_string_lossy().to_string())
            .unwrap_or_default();
        let outline = brief::extract_outline(&text, 12);
        let section = brief::render_brief_section(&stem, &outline);
        let current = std::fs::read_to_string(&day_note).unwrap_or_default();
        let updated = construct_obsidian::block::upsert_named(&current, "daily-brief", &section);
        let updated = journal_tag::ensure_journal_tag(&updated, date);
        write_atomic(&day_note, &updated)?;

        // 2. Agentic: refresh the recap (it reads the brief as context). A
        //    recap failure must not lose the deterministic update above, and
        //    the hash is only recorded on full success so the next event retries.
        self.run_daily_summary(date, journal_folder).await?;
        self.store.set_brief_hash(&path_str, &hash).await?;
        tracing::info!(
            "brief folded into day note: {stem} → {}",
            day_note.display()
        );
        Ok(())
    }

    /// prompts/daily_summary.md from prompt_dir when present, else the
    /// embedded default — recap stays tunable without recompiling.
    fn recap_template(&self) -> String {
        self.prompt_dir
            .as_ref()
            .map(|d| d.join("daily_summary.md"))
            .filter(|p| p.exists())
            .and_then(|p| std::fs::read_to_string(p).ok())
            .unwrap_or_else(|| crate::pipelines::daily::DEFAULT_RECAP_TEMPLATE.to_string())
    }

    /// Excerpt of today's Daily Brief ("" when briefs are off / no brief yet).
    fn todays_brief_text(&self, today: chrono::NaiveDate) -> String {
        let Some(folder) = &self.briefs_folder else {
            return String::new();
        };
        let dir = self.vault_path.join(folder);
        let Ok(entries) = std::fs::read_dir(&dir) else {
            return String::new();
        };
        for e in entries.flatten() {
            let name = e.file_name().to_string_lossy().to_string();
            if name.ends_with(".md")
                && construct_obsidian::watcher::parse_brief_date(&name) == Some(today)
            {
                if let Ok(text) = std::fs::read_to_string(e.path()) {
                    return crate::pipelines::daily::excerpt(&text, 1_500);
                }
            }
        }
        String::new()
    }

    async fn fail(&self, run_id: &RunId, path: &Path, msg: &str) -> anyhow::Result<()> {
        let current = std::fs::read_to_string(path).unwrap_or_default();
        let mut note = Note::parse(&current);
        note.set_str(crate::pipeline::STATUS_KEY, "error");
        let _ = write_atomic(path, &note.to_string());
        self.store
            .update_status(run_id, RunStatus::Error, Some(msg.to_string()))
            .await?;
        self.store
            .append_event(
                run_id,
                "error",
                "failed",
                serde_json::json!({"message": msg}),
            )
            .await?;
        Ok(())
    }

    /// Failure path for the daily-summary pipeline. Unlike `fail()`, this NEVER
    /// writes frontmatter to a file — journal day notes must never carry a
    /// `construct_status` claim. It only records the error on the store run and
    /// returns Err so the scheduler logs it and retries on the next fire.
    async fn fail_daily(&self, run_id: &RunId, msg: &str) -> anyhow::Result<()> {
        let _ = self
            .store
            .update_status(run_id, RunStatus::Error, Some(msg.to_string()))
            .await;
        let _ = self
            .store
            .append_event(
                run_id,
                "daily",
                "error",
                serde_json::json!({"message": msg}),
            )
            .await;
        Err(anyhow::anyhow!("daily summary failed: {msg}"))
    }
}

/// "- item" lines, or "(none)" — keeps template slots non-empty and unambiguous.
fn bullet_list_or_none(items: &[String]) -> String {
    if items.is_empty() {
        "(none)".to_string()
    } else {
        items
            .iter()
            .map(|i| format!("- {i}"))
            .collect::<Vec<_>>()
            .join("\n")
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::pipelines::PipelineKind;
    use crate::testkit::{EchoTool, ScriptedModel};
    use construct_core::model::{ChatResponse, Role, ToolCall};
    use construct_store::SqliteStore;
    use std::io::Write;

    /// Build a plain-content (no tool calls) ChatResponse.
    fn chat_text(s: &str) -> ChatResponse {
        ChatResponse {
            message: ChatMessage::assistant(s),
        }
    }

    async fn orch(provider: Arc<dyn ModelProvider>) -> Orchestrator {
        let store = Arc::new(SqliteStore::connect("sqlite::memory:").await.unwrap());
        let mut tools: HashMap<String, Arc<dyn Tool>> = HashMap::new();
        // Output includes the URL the scripted model will cite, so the grounding gate passes.
        tools.insert(
            "web_search".into(),
            Arc::new(EchoTool::new(
                "web_search",
                "Rust lang https://rust-lang.org systems programming",
            )),
        );
        Orchestrator {
            store,
            provider,
            tools,
            model: "m".into(),
            agent: "Scout".into(),
            rule: "research".into(),
            pipeline: crate::pipelines::PipelineKind::Research,
            system_prompt: "You are Scout.".into(),
            max_iterations: 5,
            done_tag: Some("theconstruct/done".into()),
            vault_path: std::path::PathBuf::from("/tmp"),
            max_tags: 8,
            exclude_dirs: vec![],
            prompt_dir: None,
            briefs_folder: None,
            file_rules: vec![],
        }
    }

    fn search_then_answer() -> ScriptedModel {
        let tool_turn = ChatResponse {
            message: ChatMessage {
                role: Role::Assistant,
                content: String::new(),
                tool_calls: vec![ToolCall {
                    id: "c1".into(),
                    name: "web_search".into(),
                    arguments: serde_json::json!({"query":"x"}),
                }],
                tool_call_id: None,
            },
        };
        let answer = ChatResponse {
            message: ChatMessage::assistant(
                r#"{"summary":"Found it","findings":["a","b"],"sources":[{"title":"Rust","url":"https://rust-lang.org"}]}"#,
            ),
        };
        ScriptedModel::new(vec![tool_turn, answer])
    }

    fn summary_model() -> ScriptedModel {
        ScriptedModel::new(vec![ChatResponse {
            message: ChatMessage::assistant(r#"{"tldr":"It is about X.","action_items":["do y"]}"#),
        }])
    }

    fn tags_model() -> ScriptedModel {
        ScriptedModel::new(vec![ChatResponse {
            message: ChatMessage::assistant(r#"{"tags":["rust","cli"]}"#),
        }])
    }

    fn organize_model() -> ScriptedModel {
        ScriptedModel::new(vec![ChatResponse {
            message: ChatMessage::assistant(r#"{"destination":"Projects","reason":"active"}"#),
        }])
    }

    #[tokio::test]
    async fn decision_ignores_runs_owned_by_other_rule() {
        let dir = tempfile::tempdir().unwrap();
        let note_path = dir.path().join("n.md");
        // A note already in 'review' from an organize run (proposed move present).
        std::fs::write(
            &note_path,
            "---\nconstruct_status: review\nconstruct_run_id: r-x\nconstruct_proposed_move: Projects\n---\nbody",
        )
        .unwrap();
        // This orchestrator is a RESEARCH orchestrator (rule = "research" by default in orch()).
        let o = orch(Arc::new(tags_model())).await; // model irrelevant; default rule "research"
                                                    // The run in the store is owned by rule "organize".
        let run_id = RunId::new();
        o.store
            .create_run(&RunRecord {
                id: run_id.clone(),
                rule: "organize".into(),
                agent: "Filer".into(),
                note_path: note_path.to_string_lossy().to_string(),
                status: RunStatus::Review,
                error: None,
            })
            .await
            .unwrap();
        // User accepts. A research orchestrator must NOT act on an organize-owned run.
        o.handle(VaultEvent::StatusChanged {
            path: note_path.clone(),
            status: "accepted".into(),
        })
        .await
        .unwrap();
        let after = std::fs::read_to_string(&note_path).unwrap();
        // Unchanged: still 'review', proposal intact, NOT flipped to done by the research branch.
        assert!(after.contains("construct_status: review"));
        assert!(after.contains("construct_proposed_move: Projects"));
    }

    #[tokio::test]
    async fn remind_runs_with_zero_model_calls() {
        use crate::testkit::PanicModel;
        let dir = tempfile::tempdir().unwrap();
        let note_path = dir.path().join("r.md");
        std::fs::write(
            &note_path,
            "remind me to call the dentist tomorrow #theconstruct/remind-me",
        )
        .unwrap();
        // PanicModel panics if the model is ever touched. If this test passes, the
        // remind-me pipeline provably made zero model calls.
        let mut o = orch(Arc::new(PanicModel)).await;
        o.pipeline = PipelineKind::RemindMe;
        o.rule = "remind-me".into();
        o.vault_path = dir.path().to_path_buf();
        o.handle(VaultEvent::NoteTagged {
            path: note_path.clone(),
            tag: "theconstruct/remind-me".into(),
        })
        .await
        .unwrap();
        let after = std::fs::read_to_string(&note_path).unwrap();
        assert!(after.contains("construct_status: done"));
        assert!(after.contains("⏰ Reminder:"));
        assert!(after.contains("call the dentist"));
        assert!(after.contains("construct_reminder_due"));
        // The run was recorded as deterministic.
        let runs = o.store.list_runs(10).await.unwrap();
        assert_eq!(runs[0].status, RunStatus::Done);
    }

    #[tokio::test]
    async fn file_this_deterministic_rule_files_with_zero_model_calls() {
        use crate::testkit::PanicModel;
        let dir = tempfile::tempdir().unwrap();
        let note_path = dir.path().join("k.md");
        std::fs::write(
            &note_path,
            "notes about k8s ingress #theconstruct/file-this",
        )
        .unwrap();
        let mut o = orch(Arc::new(PanicModel)).await;
        o.pipeline = PipelineKind::FileThis;
        o.rule = "file-this".into();
        o.vault_path = dir.path().to_path_buf();
        o.file_rules = vec![construct_config::FileRule {
            any_of: vec!["k8s".into()],
            folder: "Reference".into(),
        }];
        // PanicModel would panic if the model were touched; a matching rule means
        // file-this proposes deterministically.
        o.handle(VaultEvent::NoteTagged {
            path: note_path.clone(),
            tag: "theconstruct/file-this".into(),
        })
        .await
        .unwrap();
        let after = std::fs::read_to_string(&note_path).unwrap();
        assert!(after.contains("construct_status: review"));
        assert!(after.contains("construct_proposed_move: Reference"));
    }

    #[tokio::test]
    async fn remind_without_instruction_errors() {
        use crate::testkit::PanicModel;
        let dir = tempfile::tempdir().unwrap();
        let note_path = dir.path().join("r.md");
        std::fs::write(&note_path, "just a note #theconstruct/remind-me").unwrap();
        let mut o = orch(Arc::new(PanicModel)).await;
        o.pipeline = PipelineKind::RemindMe;
        o.rule = "remind-me".into();
        o.vault_path = dir.path().to_path_buf();
        o.handle(VaultEvent::NoteTagged {
            path: note_path.clone(),
            tag: "theconstruct/remind-me".into(),
        })
        .await
        .unwrap();
        let after = std::fs::read_to_string(&note_path).unwrap();
        assert!(after.contains("construct_status: error"));
    }

    #[tokio::test]
    async fn organize_proposes_then_moves_on_accept() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(dir.path().join("Projects")).unwrap();
        let note_path = dir.path().join("n.md");
        std::fs::write(&note_path, "body #theconstruct/organize").unwrap();
        let mut o = orch(Arc::new(organize_model())).await;
        o.pipeline = crate::pipelines::PipelineKind::Organize;
        o.vault_path = dir.path().to_path_buf();

        // propose → review (file not moved yet)
        o.handle(VaultEvent::NoteTagged {
            path: note_path.clone(),
            tag: "theconstruct/organize".into(),
        })
        .await
        .unwrap();
        let after = std::fs::read_to_string(&note_path).unwrap();
        assert!(after.contains("construct_status: review"));
        assert!(after.contains("construct_proposed_move: Projects"));
        assert!(note_path.exists());

        // accept → file moved into Projects/
        let accepted = after.replace("construct_status: review", "construct_status: accepted");
        std::fs::write(&note_path, &accepted).unwrap();
        o.handle(VaultEvent::StatusChanged {
            path: note_path.clone(),
            status: "accepted".into(),
        })
        .await
        .unwrap();
        assert!(!note_path.exists());
        let moved = dir.path().join("Projects/n.md");
        assert!(moved.exists());
        assert!(std::fs::read_to_string(&moved)
            .unwrap()
            .contains("construct_status: done"));
    }

    #[tokio::test]
    async fn tag_auto_applies() {
        let dir = tempfile::tempdir().unwrap();
        let note_path = dir.path().join("n.md");
        std::fs::write(&note_path, "body about rust #theconstruct/tag").unwrap();
        let mut o = orch(Arc::new(tags_model())).await;
        o.pipeline = crate::pipelines::PipelineKind::Tag;
        o.vault_path = dir.path().to_path_buf();
        o.handle(VaultEvent::NoteTagged {
            path: note_path.clone(),
            tag: "theconstruct/tag".into(),
        })
        .await
        .unwrap();
        let after = std::fs::read_to_string(&note_path).unwrap();
        assert!(after.contains("construct_status: done"));
        assert!(after.contains("rust"));
        assert!(after.contains("cli"));
    }

    #[tokio::test]
    async fn summarize_auto_applies_to_done() {
        let dir = tempfile::tempdir().unwrap();
        let note_path = dir.path().join("n.md");
        std::fs::write(&note_path, "Long note body #theconstruct/summarize").unwrap();
        let mut o = orch(Arc::new(summary_model())).await;
        o.pipeline = crate::pipelines::PipelineKind::Summarize;
        o.handle(VaultEvent::NoteTagged {
            path: note_path.clone(),
            tag: "theconstruct/summarize".into(),
        })
        .await
        .unwrap();
        let after = std::fs::read_to_string(&note_path).unwrap();
        assert!(after.contains("construct_status: done"));
        assert!(after.contains("TL;DR"));
        assert!(after.contains("- [ ] do y"));
    }

    #[tokio::test]
    async fn unimplemented_pipeline_sets_error() {
        let dir = tempfile::tempdir().unwrap();
        let note_path = dir.path().join("n.md");
        std::fs::write(&note_path, "Body #theconstruct/tag").unwrap();
        let mut o = orch(Arc::new(search_then_answer())).await;
        o.pipeline = crate::pipelines::PipelineKind::Organize; // not wired until Phase 3
        o.handle(VaultEvent::NoteTagged {
            path: note_path.clone(),
            tag: "theconstruct/tag".into(),
        })
        .await
        .unwrap();
        let after = std::fs::read_to_string(&note_path).unwrap();
        assert!(after.contains("construct_status: error"));
    }

    #[tokio::test]
    async fn full_research_then_accept_flow() {
        let dir = tempfile::tempdir().unwrap();
        let note_path = dir.path().join("topic.md");
        let mut f = std::fs::File::create(&note_path).unwrap();
        write!(f, "Research the Rust language #theconstruct/research").unwrap();
        drop(f);

        let o = orch(Arc::new(search_then_answer())).await;

        // tagged → research → review
        o.handle(VaultEvent::NoteTagged {
            path: note_path.clone(),
            tag: "theconstruct/research".into(),
        })
        .await
        .unwrap();
        let after = std::fs::read_to_string(&note_path).unwrap();
        assert!(after.contains("## Research"));
        assert!(after.contains("construct_status: review"));

        // human accepts
        let accepted = after.replace("construct_status: review", "construct_status: accepted");
        std::fs::write(&note_path, &accepted).unwrap();
        o.handle(VaultEvent::StatusChanged {
            path: note_path.clone(),
            status: "accepted".into(),
        })
        .await
        .unwrap();

        let done = std::fs::read_to_string(&note_path).unwrap();
        assert!(done.contains("construct_status: done"));
        assert!(done.contains("#theconstruct/done"));
        let runs = o.store.list_runs(10).await.unwrap();
        assert_eq!(runs[0].status, RunStatus::Done);
    }

    #[tokio::test]
    async fn bad_output_sets_error_status() {
        let dir = tempfile::tempdir().unwrap();
        let note_path = dir.path().join("topic.md");
        std::fs::write(&note_path, "Topic #theconstruct/research").unwrap();

        // Model answers with non-JSON → gate fails.
        let model = ScriptedModel::new(vec![ChatResponse {
            message: ChatMessage::assistant("not json"),
        }]);
        let o = orch(Arc::new(model)).await;
        o.handle(VaultEvent::NoteTagged {
            path: note_path.clone(),
            tag: "theconstruct/research".into(),
        })
        .await
        .unwrap();

        let after = std::fs::read_to_string(&note_path).unwrap();
        assert!(after.contains("construct_status: error"));
    }

    #[tokio::test]
    async fn second_trigger_is_idempotent() {
        let dir = tempfile::tempdir().unwrap();
        let note_path = dir.path().join("topic.md");
        std::fs::write(&note_path, "Topic #theconstruct/research").unwrap();
        let o = orch(Arc::new(search_then_answer())).await;
        o.handle(VaultEvent::NoteTagged {
            path: note_path.clone(),
            tag: "theconstruct/research".into(),
        })
        .await
        .unwrap();
        // a second tagged event while in review must not start a new run
        o.handle(VaultEvent::NoteTagged {
            path: note_path.clone(),
            tag: "theconstruct/research".into(),
        })
        .await
        .unwrap();
        assert_eq!(o.store.list_runs(10).await.unwrap().len(), 1);
    }

    #[tokio::test]
    async fn reconcile_restarts_stale_research_run() {
        let dir = tempfile::tempdir().unwrap();
        let note_path = dir.path().join("topic.md");
        std::fs::write(&note_path, "Topic #theconstruct/research").unwrap();
        let o = orch(Arc::new(search_then_answer())).await;

        // Simulate a crash: a run left stuck in `researching`.
        let stale = RunId::new();
        o.store
            .create_run(&RunRecord {
                id: stale.clone(),
                rule: "research".into(),
                agent: "Scout".into(),
                note_path: note_path.to_string_lossy().to_string(),
                status: RunStatus::Researching,
                error: None,
            })
            .await
            .unwrap();

        o.reconcile().await.unwrap();

        // Stale run is marked error; a fresh run drove the note to review.
        assert_eq!(
            o.store.get_run(&stale).await.unwrap().status,
            RunStatus::Error
        );
        let after = std::fs::read_to_string(&note_path).unwrap();
        assert!(after.contains("construct_status: review"));
    }

    #[tokio::test]
    async fn reconcile_recovers_running_run() {
        let dir = tempfile::tempdir().unwrap();
        let note_path = dir.path().join("n.md");
        std::fs::write(&note_path, "body about rust #theconstruct/tag").unwrap();
        let mut o = orch(Arc::new(tags_model())).await;
        o.pipeline = crate::pipelines::PipelineKind::Tag;
        o.rule = "tag".to_string();
        o.vault_path = dir.path().to_path_buf();
        // Simulate a crashed in-flight tag run stuck at Running.
        let stale = RunId::new();
        o.store
            .create_run(&RunRecord {
                id: stale.clone(),
                rule: "tag".into(),
                agent: "Librarian".into(),
                note_path: note_path.to_string_lossy().to_string(),
                status: RunStatus::Running,
                error: None,
            })
            .await
            .unwrap();
        o.reconcile().await.unwrap();
        // Stale run errored out; a fresh tag run drove the note to done.
        assert_eq!(
            o.store.get_run(&stale).await.unwrap().status,
            RunStatus::Error
        );
        let after = std::fs::read_to_string(&note_path).unwrap();
        assert!(after.contains("construct_status: done"));
    }

    #[tokio::test]
    async fn reconcile_ignores_other_rules_runs() {
        let dir = tempfile::tempdir().unwrap();
        let note_path = dir.path().join("n.md");
        std::fs::write(&note_path, "body #theconstruct/research").unwrap();
        let mut o = orch(Arc::new(tags_model())).await;
        o.pipeline = crate::pipelines::PipelineKind::Tag;
        o.rule = "tag".to_string();
        o.vault_path = dir.path().to_path_buf();
        // A stale run owned by a DIFFERENT rule ("research").
        let stale = RunId::new();
        o.store
            .create_run(&RunRecord {
                id: stale.clone(),
                rule: "research".into(),
                agent: "Scout".into(),
                note_path: note_path.to_string_lossy().to_string(),
                status: RunStatus::Queued,
                error: None,
            })
            .await
            .unwrap();
        o.reconcile().await.unwrap();
        // The tag orchestrator must NOT touch a research-owned run.
        assert_eq!(
            o.store.get_run(&stale).await.unwrap().status,
            RunStatus::Queued
        );
        let after = std::fs::read_to_string(&note_path).unwrap();
        assert!(!after.contains("construct_status"));
    }

    #[tokio::test]
    async fn inbox_recommends_when_destination_is_unknown_folder() {
        use crate::pipelines::PipelineKind;
        let tmp = tempfile::tempdir().unwrap();
        let vault = tmp.path();
        let inbox_dir = vault.join("Inbox");
        std::fs::create_dir_all(&inbox_dir).unwrap();
        // An existing folder so list_folders is non-empty but does NOT match the suggestion.
        std::fs::create_dir_all(vault.join("Projects")).unwrap();
        let note_path = inbox_dir.join("idea.md");
        std::fs::write(&note_path, "A thought. See https://example.com/x for more.").unwrap();

        // Scripted model responses, in the order run_inbox consumes them:
        //   1 per fetched URL summary, then note summary, then tags, then destination.
        let model = ScriptedModel::new(vec![
            chat_text(r#"{"tldr":"Link summary.","action_items":[]}"#), // URL #1 summary
            chat_text(r#"{"tldr":"A short thought.","action_items":["follow up"]}"#), // note summary
            chat_text(r#"{"tags":["idea","reading"]}"#),                              // tags
            chat_text(r#"{"destination":"Reading/Articles","reason":"it is an article"}"#), // move decision (unknown folder)
        ]);
        let mut tools: HashMap<String, Arc<dyn Tool>> = HashMap::new();
        tools.insert(
            "web_fetch".into(),
            Arc::new(EchoTool::new(
                "web_fetch",
                "fetched page text about something",
            )),
        );
        let store: Arc<dyn Store> =
            Arc::new(SqliteStore::connect("sqlite::memory:").await.unwrap());
        let orch = Orchestrator {
            store: store.clone(),
            provider: Arc::new(model),
            tools,
            model: "m".into(),
            agent: "Librarian".into(),
            rule: "inbox".into(),
            pipeline: PipelineKind::Inbox,
            system_prompt: "You are the Librarian.".into(),
            max_iterations: 4,
            done_tag: None,
            vault_path: vault.to_path_buf(),
            max_tags: 8,
            exclude_dirs: vec![],
            prompt_dir: None,
            briefs_folder: None,
            file_rules: vec![],
        };

        orch.handle_idle(&note_path).await.unwrap();

        let out = std::fs::read_to_string(&note_path).unwrap();
        let note = Note::parse(&out);
        // Stays in Inbox, frontmatter review, recommendation block present.
        assert!(note_path.exists());
        assert_eq!(note.get_str("construct_status").as_deref(), Some("review"));
        assert!(out.contains("construct:inbox-recommendation:start"));
        assert!(out.contains("Reading/Articles"));
        assert!(out.contains("construct:summary:start")); // summarized
        assert!(out.contains("idea")); // tagged (frontmatter tags)
        assert!(out.contains("construct:inbox-links:start")); // url enriched
                                                              // _index written with the outcome.
        let idx = std::fs::read_to_string(inbox_dir.join("_index.md")).unwrap();
        assert!(idx.contains("[[idea]]"));
        // Store run is terminal (Done) so a future re-run isn't blocked.
        let run = store
            .run_for_note(&note_path.to_string_lossy())
            .await
            .unwrap()
            .unwrap();
        assert_eq!(run.status, RunStatus::Done);
    }

    #[tokio::test]
    async fn inbox_auto_moves_when_destination_is_existing_folder() {
        use crate::pipelines::PipelineKind;
        let tmp = tempfile::tempdir().unwrap();
        let vault = tmp.path();
        let inbox_dir = vault.join("Inbox");
        std::fs::create_dir_all(&inbox_dir).unwrap();
        std::fs::create_dir_all(vault.join("Projects")).unwrap();
        let note_path = inbox_dir.join("task.md");
        std::fs::write(&note_path, "No links here, just a task.").unwrap();

        // No URLs → no per-URL summary response. Order: note summary, tags, destination.
        let model = ScriptedModel::new(vec![
            chat_text(r#"{"tldr":"A task.","action_items":[]}"#),
            chat_text(r#"{"tags":["task"]}"#),
            chat_text(r#"{"destination":"Projects","reason":"it is a project task"}"#),
        ]);
        let mut tools: HashMap<String, Arc<dyn Tool>> = HashMap::new();
        tools.insert(
            "web_fetch".into(),
            Arc::new(EchoTool::new("web_fetch", "x")),
        );
        let store: Arc<dyn Store> =
            Arc::new(SqliteStore::connect("sqlite::memory:").await.unwrap());
        let orch = Orchestrator {
            store: store.clone(),
            provider: Arc::new(model),
            tools,
            model: "m".into(),
            agent: "Librarian".into(),
            rule: "inbox".into(),
            pipeline: PipelineKind::Inbox,
            system_prompt: "You are the Librarian.".into(),
            max_iterations: 4,
            done_tag: None,
            vault_path: vault.to_path_buf(),
            max_tags: 8,
            exclude_dirs: vec![],
            prompt_dir: None,
            briefs_folder: None,
            file_rules: vec![],
        };

        orch.handle_idle(&note_path).await.unwrap();

        // Moved out of Inbox into Projects, status done.
        assert!(!note_path.exists());
        let moved = vault.join("Projects").join("task.md");
        assert!(moved.exists());
        let note = Note::parse(&std::fs::read_to_string(&moved).unwrap());
        assert_eq!(note.get_str("construct_status").as_deref(), Some("done"));
    }

    #[tokio::test]
    async fn daily_summary_builds_day_note_with_sections_and_carryover() {
        use crate::pipelines::daily;
        use crate::pipelines::PipelineKind;
        use chrono::{Local, NaiveDate, TimeZone};

        let tmp = tempfile::tempdir().unwrap();
        let vault = tmp.path();
        let today = NaiveDate::from_ymd_opt(2026, 6, 2).unwrap();
        let yesterday = NaiveDate::from_ymd_opt(2026, 6, 1).unwrap();

        // A note edited yesterday with an open checkbox.
        std::fs::create_dir_all(vault.join("Projects")).unwrap();
        let n = vault.join("Projects/plan.md");
        std::fs::write(&n, "# Plan\n- [ ] ship slice 3\n- [x] write spec\n").unwrap();
        // A second note edited yesterday with NO tasks → belongs in "Other notes".
        let m = vault.join("Projects/musing.md");
        std::fs::write(&m, "# Musing\nJust some prose, no checkboxes.\n").unwrap();
        let ft = filetime::FileTime::from_unix_time(
            Local
                .with_ymd_and_hms(2026, 6, 1, 12, 0, 0)
                .unwrap()
                .timestamp(),
            0,
        );
        filetime::set_file_mtime(&n, ft).unwrap();
        filetime::set_file_mtime(&m, ft).unwrap();

        // Yesterday's journal day-note with an unchecked carryover item.
        let yest_journal = vault.join(daily::journal_day_path("journal", yesterday));
        std::fs::create_dir_all(yest_journal.parent().unwrap()).unwrap();
        std::fs::write(&yest_journal, "## Today's Task List\n- [ ] leftover task\n").unwrap();

        // Rich recap (the only agentic step). Two responses: one per run_daily_summary
        // call (the test fires it twice to check idempotency).
        let model = ScriptedModel::new(vec![
            chat_text(
                r#"{"tldr":"You worked on the project plan.", "highlights": ["Project plan"], "action_items": ["Review budget"]}"#,
            ),
            chat_text(
                r#"{"tldr":"You worked on the project plan again.", "highlights": ["Project plan"], "action_items": ["Review budget"]}"#,
            ),
        ]);
        let tools: HashMap<String, Arc<dyn Tool>> = HashMap::new();
        let store: Arc<dyn Store> =
            Arc::new(SqliteStore::connect("sqlite::memory:").await.unwrap());
        let orch = Orchestrator {
            store: store.clone(),
            provider: Arc::new(model),
            tools,
            model: "m".into(),
            agent: "Librarian".into(),
            rule: "daily_summary".into(),
            pipeline: PipelineKind::DailySummary,
            system_prompt: "You are the Librarian.".into(),
            max_iterations: 4,
            done_tag: None,
            vault_path: vault.to_path_buf(),
            max_tags: 8,
            exclude_dirs: vec![],
            prompt_dir: None,
            briefs_folder: None,
            file_rules: vec![],
        };

        orch.run_daily_summary(today, "journal").await.unwrap();

        let day_note = vault.join(daily::journal_day_path("journal", today));
        assert!(day_note.exists());
        let text = std::fs::read_to_string(&day_note).unwrap();
        // Task scraped from yesterday's changed note.
        assert!(text.contains("- [ ] ship slice 3"));
        // Carryover from yesterday's journal note.
        assert!(text.contains("- [ ] leftover task"));
        // Rich recap.
        assert!(text.contains("You worked on the project plan."));
        assert!(text.contains("**Highlights**"));
        assert!(text.contains("- Project plan"));
        // "Other notes" lists the no-task note, but NOT plan.md (it contributed a task,
        // so it is already represented in the task list).
        assert!(text.contains("[[musing]]"));
        assert!(!text.contains("[[plan]]"));
        // Checked item not scraped.
        assert!(!text.contains("write spec"));

        // Slice 4: day note carries its own journal date tag (frontmatter + literal).
        assert!(
            text.contains("journal/2026/06/02"),
            "frontmatter tag missing"
        );
        assert!(text.contains("#journal/2026/06/02"), "literal tag missing");

        // Idempotent re-run: still one of each block.
        orch.run_daily_summary(today, "journal").await.unwrap();
        let text2 = std::fs::read_to_string(&day_note).unwrap();
        assert_eq!(text2.matches("construct:daily-tasks:start").count(), 1);
    }

    #[tokio::test]
    async fn run_brief_updates_day_note_and_hash_guard_skips_rerun() {
        use crate::pipelines::daily;
        use chrono::{Local, NaiveDate, TimeZone};

        let tmp = tempfile::tempdir().unwrap();
        let vault = tmp.path();

        // The brief date is 2026-06-09; "yesterday" for run_daily_summary is 2026-06-08.
        let brief_date = NaiveDate::from_ymd_opt(2026, 6, 9).unwrap();
        let _yesterday = NaiveDate::from_ymd_opt(2026, 6, 8).unwrap();

        // Write a changed note with mtime = 2026-06-08 so run_daily_summary sees it
        // and calls the model (rather than short-circuiting with "No notes changed").
        std::fs::create_dir_all(vault.join("Notes")).unwrap();
        let changed_note = vault.join("Notes/standup.md");
        std::fs::write(&changed_note, "# Standup notes\n- [ ] review PR\n").unwrap();
        let ft = filetime::FileTime::from_unix_time(
            Local
                .with_ymd_and_hms(2026, 6, 8, 10, 0, 0)
                .unwrap()
                .timestamp(),
            0,
        );
        filetime::set_file_mtime(&changed_note, ft).unwrap();

        // Write the brief file.
        let briefs_dir = vault.join("AI/DailyBriefs");
        std::fs::create_dir_all(&briefs_dir).unwrap();
        let brief_path = briefs_dir.join("2026-06-09.md");
        std::fs::write(
            &brief_path,
            "# Daily Brief\n\n## Calendar\n- 10:00 Standup\n- 14:00 1:1\n",
        )
        .unwrap();

        // Two model responses: one for the initial run_brief call (which calls
        // run_daily_summary), one for the third call after modifying the brief.
        // The second run_brief call (unchanged) must be guarded by the hash and
        // must NOT consume a response — if it did, the third call would fail.
        let model = ScriptedModel::new(vec![
            chat_text(
                r#"{"tldr":"Standup notes reviewed.", "highlights": ["Reviewed PR"], "action_items": ["Follow up"]}"#,
            ),
            chat_text(
                r#"{"tldr":"Brief updated with new content.", "highlights": ["New meeting"], "action_items": []}"#,
            ),
        ]);
        let store: Arc<dyn Store> =
            Arc::new(SqliteStore::connect("sqlite::memory:").await.unwrap());
        let orch = Orchestrator {
            store: store.clone(),
            provider: Arc::new(model),
            tools: HashMap::new(),
            model: "m".into(),
            agent: "Librarian".into(),
            rule: "daily_summary".into(),
            pipeline: PipelineKind::DailySummary,
            system_prompt: "You are the Librarian.".into(),
            max_iterations: 4,
            done_tag: None,
            vault_path: vault.to_path_buf(),
            max_tags: 8,
            exclude_dirs: vec![],
            prompt_dir: None,
            briefs_folder: Some("AI/DailyBriefs".into()),
            file_rules: vec![],
        };

        // 1. First run: brief is new, should process.
        orch.run_brief(&brief_path, brief_date, "journal")
            .await
            .unwrap();

        let day_note_path = vault.join(daily::journal_day_path("journal", brief_date));
        assert!(day_note_path.exists(), "day note should be created");
        let text = std::fs::read_to_string(&day_note_path).unwrap();

        // Day note contains the managed daily-brief block markers.
        assert!(
            text.contains("construct:daily-brief:start"),
            "daily-brief block start missing"
        );
        // Wikilink to the brief note.
        assert!(text.contains("[[2026-06-09]]"), "wikilink to brief missing");
        // Outline bullet from the brief.
        assert!(text.contains("10:00 Standup"), "outline bullet missing");

        // 2. Second run with UNCHANGED content: hash guard must short-circuit.
        //    The ScriptedModel still has 1 response left (for the 3rd run).
        //    If the guard is broken, this call would consume it and the 3rd run would fail.
        orch.run_brief(&brief_path, brief_date, "journal")
            .await
            .unwrap();

        // 3. Append a line to the brief; run again: model should be called once more.
        let mut f = std::fs::OpenOptions::new()
            .append(true)
            .open(&brief_path)
            .unwrap();
        std::io::Write::write_all(&mut f, b"- 15:00 Follow-up call\n").unwrap();
        drop(f);

        orch.run_brief(&brief_path, brief_date, "journal")
            .await
            .unwrap();

        // Updated day note still has the block and the original wikilink.
        let text3 = std::fs::read_to_string(&day_note_path).unwrap();
        assert!(text3.contains("[[2026-06-09]]"));
        assert!(text3.contains("construct:daily-brief:start"));
    }

    #[tokio::test]
    async fn inbox_does_not_self_move_into_its_own_folder() {
        use crate::pipelines::PipelineKind;
        let tmp = tempfile::tempdir().unwrap();
        let vault = tmp.path();
        let inbox_dir = vault.join("Inbox");
        std::fs::create_dir_all(&inbox_dir).unwrap();
        let note_path = inbox_dir.join("thing.md");
        std::fs::write(&note_path, "A note, no links.").unwrap();

        // The model proposes the Inbox folder itself as the destination.
        let model = ScriptedModel::new(vec![
            chat_text(r#"{"tldr":"A note.","action_items":[]}"#),
            chat_text(r#"{"tags":["misc"]}"#),
            chat_text(r#"{"destination":"Inbox","reason":"belongs in inbox"}"#),
        ]);
        let mut tools: HashMap<String, Arc<dyn Tool>> = HashMap::new();
        tools.insert(
            "web_fetch".into(),
            Arc::new(EchoTool::new("web_fetch", "x")),
        );
        let store: Arc<dyn Store> =
            Arc::new(SqliteStore::connect("sqlite::memory:").await.unwrap());
        let orch = Orchestrator {
            store: store.clone(),
            provider: Arc::new(model),
            tools,
            model: "m".into(),
            agent: "Librarian".into(),
            rule: "inbox".into(),
            pipeline: PipelineKind::Inbox,
            system_prompt: "You are the Librarian.".into(),
            max_iterations: 4,
            done_tag: None,
            vault_path: vault.to_path_buf(),
            max_tags: 8,
            exclude_dirs: vec![],
            prompt_dir: None,
            briefs_folder: None,
            file_rules: vec![],
        };

        orch.handle_idle(&note_path).await.unwrap();

        // The note must NOT be moved/renamed; it stays put with a recommendation.
        assert!(note_path.exists(), "note should not be self-moved/renamed");
        assert!(!inbox_dir.join("thing (1).md").exists());
        let note = Note::parse(&std::fs::read_to_string(&note_path).unwrap());
        assert_eq!(note.get_str("construct_status").as_deref(), Some("review"));
    }
}
