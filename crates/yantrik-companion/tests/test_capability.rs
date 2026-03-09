//! Tests for ModelCapabilityProfile and ToolFamily routing.
//!
//! These tests verify the adaptive intelligence layer that adjusts
//! system behavior based on detected model size.

use yantrik_ml::{ModelCapabilityProfile, ModelTier, ToolCallMode, SlotMode, ToolFamily};

// ── Model Tier Detection ──────────────────────────────────────────────

#[test]
fn tier_detection_ollama_tags() {
    // Ollama format: model:Xb-variant
    assert_eq!(ModelTier::from_model_name("qwen3.5:0.6b"), ModelTier::Tiny);
    assert_eq!(ModelTier::from_model_name("qwen3.5:1b"), ModelTier::Tiny);
    assert_eq!(ModelTier::from_model_name("qwen3.5:3b"), ModelTier::Small);
    assert_eq!(ModelTier::from_model_name("qwen3.5:7b"), ModelTier::Medium);
    assert_eq!(ModelTier::from_model_name("qwen3.5:9b"), ModelTier::Medium);
    assert_eq!(ModelTier::from_model_name("qwen3.5:14b"), ModelTier::Large);
    assert_eq!(ModelTier::from_model_name("qwen3.5:27b-nothink"), ModelTier::Large);
    assert_eq!(ModelTier::from_model_name("qwen3.5:35b"), ModelTier::Large);
    assert_eq!(ModelTier::from_model_name("llama3.2:3b-instruct"), ModelTier::Small);
}

#[test]
fn tier_detection_huggingface() {
    assert_eq!(ModelTier::from_model_name("Qwen3.5-0.6B"), ModelTier::Tiny);
    assert_eq!(ModelTier::from_model_name("Qwen3.5-9B"), ModelTier::Medium);
    assert_eq!(ModelTier::from_model_name("Llama-3.2-70B"), ModelTier::Large);
}

#[test]
fn tier_detection_cloud() {
    assert_eq!(ModelTier::from_model_name("claude-3-5-sonnet"), ModelTier::Large);
    assert_eq!(ModelTier::from_model_name("gpt-4o"), ModelTier::Large);
    assert_eq!(ModelTier::from_model_name("gemini-pro"), ModelTier::Large);
}

#[test]
fn tier_detection_unknown_defaults_medium() {
    assert_eq!(ModelTier::from_model_name("my-custom-model"), ModelTier::Medium);
}

// ── Capability Profile Construction ───────────────────────────────────

#[test]
fn profile_tiny_model() {
    let p = ModelCapabilityProfile::from_model_name("qwen3.5:0.6b");
    assert_eq!(p.tier, ModelTier::Tiny);
    assert_eq!(p.max_tools_per_prompt, 3);
    assert!(p.uses_mcq());
    assert!(!p.multi_step_capable);
    assert!(!p.supports_repair_loop);
    assert_eq!(p.max_agent_steps, 3);
    assert_eq!(p.slot_mode, SlotMode::KeyValue);
    assert!(p.confidence_threshold >= 0.9);
    assert!(!p.llm_nudge_polish);
}

#[test]
fn profile_small_model() {
    let p = ModelCapabilityProfile::from_model_name("qwen3.5:3b");
    assert_eq!(p.tier, ModelTier::Small);
    assert_eq!(p.max_tools_per_prompt, 5);
    assert_eq!(p.tool_call_mode, ToolCallMode::StructuredJSON);
    assert!(p.use_family_routing);
    assert!(!p.multi_step_capable);
    assert!(p.supports_repair_loop);
    assert_eq!(p.max_repair_attempts, 1);
}

#[test]
fn profile_medium_model() {
    let p = ModelCapabilityProfile::from_model_name("qwen3.5:9b");
    assert_eq!(p.tier, ModelTier::Medium);
    assert_eq!(p.max_tools_per_prompt, 8);
    assert_eq!(p.tool_call_mode, ToolCallMode::StructuredJSON);
    assert!(p.use_family_routing);
    assert!(p.multi_step_capable);
    assert!(p.supports_repair_loop);
    assert_eq!(p.max_repair_attempts, 2);
    assert_eq!(p.max_agent_steps, 10);
    assert!(p.llm_nudge_polish);
    assert!(p.can_summarize_freely);
    assert!(p.hallucination_firewall);
    assert_eq!(p.ambient_context_budget, 8192);
}

#[test]
fn profile_large_model() {
    let p = ModelCapabilityProfile::from_model_name("qwen3.5:27b-nothink");
    assert_eq!(p.tier, ModelTier::Large);
    assert_eq!(p.max_tools_per_prompt, 15);
    assert!(p.uses_native_tools());
    assert!(p.multi_step_capable);
    assert_eq!(p.max_agent_steps, 15);
    assert_eq!(p.max_repair_attempts, 3);
    assert!(!p.hallucination_firewall); // large models need less guardrailing
}

#[test]
fn profile_degraded() {
    let d = ModelCapabilityProfile::degraded();
    assert_eq!(d.tier, ModelTier::Tiny);
    assert_eq!(d.max_tools_per_prompt, 3);
    assert_eq!(d.max_effective_context, 2048);
    assert_eq!(d.max_history_turns, 2);
    assert!(!d.supports_repair_loop);
    assert_eq!(d.ambient_context_budget, 0);
}

// ── Generation Config ─────────────────────────────────────────────────

#[test]
fn gen_config_tool_vs_chat() {
    let p = ModelCapabilityProfile::from_model_name("qwen3.5:9b");
    let tool = p.tool_gen_config();
    let chat = p.chat_gen_config();

    // Tool calling should use lower temperature
    assert!(tool.temperature < chat.temperature);
    assert_eq!(tool.max_tokens, p.max_generation_tokens);
}

