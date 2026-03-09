//! Sub-agent — parallel task execution via independent LLM conversations.
//!
//! Each sub-agent gets its own thread, YantrikDB connection, ToolRegistry,
//! and LLM conversation. The parent agent's LLM backend is shared via Arc.

use std::sync::Arc;
use std::time::Instant;

use yantrik_ml::{ChatMessage, GenerationConfig, LLMBackend, parse_tool_calls};
use yantrikdb_core::YantrikDB;

use crate::config::CompanionConfig;
use crate::tools::{self, PermissionLevel, ToolContext, parse_permission};

/// Configuration for a sub-agent.
#[derive(Clone)]
pub struct SubAgentConfig {
    pub max_steps: usize,
    pub max_tokens: usize,
    pub temperature: f64,
    pub user_name: String,
    pub db_path: String,
    pub embedding_dim: usize,
    pub companion_config: CompanionConfig,
}

/// Result from a completed sub-agent.
pub struct SubAgentResult {
    pub agent_id: String,
    pub task: String,
    pub response: String,
    pub tool_calls_made: Vec<String>,
    pub success: bool,
    pub elapsed_ms: u64,
}

/// A focused sub-agent that runs a single task with its own LLM conversation.
struct SubAgent {
    task: String,
    agent_id: String,
    llm: Arc<dyn LLMBackend>,
    db: YantrikDB,
    registry: tools::ToolRegistry,
    config: SubAgentConfig,
}

impl SubAgent {
    fn new(
        task: String,
        agent_id: String,
        llm: Arc<dyn LLMBackend>,
        config: SubAgentConfig,
    ) -> Result<Self, String> {
        // Open own DB connection (SQLite WAL allows concurrent readers)
        let mut db = YantrikDB::new(&config.db_path, config.embedding_dim)
            .map_err(|e| format!("Failed to open DB: {e}"))?;

        // Set busy timeout for concurrent access
        db.conn()
            .execute_batch("PRAGMA busy_timeout = 5000;")
            .ok();

        // Build tool registry from config
        let registry = tools::build_registry(&config.companion_config);

        Ok(Self {
            task,
            agent_id,
            llm,
            db,
            registry,
            config,
        })
    }

