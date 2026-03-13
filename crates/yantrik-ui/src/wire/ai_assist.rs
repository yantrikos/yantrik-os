//! Reusable AI assist helper for app-level AI integration.
//!
//! Provides a standard pattern for sending prompts to the companion
//! and streaming responses back to Slint UI properties. Used by
//! System Monitor, Package Manager, Container Manager, Notes, etc.
//!
//! Pattern: App builds a prompt → calls `ai_request()` → Timer polls
//! channel → response accumulates in a Slint string property → done.

use std::cell::RefCell;
use std::rc::Rc;
use std::sync::Arc;
use std::time::Duration;

use crossbeam_channel::Receiver;
use slint::{ComponentHandle, SharedString, Timer, TimerMode};

use crate::App;
use crate::bridge::CompanionBridge;

/// State for an in-progress AI assist request.
/// Shared via Rc<RefCell<>> within a single wire module.
pub struct AiAssistState {
    pub timer: Option<Timer>,
    pub is_working: bool,
}

impl AiAssistState {
    pub fn new() -> Rc<RefCell<Self>> {
        Rc::new(RefCell::new(Self {
            timer: None,
            is_working: false,
        }))
    }
}

/// Configuration for an AI assist request.
pub struct AiAssistRequest {
    /// The full prompt to send to the companion.
    pub prompt: String,
    /// Timeout in seconds before giving up.
    pub timeout_secs: u64,
    /// Callback to set the "is working" flag on the UI.
    pub set_working: Box<dyn Fn(&App, bool) + 'static>,
    /// Callback to set the response text on the UI.
    pub set_response: Box<dyn Fn(&App, &str) + 'static>,
    /// Callback to get the current accumulated response from the UI.
    pub get_response: Box<dyn Fn(&App) -> String + 'static>,
}

/// Fire an AI assist request. Sends the prompt to the companion bridge,
/// starts a polling timer, and streams tokens into the UI.
///
/// Returns immediately — the timer handles async completion.
pub fn ai_request(
    ui_weak: &slint::Weak<App>,
    bridge: &Arc<CompanionBridge>,
    state: &Rc<RefCell<AiAssistState>>,
    request: AiAssistRequest,
) {
    // Cancel any in-progress request
    {
        let mut s = state.borrow_mut();
        s.timer = None;
        s.is_working = true;
    }

    if let Some(ui) = ui_weak.upgrade() {
        (request.set_working)(&ui, true);
        (request.set_response)(&ui, "");
    }

    let token_rx = bridge.send_message(request.prompt);
    let weak = ui_weak.clone();
    let state_clone = state.clone();
    let start_time = std::time::Instant::now();
    let timeout = Duration::from_secs(request.timeout_secs);

    let timer = Timer::default();
    timer.start(TimerMode::Repeated, Duration::from_millis(16), move || {
        let mut done = false;

        // Drain available tokens
        while let Ok(token) = token_rx.try_recv() {
            if token == "__DONE__" {
                done = true;
                break;
            }
            if token == "__REPLACE__" {
                if let Some(ui) = weak.upgrade() {
                    (request.set_response)(&ui, "");
                }
                continue;
            }
            // Skip internal markers
            if token.starts_with("__") && token.ends_with("__") {
                continue;
            }
            if let Some(ui) = weak.upgrade() {
                let current = (request.get_response)(&ui);
                let mut updated = format!("{}{}", current, token);
                // Strip [Using ...] tool markers
                while let Some(start) = updated.find("[Using ") {
                    if let Some(end) = updated[start..].find("...]") {
                        updated = format!("{}{}", &updated[..start], &updated[start + end + 4..]);
                    } else {
                        break;
                    }
                }
                let trimmed = updated.trim_start_matches('\n').to_string();
                (request.set_response)(&ui, &trimmed);
            }
        }

        // Timeout check
        if !done && start_time.elapsed() > timeout {
            if let Some(ui) = weak.upgrade() {
                if (request.get_response)(&ui).is_empty() {
                    (request.set_response)(&ui, "AI is busy — try again later.");
                }
                (request.set_working)(&ui, false);
            }
            state_clone.borrow_mut().timer = None;
            state_clone.borrow_mut().is_working = false;
            return;
        }

        if done {
            if let Some(ui) = weak.upgrade() {
                (request.set_working)(&ui, false);
            }
            state_clone.borrow_mut().timer = None;
            state_clone.borrow_mut().is_working = false;
        }
    });

    state.borrow_mut().timer = Some(timer);
}