// ── Tool Family Routing ───────────────────────────────────────────────

#[test]
fn family_route_email() {
    let families = ToolFamily::route_query("check my email inbox for unread messages");
    assert!(!families.is_empty());
    assert_eq!(families[0].0, ToolFamily::Communicate);
}

#[test]
fn family_route_calendar() {
    let families = ToolFamily::route_query("what's on my calendar tomorrow");
    assert!(!families.is_empty());
    assert_eq!(families[0].0, ToolFamily::Schedule);
}

#[test]
fn family_route_weather() {
    let families = ToolFamily::route_query("will it rain today");
    assert!(!families.is_empty());
    assert_eq!(families[0].0, ToolFamily::World);
}

#[test]
fn family_route_browse() {
    let families = ToolFamily::route_query("search the web for Rust async patterns");
    assert!(!families.is_empty());
    assert_eq!(families[0].0, ToolFamily::Browse);
}

#[test]
fn family_route_files() {
    let families = ToolFamily::route_query("read the config file in /etc");
    assert!(!families.is_empty());
    assert_eq!(families[0].0, ToolFamily::Files);
}

#[test]
fn family_route_memory() {
    let families = ToolFamily::route_query("what did I say about the trip");
    assert!(!families.is_empty());
    // "what did" matches Remember family
    assert_eq!(families[0].0, ToolFamily::Remember);
}

#[test]
fn family_route_system() {
    let families = ToolFamily::route_query("set a reminder for 3pm");
    assert!(!families.is_empty());
    assert_eq!(families[0].0, ToolFamily::System);
}

#[test]
fn family_route_delegate() {
    let families = ToolFamily::route_query("do these three things simultaneously");
    assert!(!families.is_empty());
    assert_eq!(families[0].0, ToolFamily::Delegate);
}

#[test]
fn family_no_match_generic() {
    let families = ToolFamily::route_query("hello how are you");
    assert!(families.is_empty());
}

#[test]
fn family_tools_coverage() {
    // Every family should have at least one tool
    for family in ToolFamily::ALL {
        assert!(!family.tools().is_empty(), "{} has no tools", family);
        assert!(!family.keywords().is_empty(), "{} has no keywords", family);
    }
}

#[test]
fn family_best_for_query() {
    assert_eq!(ToolFamily::best_for_query("send email to Bob"), Some(ToolFamily::Communicate));
    assert_eq!(ToolFamily::best_for_query("browse this website for me"), Some(ToolFamily::Browse));
    assert_eq!(ToolFamily::best_for_query("hello"), None);
}

// ── Profile Summary ──────────────────────────────────────────────────

#[test]
fn profile_summary_contains_key_info() {
    let p = ModelCapabilityProfile::from_model_name("qwen3.5:9b");
    let s = p.summary();
    assert!(s.contains("medium"), "Summary: {}", s);
    assert!(s.contains("9.0B"), "Summary: {}", s);
    assert!(s.contains("StructuredJSON"), "Summary: {}", s);
    assert!(s.contains("family_routing=true"), "Summary: {}", s);
}

// ── ModelTier Ordering ───────────────────────────────────────────────

#[test]
fn tier_ordering() {
    assert!(ModelTier::Tiny < ModelTier::Small);
    assert!(ModelTier::Small < ModelTier::Medium);
    assert!(ModelTier::Medium < ModelTier::Large);
}

// ── Adaptive Tool Selection Integration ──────────────────────────────

#[test]
fn medium_profile_limits_tools() {
    // A 9B model should get max 8 tools per prompt
    let p = ModelCapabilityProfile::from_model_name("qwen3.5:9b");
    assert_eq!(p.max_tools_per_prompt, 8);
    assert!(p.use_family_routing);

    // Family routing for "check email" should return Communicate family tools
    let families = ToolFamily::route_query("check my email inbox");
    assert_eq!(families[0].0, ToolFamily::Communicate);

    // The family should have bounded tools
    let email_tools = ToolFamily::Communicate.tools();
    assert!(email_tools.len() <= 10, "Communicate family has {} tools", email_tools.len());
}

#[test]
fn large_profile_gets_more_tools() {
    let large = ModelCapabilityProfile::from_model_name("qwen3.5:27b-nothink");
    let medium = ModelCapabilityProfile::from_model_name("qwen3.5:9b");

    assert!(large.max_tools_per_prompt > medium.max_tools_per_prompt);
    assert!(large.max_agent_steps > medium.max_agent_steps);
    assert!(large.max_effective_context > medium.max_effective_context);
}

#[test]
fn tiny_profile_uses_mcq() {
    let p = ModelCapabilityProfile::from_model_name("qwen3.5:0.6b");
    assert!(p.uses_mcq());
    assert!(!p.use_family_routing); // MCQ already narrows choices
    assert!(!p.multi_step_capable);
    assert!(!p.supports_repair_loop);
}

#[test]
fn profile_adapts_gen_config_per_tier() {
    let tiny = ModelCapabilityProfile::from_model_name("qwen3.5:0.6b");
    let large = ModelCapabilityProfile::from_model_name("qwen3.5:27b-nothink");

    let tiny_cfg = tiny.tool_gen_config();
    let large_cfg = large.tool_gen_config();

    // Tiny models should get fewer max tokens
    assert!(tiny_cfg.max_tokens < large_cfg.max_tokens);
    // Tiny models should use lower temperature for tool calling
    assert!(tiny_cfg.temperature <= large_cfg.temperature);
}
