//! Cron-driven scheduled workflows.
//!
//! A `--schedules <file>` maps cron expressions to workflow runs. Every entry
//! is parsed and path-checked **at boot** ([`load_schedules`]) so an invalid
//! cron or a missing/escaping workflow fails startup rather than silently never
//! firing. At runtime each schedule gets its own Tokio task that sleeps until
//! the next occurrence and runs the workflow through the same engine path as
//! `POST /v1/workflows/run` ([`crate::handlers::run_workflow_at`]).
//!
//! Overlap policy is **skip-and-log**: if a schedule's previous run is still in
//! flight when its next tick arrives, the tick is skipped (a daemon must not
//! stack runs). No catch-up, no misfire queue — a tick missed while the daemon
//! was down is simply not run.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::str::FromStr;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use chrono::Utc;
use croner::Cron;
use serde::Deserialize;
use tokio::task::JoinHandle;
use tracing::{error, info, warn};

use crate::handlers::{resolve_workflow_path, run_workflow_at, stringify_inputs};
use crate::state::AppState;
use mv_core::MvError;

#[derive(Debug, Deserialize)]
struct SchedulesConfig {
    #[serde(default)]
    schedules: Vec<ScheduleEntry>,
}

#[derive(Debug, Deserialize)]
struct ScheduleEntry {
    /// Cron expression (5-field, or 6-field with leading seconds).
    cron: String,
    /// Workflow file name, resolved inside the server's workflows dir.
    workflow: String,
    #[serde(default)]
    inputs: HashMap<String, serde_json::Value>,
    /// Optional label for logs; defaults to the workflow name.
    #[serde(default)]
    name: Option<String>,
}

/// A validated schedule ready to run.
#[derive(Debug)]
pub struct ParsedSchedule {
    pub label: String,
    pub cron: Cron,
    pub workflow_path: PathBuf,
    pub inputs: HashMap<String, String>,
}

/// Parse and validate every entry in the schedules file. Fails on the first
/// invalid cron expression, escaping/absent workflow name, or missing workflow
/// file — bad config is a boot error, not a silent no-op.
pub fn load_schedules(path: &Path, workflows_dir: &Path) -> Result<Vec<ParsedSchedule>, MvError> {
    let content = std::fs::read_to_string(path).map_err(|e| MvError::ConfigParseError {
        path: path.display().to_string(),
        details: format!("cannot read schedules file: {e}"),
    })?;
    let cfg: SchedulesConfig =
        serde_yml::from_str(&content).map_err(|e| MvError::ConfigParseError {
            path: path.display().to_string(),
            details: format!("invalid schedules YAML: {e}"),
        })?;

    let mut parsed = Vec::with_capacity(cfg.schedules.len());
    for (i, entry) in cfg.schedules.into_iter().enumerate() {
        let label = entry
            .name
            .clone()
            .unwrap_or_else(|| format!("schedule[{i}] ({})", entry.workflow));

        let cron = Cron::from_str(&entry.cron).map_err(|err| MvError::ConfigParseError {
            path: path.display().to_string(),
            details: format!("{label}: invalid cron '{}': {err}", entry.cron),
        })?;

        let workflow_path =
            resolve_workflow_path(workflows_dir, &entry.workflow).map_err(|api| {
                MvError::ConfigParseError {
                    path: path.display().to_string(),
                    details: format!("{label}: {}", api.message),
                }
            })?;
        if !workflow_path.exists() {
            return Err(MvError::ConfigParseError {
                path: path.display().to_string(),
                details: format!("{label}: workflow file '{}' not found", entry.workflow),
            });
        }

        parsed.push(ParsedSchedule {
            label,
            cron,
            workflow_path,
            inputs: stringify_inputs(entry.inputs),
        });
    }
    Ok(parsed)
}

/// Spawn one Tokio task per schedule. Returns the handles so a graceful
/// shutdown can abort them (stopping the scheduler before MCP teardown).
pub fn spawn(state: AppState, schedules: Vec<ParsedSchedule>) -> Vec<JoinHandle<()>> {
    schedules
        .into_iter()
        .map(|sched| {
            let state = state.clone();
            tokio::spawn(run_schedule_loop(state, sched))
        })
        .collect()
}