/// Build a system analysis prompt with structured context.
pub fn system_analysis_prompt(context: &str, question: &str) -> String {
    format!(
        "You are a system administrator assistant. Analyze the following system data and provide a clear, concise explanation.\n\n\
         ## System Data\n{}\n\n\
         ## Question\n{}\n\n\
         Be direct and practical. Use plain language. If something looks abnormal, explain why and what the user can do about it. \
         Keep your response under 200 words.",
        context, question
    )
}

/// Build a package explanation prompt.
pub fn package_explain_prompt(name: &str, info: &str) -> String {
    format!(
        "Explain this Linux package in 2-3 sentences for a power user:\n\n\
         Package: {}\n{}\n\n\
         What does it do? When would someone use it? Are there common alternatives?",
        name, info
    )
}

/// Build an intent-to-package prompt for the package manager.
pub fn intent_to_package_prompt(intent: &str) -> String {
    format!(
        "The user needs a tool/package on Alpine Linux (apk). Their request: \"{}\"\n\n\
         Suggest 1-3 specific Alpine Linux packages that match this need. \
         For each: package name, one-line description, and install command.\n\
         Format as a short list. Only suggest real packages from the Alpine repository.",
        intent
    )
}

/// Build a container log analysis prompt.
pub fn container_log_prompt(container_name: &str, logs: &str) -> String {
    format!(
        "Analyze these container logs and provide a brief diagnosis:\n\n\
         Container: {}\n\
         ```\n{}\n```\n\n\
         1. What is the container doing?\n\
         2. Are there any errors or warnings? If so, what do they mean?\n\
         3. Any suggested actions?\n\n\
         Be concise — 3-5 sentences max.",
        container_name, logs
    )
}

/// Build a note structuring prompt.
pub fn note_structure_prompt(content: &str) -> String {
    format!(
        "Restructure this note into clean, organized markdown. Preserve all information but improve organization:\n\n\
         ```\n{}\n```\n\n\
         Rules:\n\
         - Add a clear # heading if missing\n\
         - Use ## subheadings to group related content\n\
         - Convert loose text to bullet points where appropriate\n\
         - Extract any action items into a ## Action Items section\n\
         - Extract any decisions into a ## Decisions section (if any)\n\
         - Keep the original meaning and tone\n\
         - Output ONLY the restructured markdown, no explanation",
        content
    )
}

/// Build a note summarization prompt.
pub fn note_summarize_prompt(content: &str) -> String {
    format!(
        "Summarize this note concisely:\n\n\
         ```\n{}\n```\n\n\
         Provide:\n\
         - Key points (3-5 bullets)\n\
         - Any action items or deadlines mentioned\n\
         - Any decisions recorded\n\n\
         Keep it under 100 words. Output ONLY the summary.",
        content
    )
}

/// Build a network configuration analysis prompt.
pub fn network_analysis_prompt(context: &str) -> String {
    format!(
        "You are a network administrator assistant. Analyze this network configuration:\n\n\
         {}\n\n\
         Provide a brief assessment: Is everything normal? Any security concerns? \
         Any optimization suggestions? Keep it under 150 words.",
        context
    )
}

/// Build a weather insights prompt.
pub fn weather_insights_prompt(context: &str) -> String {
    format!(
        "Based on this weather data, provide practical advice for today:\n\n\
         {}\n\n\
         What should the user know? Should they bring an umbrella, wear warm clothes, \
         or be aware of any weather-related risks? Keep it to 2-3 sentences, \
         conversational and helpful.",
        context
    )
}

/// Build a download status analysis prompt.
pub fn download_analysis_prompt(context: &str) -> String {
    format!(
        "Analyze these download activities:\n\n\
         {}\n\n\
         Summarize: how many downloads, any failures, overall speed. \
         If there are failed downloads, suggest what might have gone wrong. \
         Keep it under 100 words.",
        context
    )
}

/// Build a device/hardware analysis prompt.
pub fn device_analysis_prompt(context: &str) -> String {
    format!(
        "You are a hardware specialist. Analyze this device information:\n\n\
         {}\n\n\
         Briefly explain what key hardware is present, note anything unusual, \
         and mention if any drivers or devices need attention. Keep it under 150 words.",
        context
    )
}

