//! The `workflow run` / `workflow validate` commands.

use rig::tool::server::ToolServer;

use crate::cli::{WorkflowRunArgs, WorkflowValidateArgs};
use crate::commands::connect_mcp_servers;
use crate::executors::{HandleToolExecutor, RigPromptExecutor};

pub async fn run_workflow(args: &WorkflowRunArgs, json: bool) -> Result<(), mv_core::MvError> {
    use std::path::Path;

    let path = Path::new(&args.file);
    let workflow = mv_core::workflow::parser::load_from_file(path)?;

    // Validate structure
    let workflow_dir = path.parent().unwrap_or(Path::new("."));
    let validation_errors = mv_core::workflow::validate::validate(&workflow, Some(workflow_dir));
    if !validation_errors.is_empty() {
        let details = validation_errors
            .iter()
            .map(|e| format!("  - {e}"))
            .collect::<Vec<_>>()
            .join("\n");
        return Err(mv_core::MvError::WorkflowValidationError { details });
    }

    // Build inputs map
    let inputs: std::collections::HashMap<String, String> = args.inputs.iter().cloned().collect();

    // Set up executors
    let registry = mv_core::ModelRegistry::resolve(args.config.as_deref())?;

    // Validate every model reference (bare `model:` and `prefer:` lists, in
    // branch/parallel arms too) against the registry up front — a typo or an
    // unknown `prefer` entry must fail before execution, not mid-run. Uses the
    // same `ModelNotInRegistry` error the runtime executor would raise.
    for (_step_id, model_id) in workflow.model_references() {
        if registry.get(&model_id).is_none() {
            return Err(mv_core::MvError::ModelNotInRegistry {
                model: model_id,
                available: registry.available_ids().join(", "),
            });
        }
    }

    let tool_server = ToolServer::new()
        .tool(mv_core::tools::file_list::FileList)
        .tool(mv_core::tools::file_read::FileRead)
        .tool(mv_core::tools::shell_exec::ShellExec)
        .tool(mv_core::tools::http_get::HttpGet);
    let agent_handle = tool_server.run();

    let mcp_connections = connect_mcp_servers(args.mcp_config.as_deref(), &agent_handle).await?;

    let default_model = registry.default_model().id.clone();
    let prompt_exec = RigPromptExecutor {
        registry,
        agent_handle: agent_handle.clone(),
    };
    let tool_exec = HandleToolExecutor {
        handle: agent_handle,
    };

    let result = mv_core::workflow::engine::execute_workflow(
        &workflow,
        inputs,
        &prompt_exec,
        &tool_exec,
        workflow_dir,
        &default_model,
    )
    .await;

    // Always shut down MCP connections, even on error
    mv_core::mcp::client::shutdown_all(mcp_connections).await;

    let result = result?;

    // Print outputs. Values are typed (sprint 012): in `--json` they serialize
    // naturally; in text mode a string prints raw (no quotes — back-compat),
    // and any other value prints as pretty JSON.
    if json {
        let obj = serde_json::json!({
            "workflow": workflow.name,
            "outputs": result.outputs,
        });
        println!("{}", serde_json::to_string_pretty(&obj).unwrap());
    } else {
        for (name, value) in &result.outputs {
            println!("## {name}\n");
            match value {
                serde_json::Value::String(s) => println!("{s}\n"),
                other => println!(
                    "{}\n",
                    serde_json::to_string_pretty(other).unwrap_or_else(|_| other.to_string())
                ),
            }
        }
    }

    Ok(())
}

pub async fn run_workflow_validate(args: &WorkflowValidateArgs) -> Result<(), mv_core::MvError> {
    let path = std::path::Path::new(&args.file);
    let workflow = mv_core::workflow::parser::load_from_file(path)?;

    let workflow_dir = path.parent().unwrap_or(std::path::Path::new("."));
    let errors = mv_core::workflow::validate::validate(&workflow, Some(workflow_dir));
    if errors.is_empty() {
        println!(
            "\u{2713} workflow '{}' is valid ({} steps, {} input{}, {} output{})",
            workflow.name,
            workflow.steps.len(),
            workflow.inputs.len(),
            if workflow.inputs.len() == 1 { "" } else { "s" },
            workflow.outputs.len(),
            if workflow.outputs.len() == 1 { "" } else { "s" },
        );
        Ok(())
    } else {
        let details = errors
            .iter()
            .map(|e| format!("  - {e}"))
            .collect::<Vec<_>>()
            .join("\n");
        Err(mv_core::MvError::WorkflowValidationError { details })
    }
}
