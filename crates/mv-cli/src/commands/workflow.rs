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
    let validation_errors = mv_core::workflow::validate::validate(&workflow);
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

    let tool_server = ToolServer::new()
        .tool(mv_core::tools::file_list::FileList)
        .tool(mv_core::tools::file_read::FileRead)
        .tool(mv_core::tools::shell_exec::ShellExec)
        .tool(mv_core::tools::http_get::HttpGet);
    let agent_handle = tool_server.run();

    let mcp_connections = connect_mcp_servers(args.mcp_config.as_deref(), &agent_handle).await?;

    let prompt_exec = RigPromptExecutor {
        registry,
        agent_handle: agent_handle.clone(),
    };
    let tool_exec = HandleToolExecutor {
        handle: agent_handle,
    };

    let workflow_dir = path.parent().unwrap_or(Path::new("."));
    let result = mv_core::workflow::engine::execute_workflow(
        &workflow,
        inputs,
        &prompt_exec,
        &tool_exec,
        workflow_dir,
    )
    .await;

    // Always shut down MCP connections, even on error
    mv_core::mcp::client::shutdown_all(mcp_connections).await;

    let result = result?;

    // Print outputs
    if json {
        let obj = serde_json::json!({
            "workflow": workflow.name,
            "outputs": result.outputs,
        });
        println!("{}", serde_json::to_string_pretty(&obj).unwrap());
    } else {
        for (name, value) in &result.outputs {
            println!("## {name}\n");
            println!("{value}\n");
        }
    }

    Ok(())
}

pub async fn run_workflow_validate(args: &WorkflowValidateArgs) -> Result<(), mv_core::MvError> {
    let path = std::path::Path::new(&args.file);
    let workflow = mv_core::workflow::parser::load_from_file(path)?;

    let errors = mv_core::workflow::validate::validate(&workflow);
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