/// Build a code snippet explanation prompt.
pub fn snippet_explain_prompt(title: &str, language: &str, code: &str) -> String {
    format!(
        "Explain this code snippet briefly:\n\n\
         Title: {}\nLanguage: {}\n```{}\n{}\n```\n\n\
         What does it do? Any notable patterns or potential issues? Keep it under 100 words.",
        title, language, language, code
    )
}

/// Build a music track info prompt.
pub fn music_info_prompt(context: &str) -> String {
    format!(
        "Based on this music playback info:\n\n\
         {}\n\n\
         Share a brief interesting fact about this artist or genre, \
         or suggest similar music. Keep it to 2-3 sentences.",
        context
    )
}

/// Build an image description prompt.
pub fn image_describe_prompt(filename: &str) -> String {
    format!(
        "The user is viewing an image file: \"{}\"\n\n\
         Based on the filename, provide any insights about what this image might contain, \
         its format, or useful metadata. If the filename is descriptive, explain what it suggests. \
         Keep it to 2-3 sentences.",
        filename
    )
}

/// Build a calendar insights prompt.
pub fn calendar_insights_prompt(context: &str) -> String {
    format!(
        "Look at this calendar data and provide a brief scheduling insight:\n\n\
         {}\n\n\
         Any scheduling conflicts? Is the day overbooked or free? \
         Any suggestions for the user? Keep it to 2-3 sentences.",
        context
    )
}

/// Build a permission/security audit prompt.
pub fn permission_analysis_prompt(context: &str) -> String {
    format!(
        "You are a Linux security auditor. Review this permission data:\n\n\
         {}\n\n\
         Identify any security concerns: SUID files that shouldn't be SUID, \
         world-writable files in sensitive locations, users with excessive privileges. \
         Be specific and actionable. Keep it under 150 words.",
        context
    )
}

// ── Office Suite AI Prompts ──

/// Build a natural-language to formula conversion prompt for ySheets.
pub fn sheet_formula_prompt(description: &str, context: &str) -> String {
    format!(
        "You are a spreadsheet formula expert. Convert this natural language request into a spreadsheet formula.\n\n\
         ## Request\n{}\n\n\
         ## Current Sheet Context\n{}\n\n\
         Rules:\n\
         - Output ONLY the formula starting with =\n\
         - Use standard functions: SUM, AVERAGE, COUNT, IF, VLOOKUP, SUMIF, COUNTIF, MAX, MIN, etc.\n\
         - Cell references use A1 notation (e.g., A1, B2:B100)\n\
         - If the request is ambiguous, choose the most common interpretation\n\
         - No explanation, just the formula",
        description, context
    )
}

/// Build a spreadsheet data analysis prompt.
pub fn sheet_analyze_prompt(data_summary: &str) -> String {
    format!(
        "Analyze this spreadsheet data and provide insights:\n\n\
         {}\n\n\
         Provide:\n\
         - Key patterns or trends you notice\n\
         - Any outliers or anomalies\n\
         - Suggestions for useful calculations or charts\n\
         - Data quality issues (if any)\n\n\
         Keep it concise — under 150 words. Be specific with cell references when possible.",
        data_summary
    )
}

/// Build a chart suggestion prompt for ySheets.
pub fn sheet_chart_prompt(data_summary: &str) -> String {
    format!(
        "Based on this spreadsheet data, suggest the best chart type and configuration:\n\n\
         {}\n\n\
         Recommend:\n\
         - Chart type (bar, line, pie, scatter, area)\n\
         - Which columns to use for labels vs values\n\
         - A good title for the chart\n\
         - Any formatting tips\n\n\
         Keep it under 100 words.",
        data_summary
    )
}

/// Build a document drafting prompt for yDoc.
pub fn doc_draft_prompt(instruction: &str, existing_content: &str) -> String {
    let context = if existing_content.is_empty() {
        String::new()
    } else {
        format!(
            "\n\n## Existing Document Content\n```\n{}\n```\n\nBuild on or continue from the existing content.",
            if existing_content.len() > 2000 { &existing_content[..2000] } else { existing_content }
        )
    };
    format!(
        "You are a professional writer. Draft content based on this instruction:\n\n\
         ## Instruction\n{}{}\n\n\
         Rules:\n\
         - Write in clean markdown format\n\
         - Use appropriate headings, bullet points, and structure\n\
         - Match the tone and style of existing content if present\n\
         - Be thorough but concise\n\
         - Output ONLY the drafted content, no meta-commentary",
        instruction, context
    )
}