async fn run_schedule_loop(state: AppState, sched: ParsedSchedule) {
    // `true` while a run spawned by this loop is still in flight. The loop never
    // blocks on the run (it is spawned), so the flag is what enforces skip-on-
    // overlap across ticks.
    let running = Arc::new(AtomicBool::new(false));
    info!(schedule = %sched.label, "scheduler task started");

    loop {
        let now = Utc::now();
        let next = match sched.cron.find_next_occurrence(&now, false) {
            Ok(n) => n,
            Err(e) => {
                error!(schedule = %sched.label, error = %e, "cannot compute next fire; stopping schedule");
                return;
            }
        };
        let wait = (next - now).to_std().unwrap_or(std::time::Duration::ZERO);
        tokio::time::sleep(wait).await;

        if running.swap(true, Ordering::SeqCst) {
            // The previous run is still going — skip this tick (the in-flight
            // run owns the flag and will clear it on completion).
            warn!(schedule = %sched.label, "previous run still in progress; skipping this tick");
            continue;
        }

        let state = state.clone();
        let path = sched.workflow_path.clone();
        let inputs = sched.inputs.clone();
        let label = sched.label.clone();
        let flag = running.clone();
        tokio::spawn(async move {
            info!(schedule = %label, "running scheduled workflow");
            match run_workflow_at(&state, &path, inputs).await {
                Ok((name, _)) => {
                    info!(schedule = %label, workflow = %name, "scheduled workflow completed")
                }
                Err(e) => error!(schedule = %label, error = %e, "scheduled workflow failed"),
            }
            flag.store(false, Ordering::SeqCst);
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn write(dir: &Path, name: &str, body: &str) -> PathBuf {
        let p = dir.join(name);
        std::fs::write(&p, body).unwrap();
        p
    }

    const TOOL_WF: &str = "name: touch\nversion: \"1.0\"\ndescription: t\n\nsteps:\n  - id: s\n    name: s\n    type: tool\n    output: o\n    tool: file_list\n    inputs:\n      path: \".\"\n\noutputs:\n  - name: o\n    from: s\n";

    #[test]
    fn load_accepts_valid_schedule() {
        let dir = tempfile::tempdir().unwrap();
        write(dir.path(), "touch.yaml", TOOL_WF);
        let sched = write(
            dir.path(),
            "schedules.yaml",
            "schedules:\n  - cron: \"0 0 * * *\"\n    workflow: touch.yaml\n    name: nightly\n",
        );
        let parsed = load_schedules(&sched, dir.path()).unwrap();
        assert_eq!(parsed.len(), 1);
        assert_eq!(parsed[0].label, "nightly");
    }

    #[test]
    fn load_rejects_invalid_cron() {
        let dir = tempfile::tempdir().unwrap();
        write(dir.path(), "touch.yaml", TOOL_WF);
        let sched = write(
            dir.path(),
            "schedules.yaml",
            "schedules:\n  - cron: \"not a cron\"\n    workflow: touch.yaml\n",
        );
        let err = load_schedules(&sched, dir.path()).unwrap_err();
        assert!(err.to_string().contains("invalid cron"), "{err}");
    }

    #[test]
    fn load_rejects_missing_workflow_file() {
        let dir = tempfile::tempdir().unwrap();
        let sched = write(
            dir.path(),
            "schedules.yaml",
            "schedules:\n  - cron: \"0 0 * * *\"\n    workflow: nope.yaml\n",
        );
        let err = load_schedules(&sched, dir.path()).unwrap_err();
        assert!(err.to_string().contains("not found"), "{err}");
    }

    #[test]
    fn load_rejects_workflow_path_traversal() {
        let dir = tempfile::tempdir().unwrap();
        let sched = write(
            dir.path(),
            "schedules.yaml",
            "schedules:\n  - cron: \"0 0 * * *\"\n    workflow: ../escape.yaml\n",
        );
        let err = load_schedules(&sched, dir.path()).unwrap_err();
        assert!(err.to_string().contains("workflows directory"), "{err}");
    }
}