    fn run(self) -> SubAgentResult {
        let start = Instant::now();
        let max_perm = parse_permission(&self.config.companion_config.tools.max_permission);

        // System prompt: focused task execution, no personality/bond
        let system = format!(
            "You are a focused sub-agent. Your task: {}\n\n\
             Complete this task using available tools. Be concise and direct.\n\
             Do NOT engage in conversation — just execute the task and report results.\n\
             User name: {}",
            self.task, self.config.user_name,
        );

        // Select tools relevant to the task
        let selected = crate::companion::select_tools_for_query(&self.task, &self.db, 8);
        let native_tools = self.registry.definitions_for(&selected, max_perm);

        let mut messages = vec![
            ChatMessage::system(&system),
            ChatMessage::user(&self.task),
        ];

        let gen_config = GenerationConfig {
            max_tokens: self.config.max_tokens,
            temperature: self.config.temperature,
            top_p: Some(0.9),
            ..Default::default()
        };

        let mut tool_calls_made = Vec::new();
        let mut response_text = String::new();

        // Check if backend supports native tool calling
        let use_native = !native_tools.is_empty();

        for _step in 0..self.config.max_steps {
            let tools_param: Option<&[serde_json::Value]> = if use_native {
                Some(&native_tools)
            } else {
                None
            };

            // LLM call with retry (up to 2 retries with backoff)
            let llm_response = {
                let mut last_err = String::new();
                let mut result = None;
                for attempt in 0..3 {
                    match self.llm.chat(&messages, &gen_config, tools_param) {
                        Ok(r) => { result = Some(r); break; }
                        Err(e) => {
                            last_err = e.to_string();
                            tracing::warn!(
                                agent = %self.agent_id,
                                attempt = attempt + 1,
                                "Sub-agent LLM error (retrying): {e}"
                            );
                            if attempt < 2 {
                                std::thread::sleep(std::time::Duration::from_millis(
                                    500 * (attempt as u64 + 1)
                                ));
                            }
                        }
                    }
                }
                match result {
                    Some(r) => r,
                    None => {
                        return SubAgentResult {
                            agent_id: self.agent_id,
                            task: self.task,
                            response: format!("Error after 3 attempts: {last_err}"),
                            tool_calls_made,
                            success: false,
                            elapsed_ms: start.elapsed().as_millis() as u64,
                        };
                    }
                }
            };

            // Parse tool calls
            let tc = if !llm_response.tool_calls.is_empty() {
                llm_response.tool_calls.clone()
            } else {
                parse_tool_calls(&llm_response.text)
            };

            if tc.is_empty() {
                // No tool calls — this is the final response
                response_text = llm_response.text;
                break;
            }

            // Add assistant message
            if use_native && !llm_response.api_tool_calls.is_empty() {
                messages.push(ChatMessage::assistant_with_tool_calls(
                    &llm_response.text,
                    llm_response.api_tool_calls.clone(),
                ));
            } else {
                messages.push(ChatMessage::assistant(&llm_response.text));
            }

            // Execute tool calls
            let task_manager = std::sync::Mutex::new(
                crate::task_manager::TaskManager::new(),
            );
            let ctx = ToolContext {
                db: &self.db,
                max_permission: max_perm,
                registry_metadata: None,
                task_manager: Some(&task_manager),
                incognito: false,
                agent_spawner: None, // Sub-agents cannot spawn further sub-agents
            };

            for (idx, call) in tc.iter().enumerate() {
                tool_calls_made.push(call.name.clone());
                let result = self.registry.execute(&ctx, &call.name, &call.arguments);

                tracing::debug!(
                    agent = %self.agent_id,
                    tool = %call.name,
                    result_len = result.len(),
                    "Sub-agent tool result"
                );

                // Append tool result
                if use_native && !llm_response.api_tool_calls.is_empty() {
                    let call_id = llm_response.api_tool_calls.get(idx)
                        .map(|tc| tc.id.as_str())
                        .unwrap_or("call_sub");
                    messages.push(ChatMessage::tool(call_id, &call.name, &result));
                } else {
                    messages.push(ChatMessage::user(&format!(
                        "<data:tool_result name=\"{}\">\n{}\n</data:tool_result>",
                        call.name, result
                    )));
                }
            }
        }

        // If we never got a text response, request a summary
        if response_text.is_empty() && !tool_calls_made.is_empty() {
            messages.push(ChatMessage::user(
                "Summarize what you found/accomplished in 1-2 sentences. No more tool calls.",
            ));
            if let Ok(summary) = self.llm.chat(&messages, &gen_config, None) {
                response_text = summary.text;
            }
        }

        SubAgentResult {
            agent_id: self.agent_id,
            task: self.task,
            response: response_text,
            tool_calls_made,
            success: true,
            elapsed_ms: start.elapsed().as_millis() as u64,
        }
    }
}

/// Spawn parallel sub-agents for independent tasks.
///
/// Each sub-agent runs in its own thread with its own DB connection.
/// Returns results in the same order as the input tasks.
pub fn spawn_parallel_agents(
    tasks: Vec<String>,
    llm: Arc<dyn LLMBackend>,
    config: SubAgentConfig,
) -> Vec<SubAgentResult> {
    let max_agents = 5.min(tasks.len());
    let tasks = &tasks[..max_agents];

    let handles: Vec<_> = tasks
        .iter()
        .enumerate()
        .map(|(i, task)| {
            let task = task.clone();
            let llm = llm.clone();
            let config = config.clone();
            let agent_id = format!("sub-{i}");

            std::thread::spawn(move || {
                match SubAgent::new(task.clone(), agent_id.clone(), llm, config) {
                    Ok(agent) => {
                        tracing::info!(agent = %agent_id, task = %task, "Sub-agent started");
                        agent.run()
                    }
                    Err(e) => SubAgentResult {
                        agent_id,
                        task,
                        response: format!("Failed to initialize: {e}"),
                        tool_calls_made: vec![],
                        success: false,
                        elapsed_ms: 0,
                    },
                }
            })
        })
        .collect();

    handles
        .into_iter()
        .map(|h| {
            h.join().unwrap_or_else(|_| SubAgentResult {
                agent_id: "unknown".into(),
                task: "unknown".into(),
                response: "Sub-agent thread panicked".into(),
                tool_calls_made: vec![],
                success: false,
                elapsed_ms: 0,
            })
        })
        .collect()
}