/// Build a document summarization prompt for yDoc.
pub fn doc_summarize_prompt(content: &str) -> String {
    format!(
        "Summarize this document concisely:\n\n\
         ```\n{}\n```\n\n\
         Provide:\n\
         - Executive summary (2-3 sentences)\n\
         - Key points (3-5 bullets)\n\
         - Action items (if any)\n\
         - Decisions or conclusions\n\n\
         Keep it under 150 words. Output ONLY the summary.",
        if content.len() > 4000 { &content[..4000] } else { content }
    )
}

/// Build a document improvement prompt for yDoc.
pub fn doc_improve_prompt(content: &str) -> String {
    format!(
        "Review and improve this document. Suggest specific changes:\n\n\
         ```\n{}\n```\n\n\
         Provide:\n\
         - Clarity improvements (reword confusing sentences)\n\
         - Structure suggestions (better headings, organization)\n\
         - Grammar and style fixes\n\
         - Missing information that should be added\n\n\
         Be specific — reference exact passages. Keep it under 200 words.",
        if content.len() > 4000 { &content[..4000] } else { content }
    )
}

/// Build a document translation prompt for yDoc.
pub fn doc_translate_prompt(content: &str, target_language: &str) -> String {
    format!(
        "Translate this document to {}. Preserve markdown formatting:\n\n\
         ```\n{}\n```\n\n\
         Rules:\n\
         - Maintain all headings, lists, and structure\n\
         - Keep technical terms that are commonly used in English\n\
         - Preserve code blocks unchanged\n\
         - Output ONLY the translated content",
        target_language,
        if content.len() > 4000 { &content[..4000] } else { content }
    )
}

/// Build a JSON-based deck generation prompt.
pub fn pres_generate_deck_json_prompt(topic: &str, instruction: &str) -> String {
    let extra = if instruction.is_empty() { String::new() } else { format!("\nAdditional instructions: {}", instruction) };
    format!(
        r#"Generate a presentation deck on this topic. Return ONLY valid JSON, no other text.{}

Topic: {}

Output JSON schema:
{{
  "title": "deck title",
  "slides": [
    {{
      "title": "slide title",
      "bullets": ["bullet 1", "bullet 2", "bullet 3"],
      "speaker_notes": "what to say for this slide",
      "slide_type": "title_slide|title_and_content|two_column|section_header|blank"
    }}
  ]
}}

Rules:
- Generate 5-8 slides
- First slide: title_slide with subtitle in bullets[0]
- Last slide: summary or Q&A
- Each slide: 3-6 bullets, max 15 words each
- Speaker notes: 2-3 conversational sentences, not a script
- No markdown in bullets
- Output ONLY the JSON object"#,
        extra, topic
    )
}

/// Build a JSON-based slide improvement prompt.
pub fn pres_improve_slide_json_prompt(title: &str, bullets: &[String], notes: &str, deck_title: &str, instruction: &str) -> String {
    let bullets_json: Vec<String> = bullets.iter().map(|b| format!("\"{}\"", b.replace('"', "\\\""))).collect();
    let extra = if instruction.is_empty() { String::new() } else { format!("\nInstruction: {}", instruction) };
    format!(
        r#"Improve this presentation slide. Return ONLY valid JSON, no other text.{}

Deck: {}
Current slide:
{{
  "title": "{}",
  "bullets": [{}],
  "speaker_notes": "{}"
}}

Output the improved slide as JSON:
{{
  "title": "improved title",
  "bullets": ["improved bullet 1", "improved bullet 2"],
  "speaker_notes": "improved notes",
  "slide_type": "title_and_content"
}}

Rules:
- Make title more engaging and specific
- Simplify bullets: max 6, each under 15 words
- No markdown or special formatting in bullets
- Speaker notes: 2-3 conversational sentences
- Preserve the core meaning
- Output ONLY the JSON object"#,
        extra, deck_title, title.replace('"', "\\\""),
        bullets_json.join(", "),
        notes.replace('"', "\\\"")
    )
}

/// Build a JSON-based speaker notes generation prompt.
pub fn pres_generate_notes_json_prompt(title: &str, bullets: &[String], deck_title: &str) -> String {
    let bullets_json: Vec<String> = bullets.iter().map(|b| format!("\"{}\"", b.replace('"', "\\\""))).collect();
    format!(
        r#"Generate speaker notes for this slide. Return ONLY valid JSON, no other text.

Deck: {}
Slide:
{{
  "title": "{}",
  "bullets": [{}]
}}

Output JSON:
{{
  "title": "{}",
  "bullets": [{}],
  "speaker_notes": "detailed speaker notes here",
  "slide_type": "title_and_content"
}}

Rules:
- Speaker notes: 3-5 conversational sentences
- Expand on bullet points with examples or context
- Include transition to next topic
- Do NOT change title or bullets — copy them exactly
- Output ONLY the JSON object"#,
        deck_title, title.replace('"', "\\\""),
        bullets_json.join(", "),
        title.replace('"', "\\\""),
        bullets_json.join(", ")
    )
}

/// Build a JSON-based simplify prompt.
pub fn pres_simplify_json_prompt(title: &str, bullets: &[String], notes: &str, audience: &str) -> String {
    let bullets_json: Vec<String> = bullets.iter().map(|b| format!("\"{}\"", b.replace('"', "\\\""))).collect();
    let aud = if audience.is_empty() { "general audience" } else { audience };
    format!(
        r#"Simplify this slide for the target audience. Return ONLY valid JSON, no other text.

Target audience: {}
Current slide:
{{
  "title": "{}",
  "bullets": [{}],
  "speaker_notes": "{}"
}}

Output the simplified slide as JSON:
{{
  "title": "simplified title",
  "bullets": ["simplified bullet 1", "simplified bullet 2"],
  "speaker_notes": "simplified notes",
  "slide_type": "title_and_content"
}}

Rules:
- Reduce jargon for the target audience
- Use shorter, simpler sentences
- Max 5 bullets, each under 12 words
- Speaker notes should explain in plain language
- Output ONLY the JSON object"#,
        aud, title.replace('"', "\\\""),
        bullets_json.join(", "),
        notes.replace('"', "\\\"")
    )
}

/// Build a JSON-based split slide prompt.
pub fn pres_split_slide_json_prompt(title: &str, bullets: &[String], notes: &str) -> String {
    let bullets_json: Vec<String> = bullets.iter().map(|b| format!("\"{}\"", b.replace('"', "\\\""))).collect();
    format!(
        r#"This slide has too much content. Split it into 2-3 focused slides. Return ONLY valid JSON, no other text.

Current slide:
{{
  "title": "{}",
  "bullets": [{}],
  "speaker_notes": "{}"
}}

Output JSON — an object with a "slides" array:
{{
  "slides": [
    {{
      "title": "first sub-topic title",
      "bullets": ["bullet 1", "bullet 2"],
      "speaker_notes": "notes for first slide",
      "slide_type": "title_and_content"
    }},
    {{
      "title": "second sub-topic title",
      "bullets": ["bullet 1", "bullet 2"],
      "speaker_notes": "notes for second slide",
      "slide_type": "title_and_content"
    }}
  ]
}}

Rules:
- Split into 2-3 slides, each with a clear focus
- Each slide: 3-5 bullets max
- Preserve all original content
- Add speaker notes per slide
- Output ONLY the JSON object"#,
        title.replace('"', "\\\""),
        bullets_json.join(", "),
        notes.replace('"', "\\\"")
    )
}

/// Build a JSON-based layout suggestion prompt.
pub fn pres_suggest_layout_json_prompt(title: &str, bullets: &[String], notes: &str) -> String {
    let bullets_json: Vec<String> = bullets.iter().map(|b| format!("\"{}\"", b.replace('"', "\\\""))).collect();
    format!(
        r#"Suggest the best slide layout for this content. Return ONLY valid JSON, no other text.

Available layouts: title_slide, title_and_content, two_column, section_header, blank, image_and_text

Current slide:
{{
  "title": "{}",
  "bullets": [{}],
  "speaker_notes": "{}"
}}

Output the slide with suggested layout:
{{
  "title": "{}",
  "bullets": [{}],
  "speaker_notes": "{}",
  "slide_type": "suggested_layout_name"
}}

Rules:
- Choose the most appropriate layout from the available options
- Do NOT change title, bullets, or notes — copy them exactly
- Only change the slide_type field
- Output ONLY the JSON object"#,
        title.replace('"', "\\\""),
        bullets_json.join(", "),
        notes.replace('"', "\\\""),
        title.replace('"', "\\\""),
        bullets_json.join(", "),
        notes.replace('"', "\\\"")
    )
}

/// Build a JSON-based "structure text into slides" prompt.
pub fn pres_structure_text_json_prompt(text: &str, instruction: &str) -> String {
    let extra = if instruction.is_empty() { String::new() } else { format!("\nAdditional instructions: {}", instruction) };
    format!(
        r#"Structure this text into presentation slides. Return ONLY valid JSON, no other text.{}

Source text:
{}

Output JSON schema:
{{
  "title": "presentation title",
  "slides": [
    {{
      "title": "slide title",
      "bullets": ["key point 1", "key point 2"],
      "speaker_notes": "expanded context",
      "slide_type": "title_slide|title_and_content|section_header"
    }}
  ]
}}

Rules:
- One clear idea per slide
- First slide: title_slide with overview
- 3-8 slides depending on content length
- Each slide: 3-6 bullets, max 15 words each
- Capture ALL key points from source text
- Speaker notes provide expanded detail from source
- Output ONLY the JSON object"#,
        extra, if text.len() > 4000 { &text[..4000] } else { text }
    )
}

/// Build a batch speaker notes generation prompt.
pub fn pres_generate_all_notes_json_prompt(slides: &[(String, Vec<String>)]) -> String {
    let slides_json: Vec<String> = slides.iter().map(|(title, bullets)| {
        let bj: Vec<String> = bullets.iter().map(|b| format!("\"{}\"", b.replace('"', "\\\""))).collect();
        format!("    {{ \"title\": \"{}\", \"bullets\": [{}] }}", title.replace('"', "\\\""), bj.join(", "))
    }).collect();
    format!(
        r#"Generate speaker notes for ALL slides in this deck. Return ONLY valid JSON, no other text.

Slides:
[
{}
]

Output JSON — a "slides" array with notes added:
{{
  "slides": [
    {{
      "title": "exact slide title",
      "bullets": ["exact bullets"],
      "speaker_notes": "generated speaker notes",
      "slide_type": "title_and_content"
    }}
  ]
}}

Rules:
- Generate 2-3 sentence speaker notes per slide
- Notes should expand on bullets with examples/context
- Include transitions between slides
- Do NOT change titles or bullets — copy them exactly
- Output ONLY the JSON object"#,
        slides_json.join(",\n")
    )
}

/// Generic office AI prompt for free-form questions about a document/sheet/presentation.
pub fn office_freeform_prompt(app_type: &str, context: &str, question: &str) -> String {
    format!(
        "You are an AI assistant for a {} application. The user has a question.\n\n\
         ## Current Content Context\n{}\n\n\
         ## User Question\n{}\n\n\
         Provide a helpful, concise answer. If suggesting content changes, format them clearly \
         so the user can apply them. Keep it under 200 words.",
        app_type,
        if context.len() > 3000 { &context[..3000] } else { context },
        question
    )
}

/// Build a contextual insights prompt — tells the companion to use recall to find related items.
/// This prompt is designed to go through the full agent loop so the companion can
/// use its recall, email_check, calendar, and browse tools to find context.
pub fn contextual_insights_prompt(content: &str, app_type: &str) -> String {
    format!(
        "The user is working on a {} and wants contextual insights. \
         First, use the recall tool to search for related items in memory \
         (emails, calendar events, previous conversations, notes). \
         Search for key terms from the content below.\n\n\
         ## Current Content\n{}\n\n\
         After searching memory, provide:\n\
         1. **Source**: What is this likely related to? (meeting, project, report, email thread)\n\
         2. **Related items**: What related emails, calendar events, or docs were found\n\
         3. **Missing**: What information should be added based on context\n\
         4. **Tips**: 2-3 specific actionable suggestions\n\n\
         Be specific and reference actual items you found in memory. Keep it under 200 words.",
        app_type,
        if content.len() > 3000 { &content[..3000] } else { content }
    )
}
