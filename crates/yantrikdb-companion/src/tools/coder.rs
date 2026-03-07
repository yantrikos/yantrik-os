//! YantrikCoder — a powerful code execution engine modeled after Claude Code.
//!
//! Tools:
//!   code_execute  — Run code inline without saving (quick computations, one-offs)
//!   script_write  — Create/update persistent scripts in managed workspace
//!   script_run    — Execute scripts with auto-deps, timeout-kill, file detection
//!   script_patch  — Edit specific lines in a script (like a code editor)
//!   script_list   — List all scripts with metadata
//!   script_read   — Read script contents
//!   script_delete — Remove script + registry entry
//!
//! Architecture:
//!   ~/.config/yantrik/scripts/        — managed script workspace
//!   ~/.config/yantrik/scripts/output/ — generated files (images, CSVs, HTML)
//!   ~/.config/yantrik/scripts/registry.json — metadata tracking
//!
//! Design principles (from Claude Code):
//!   1. ACT, don't advise — execute code, don't tell the user to run it
//!   2. Self-healing — on failure, return structured errors for automatic retry
//!   3. File awareness — detect generated outputs (images, charts, data files)
//!   4. Real timeouts — kill runaway processes after deadline
//!   5. Auto-deps — pip, npm, gem installed before execution

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};

use super::{Tool, ToolContext, ToolRegistry, PermissionLevel};

const SCRIPTS_DIR: &str = "/home/yantrik/.config/yantrik/scripts";
const OUTPUT_DIR: &str = "/home/yantrik/.config/yantrik/scripts/output";
const REGISTRY_FILE: &str = "/home/yantrik/.config/yantrik/scripts/registry.json";
const MAX_OUTPUT: usize = 6000;
const DEFAULT_TIMEOUT: u64 = 120;
const MAX_TIMEOUT: u64 = 600;

pub fn register(reg: &mut ToolRegistry) {
    reg.register(Box::new(CodeExecuteTool));
    reg.register(Box::new(ScriptWriteTool));
    reg.register(Box::new(ScriptRunTool));
    reg.register(Box::new(ScriptPatchTool));
    reg.register(Box::new(ScriptListTool));
    reg.register(Box::new(ScriptReadTool));
    reg.register(Box::new(ScriptDeleteTool));
}

// ═══════════════════════════════════════════════════════════════════
// Registry — persistent metadata for all managed scripts
// ═══════════════════════════════════════════════════════════════════

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
struct ScriptEntry {
    name: String,
    filename: String,
    language: String,
    description: String,
    created_at: f64,
    updated_at: f64,
    last_run: Option<f64>,
    last_run_status: Option<String>,
    last_error: Option<String>,
    run_count: u32,
    success_count: u32,
    #[serde(default)]
    dependencies: Vec<String>,
    #[serde(default)]
    generated_files: Vec<String>,
    #[serde(default)]
    tags: Vec<String>,
}

fn scripts_dir() -> PathBuf { PathBuf::from(SCRIPTS_DIR) }
fn output_dir() -> PathBuf { PathBuf::from(OUTPUT_DIR) }
fn now_ts() -> f64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_secs_f64()
}

fn load_registry() -> HashMap<String, ScriptEntry> {
    let path = Path::new(REGISTRY_FILE);
    if !path.exists() { return HashMap::new(); }
    std::fs::read_to_string(path)
        .ok()
        .and_then(|d| serde_json::from_str(&d).ok())
        .unwrap_or_default()
}

fn save_registry(registry: &HashMap<String, ScriptEntry>) {
    let _ = std::fs::create_dir_all(scripts_dir());
    if let Ok(data) = serde_json::to_string_pretty(registry) {
        let _ = std::fs::write(REGISTRY_FILE, data);
    }
}

fn sanitize_name(name: &str) -> String {
    name.chars()
        .map(|c| if c.is_alphanumeric() || c == '_' || c == '-' { c } else { '_' })
        .collect()
}

// ═══════════════════════════════════════════════════════════════════
// Language detection & package management
// ═══════════════════════════════════════════════════════════════════

fn detect_language(filename: &str, content: &str) -> String {
    // Check shebang first
    let first_line = content.lines().next().unwrap_or("");
    if first_line.contains("python") { return "python".into(); }
    if first_line.contains("node") { return "node".into(); }
    if first_line.contains("ruby") { return "ruby".into(); }
    if first_line.contains("bash") || first_line.contains("/bin/sh") { return "bash".into(); }

    // Then extension
    if filename.ends_with(".py") { return "python".into(); }
    if filename.ends_with(".js") || filename.ends_with(".mjs") { return "node".into(); }
    if filename.ends_with(".ts") { return "typescript".into(); }
    if filename.ends_with(".rb") { return "ruby".into(); }
    if filename.ends_with(".sh") { return "bash".into(); }
    if filename.ends_with(".html") || filename.ends_with(".htm") { return "html".into(); }

    // Content heuristics
    if content.contains("import ") && (content.contains("def ") || content.contains("print(")) {
        return "python".into();
    }
    if content.contains("const ") || content.contains("require(") || content.contains("console.log") {
        return "node".into();
    }

    "bash".into()
}

fn interpreter_for(language: &str) -> Option<&'static str> {
    match language {
        "python" => Some("python3"),
        "bash" | "sh" => Some("bash"),
        "node" | "javascript" => Some("node"),
        "typescript" => Some("npx"),
        "ruby" => Some("ruby"),
        "html" => None, // not executed via interpreter
        _ => Some("bash"),
    }
}

fn extension_for(language: &str) -> &'static str {
    match language {
        "python" => ".py",
        "bash" | "sh" => ".sh",
        "node" | "javascript" => ".js",
        "typescript" => ".ts",
        "ruby" => ".rb",
        "html" => ".html",
        _ => ".sh",
    }
}

/// Python standard library modules — don't try to pip install these.
const PYTHON_STDLIB: &[&str] = &[
    "os", "sys", "json", "re", "math", "datetime", "time", "collections",
    "itertools", "functools", "pathlib", "subprocess", "argparse", "logging",
    "csv", "io", "struct", "hashlib", "base64", "random", "string",
    "threading", "multiprocessing", "http", "urllib", "socket", "ssl",
    "typing", "abc", "enum", "dataclasses", "copy", "pprint", "inspect",
    "unittest", "textwrap", "shutil", "glob", "tempfile", "statistics",
    "contextlib", "operator", "heapq", "bisect", "decimal", "fractions",
    "cmath", "array", "queue", "select", "signal", "mmap", "ctypes",
    "sqlite3", "dbm", "gzip", "zipfile", "tarfile", "lzma", "bz2",
    "configparser", "secrets", "hmac", "pickle", "shelve", "marshal",
    "xml", "html", "email", "mailbox", "mimetypes", "encodings",
    "codecs", "unicodedata", "locale", "gettext", "warnings", "traceback",
    "atexit", "gc", "weakref", "types", "importlib", "pkgutil",
    "platform", "sysconfig", "site", "dis", "ast", "symtable",
    "token", "tokenize", "pdb", "profile", "cProfile", "timeit",
    "concurrent", "asyncio", "venv", "ensurepip", "distutils",
    "turtle", "tkinter", "cmd", "code", "readline",
];

/// Well-known pip package name mappings (import name → pip name).
const PIP_NAME_MAP: &[(&str, &str)] = &[
    ("cv2", "opencv-python"),
    ("PIL", "Pillow"),
    ("sklearn", "scikit-learn"),
    ("skimage", "scikit-image"),
    ("bs4", "beautifulsoup4"),
    ("yaml", "pyyaml"),
    ("attr", "attrs"),
    ("dateutil", "python-dateutil"),
    ("dotenv", "python-dotenv"),
    ("gi", "PyGObject"),
    ("wx", "wxPython"),
    ("serial", "pyserial"),
    ("usb", "pyusb"),
    ("Crypto", "pycryptodome"),
    ("jose", "python-jose"),
    ("jwt", "PyJWT"),
    ("magic", "python-magic"),
    ("docx", "python-docx"),
    ("pptx", "python-pptx"),
    ("openpyxl", "openpyxl"),
    ("yfinance", "yfinance"),
];

fn detect_python_deps(content: &str) -> Vec<String> {
    let mut packages = HashSet::new();
    for line in content.lines() {
        let trimmed = line.trim();
        // Skip comments
        if trimmed.starts_with('#') { continue; }

        let module = if let Some(rest) = trimmed.strip_prefix("import ") {
            // Handle "import x, y, z" and "import x as y"
            rest.split(',')
                .filter_map(|p| p.trim().split_whitespace().next())
                .map(|m| m.split('.').next().unwrap_or(""))
                .filter(|m| !m.is_empty())
                .collect::<Vec<_>>()
        } else if let Some(rest) = trimmed.strip_prefix("from ") {
            let m = rest.split_whitespace().next().unwrap_or("").split('.').next().unwrap_or("");
            if m.is_empty() { vec![] } else { vec![m] }
        } else {
            vec![]
        };

        for m in module {
            if !PYTHON_STDLIB.contains(&m) && !m.starts_with('_') {
                // Map to pip name
                let pip_name = PIP_NAME_MAP.iter()
                    .find(|(import, _)| *import == m)
                    .map(|(_, pip)| pip.to_string())
                    .unwrap_or_else(|| m.to_string());
                packages.insert(pip_name);
            }
        }
    }
    let mut sorted: Vec<_> = packages.into_iter().collect();
    sorted.sort();
    sorted
}

fn detect_node_deps(content: &str) -> Vec<String> {
    let mut packages = HashSet::new();
    let builtin = ["fs", "path", "os", "http", "https", "url", "util", "crypto",
                   "stream", "events", "child_process", "cluster", "net", "dns",
                   "readline", "tls", "zlib", "buffer", "querystring", "assert",
                   "process", "v8", "vm", "worker_threads", "perf_hooks"];

    for line in content.lines() {
        let trimmed = line.trim();
        // require('package')
        if let Some(start) = trimmed.find("require(") {
            let rest = &trimmed[start + 8..];
            let quote = rest.chars().next().unwrap_or(' ');
            if quote == '\'' || quote == '"' {
                if let Some(end) = rest[1..].find(quote) {
                    let pkg = &rest[1..1 + end];
                    let base = pkg.split('/').next().unwrap_or(pkg);
                    if !base.starts_with('.') && !builtin.contains(&base) {
                        packages.insert(base.to_string());
                    }
                }
            }
        }
        // import ... from 'package'
        if trimmed.starts_with("import ") {
            if let Some(from_idx) = trimmed.find("from ") {
                let rest = &trimmed[from_idx + 5..];
                let quote = rest.chars().next().unwrap_or(' ');
                if quote == '\'' || quote == '"' {
                    if let Some(end) = rest[1..].find(quote) {
                        let pkg = &rest[1..1 + end];
                        let base = pkg.split('/').next().unwrap_or(pkg);
                        if !base.starts_with('.') && !builtin.contains(&base) {
                            packages.insert(base.to_string());
                        }
                    }
                }
            }
        }
    }
    let mut sorted: Vec<_> = packages.into_iter().collect();
    sorted.sort();
    sorted
}

/// Install dependencies before execution. Returns error message if installation fails.
fn install_deps(language: &str, deps: &[String]) -> Option<String> {
    if deps.is_empty() { return None; }

    let (cmd, args_prefix): (&str, &[&str]) = match language {
        "python" => ("pip3", &["install", "-q", "--break-system-packages"]),
        "node" | "javascript" | "typescript" => ("npm", &["install", "--no-save", "--silent"]),
        "ruby" => ("gem", &["install", "--no-document"]),
        _ => return None,
    };

    tracing::info!(lang = language, deps = ?deps, "Auto-installing dependencies");

    let mut command = std::process::Command::new(cmd);
    command.args(args_prefix);
    command.args(deps);
    command.current_dir(scripts_dir());

    match command.output() {
        Ok(output) if output.status.success() => None,
        Ok(output) => {
            let err = String::from_utf8_lossy(&output.stderr);
            Some(format!("Dependency install failed ({cmd}): {}", err.chars().take(500).collect::<String>()))
        }
        Err(e) => Some(format!("Could not run {cmd}: {e}")),
    }
}

// ═══════════════════════════════════════════════════════════════════
// Execution engine — real timeout, file detection, structured errors
// ═══════════════════════════════════════════════════════════════════

/// Snapshot the output directory before execution so we can detect new files.
fn snapshot_dir(dir: &Path) -> HashSet<PathBuf> {
    walkdir(dir).into_iter().collect()
}

/// Simple recursive directory listing.
fn walkdir(dir: &Path) -> Vec<PathBuf> {
    let mut files = Vec::new();
    if let Ok(entries) = std::fs::read_dir(dir) {
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_file() {
                files.push(path);
            } else if path.is_dir() {
                files.extend(walkdir(&path));
            }
        }
    }
    files
}

/// Detect files generated by script execution.
fn detect_generated_files(before: &HashSet<PathBuf>, scan_dirs: &[&Path]) -> Vec<String> {
    let mut generated = Vec::new();
    for dir in scan_dirs {
        for path in walkdir(dir) {
            if !before.contains(&path) {
                if let Some(s) = path.to_str() {
                    generated.push(s.to_string());
                }
            }
        }
    }
    generated.sort();
    generated
}

/// Classify a generated file for reporting.
fn file_type_label(path: &str) -> &'static str {
    let lower = path.to_lowercase();
    if lower.ends_with(".png") || lower.ends_with(".jpg") || lower.ends_with(".jpeg")
        || lower.ends_with(".gif") || lower.ends_with(".svg") || lower.ends_with(".webp") {
        "image"
    } else if lower.ends_with(".csv") || lower.ends_with(".tsv") {
        "data"
    } else if lower.ends_with(".json") || lower.ends_with(".jsonl") {
        "json"
    } else if lower.ends_with(".html") || lower.ends_with(".htm") {
        "web"
    } else if lower.ends_with(".pdf") {
        "pdf"
    } else if lower.ends_with(".txt") || lower.ends_with(".md") || lower.ends_with(".log") {
        "text"
    } else if lower.ends_with(".xlsx") || lower.ends_with(".xls") {
        "spreadsheet"
    } else {
        "file"
    }
}

/// Parse Python traceback for structured error reporting.
fn parse_python_error(stderr: &str) -> Option<String> {
    let lines: Vec<&str> = stderr.lines().collect();
    if lines.is_empty() { return None; }

    // Find the last "File ..." line and the error line
    let mut file_line = None;
    let mut error_line = None;
    let mut line_num = None;

    for (i, line) in lines.iter().enumerate() {
        if line.trim_start().starts_with("File ") {
            file_line = Some(*line);
            // Extract line number
            if let Some(start) = line.find("line ") {
                let rest = &line[start + 5..];
                if let Some(end) = rest.find(|c: char| !c.is_ascii_digit()) {
                    line_num = rest[..end].parse::<usize>().ok();
                }
            }
        }
        // The actual error is usually the last non-empty line
        if !line.trim().is_empty() {
            error_line = Some(i);
        }
    }

    let error_text = error_line.map(|i| lines[i].trim());

    match (error_text, file_line, line_num) {
        (Some(err), Some(file), Some(ln)) => Some(format!(
            "ERROR_STRUCTURED: {err}\nFILE: {file}\nLINE: {ln}\nFIX_HINT: Edit line {ln} to fix: {err}"
        )),
        (Some(err), _, _) => Some(format!("ERROR_STRUCTURED: {err}")),
        _ => None,
    }
}

/// Parse Node.js error for structured reporting.
fn parse_node_error(stderr: &str) -> Option<String> {
    for line in stderr.lines() {
        let trimmed = line.trim();
        // "at Object.<anonymous> (/path/file.js:10:5)"
        if trimmed.starts_with("at ") && trimmed.contains(':') {
            if let Some(paren) = trimmed.rfind('(') {
                let loc = &trimmed[paren + 1..trimmed.len() - 1];
                return Some(format!("ERROR_LOCATION: {loc}"));
            }
        }
        // "SyntaxError: ..." or "TypeError: ..."
        if trimmed.contains("Error:") && !trimmed.starts_with("at ") {
            return Some(format!("ERROR_STRUCTURED: {trimmed}"));
        }
    }
    None
}

struct ExecutionResult {
    exit_code: i32,
    stdout: String,
    stderr: String,
    duration_secs: f64,
    timed_out: bool,
    generated_files: Vec<String>,
    error_analysis: Option<String>,
    sandboxed: bool,
}

// ── Sandbox modes ──

/// Controls how isolated the script execution environment is.
#[derive(Debug, Clone, Copy, PartialEq)]
enum SandboxMode {
    /// Full isolation: no network, read-only filesystem, PID namespace.
    /// Default for code_execute and script_run.
    Strict,
    /// Filesystem isolation but network allowed.
    /// For web portals, API calls, data fetching scripts.
    Network,
    /// No sandbox. Only used if bwrap unavailable.
    None,
}

impl SandboxMode {
    fn from_str(s: &str) -> Self {
        match s {
            "network" | "web" => Self::Network,
            "none" | "unsafe" => Self::None,
            _ => Self::Strict,
        }
    }
}

/// Check if bubblewrap is available on this system.
fn has_bwrap() -> bool {
    let result = std::process::Command::new("/usr/bin/bwrap")
        .arg("--version")
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .env_remove("LD_PRELOAD") // bwrap is a native binary, doesn't need LD_PRELOAD
        .status();
    match &result {
        Ok(s) => {
            tracing::debug!(success = s.success(), "bwrap availability check");
            s.success()
        }
        Err(e) => {
            tracing::warn!(error = %e, "bwrap not available");
            false
        }
    }
}

/// Build bwrap arguments for sandboxed execution.
fn build_bwrap_args(
    sandbox: SandboxMode,
    workspace: &Path,
    output: &Path,
) -> Vec<String> {
    let mut args: Vec<String> = Vec::new();

    // Read-only system mounts
    for dir in &["/usr", "/lib", "/bin", "/etc"] {
        if Path::new(dir).exists() {
            args.push("--ro-bind".into());
            args.push(dir.to_string());
            args.push(dir.to_string());
        }
    }
    // /sbin for system tools some scripts may need
    if Path::new("/sbin").exists() {
        args.push("--ro-bind".into());
        args.push("/sbin".into());
        args.push("/sbin".into());
    }
    // lib64 symlink (needed on some systems)
    if Path::new("/usr/lib64").exists() {
        args.push("--symlink".into());
        args.push("usr/lib64".into());
        args.push("/lib64".into());
    }

    // Writable workspace (scripts + output)
    args.push("--bind".into());
    args.push(workspace.to_string_lossy().to_string());
    args.push(workspace.to_string_lossy().to_string());
    if output != workspace && output.starts_with(workspace) {
        // output is inside workspace, already covered
    } else if output.exists() {
        args.push("--bind".into());
        args.push(output.to_string_lossy().to_string());
        args.push(output.to_string_lossy().to_string());
    }

    // Writable /tmp for temp files
    args.push("--tmpfs".into());
    args.push("/tmp".into());

    // /dev and /proc (needed for basic operation)
    args.push("--dev".into());
    args.push("/dev".into());
    args.push("--proc".into());
    args.push("/proc".into());

    // PID namespace (always isolate PIDs)
    args.push("--unshare-pid".into());

    // Network isolation
    match sandbox {
        SandboxMode::Strict => {
            args.push("--unshare-net".into());
        }
        SandboxMode::Network => {
            args.push("--share-net".into());
        }
        SandboxMode::None => unreachable!("bwrap not used in None mode"),
    }

    // Die when parent dies (prevent orphan processes)
    args.push("--die-with-parent".into());

    // Resource limits via inner ulimit wrapper
    // (bwrap doesn't set ulimits directly — we wrap the command)

    args.push("--".into());
    args
}

/// Execute a file with real timeout (kill on deadline).
/// Scripts run inside a bubblewrap sandbox by default.
fn execute_script(
    filepath: &Path,
    language: &str,
    args: &str,
    env_vars: &HashMap<String, String>,
    timeout_secs: u64,
    working_dir: Option<&str>,
    sandbox: SandboxMode,
) -> ExecutionResult {
    let interpreter = match interpreter_for(language) {
        Some(i) => i,
        None => {
            return ExecutionResult {
                exit_code: 0,
                stdout: format!("File created: {}", filepath.display()),
                stderr: String::new(),
                duration_secs: 0.0,
                timed_out: false,
                generated_files: vec![filepath.to_string_lossy().to_string()],
                error_analysis: None,
                sandboxed: false,
            };
        }
    };

    // Snapshot directories for file detection
    let out_dir = output_dir();
    let _ = std::fs::create_dir_all(&out_dir);
    let scripts = scripts_dir();
    let before = snapshot_dir(&out_dir);
    let before_scripts = snapshot_dir(&scripts);

    // Decide whether to use bwrap
    let use_bwrap = sandbox != SandboxMode::None && has_bwrap();

    // Build command: either bwrap-wrapped or direct
    let mut cmd = if use_bwrap {
        let bwrap_args = build_bwrap_args(sandbox, &scripts, &out_dir);
        let mut c = std::process::Command::new("/usr/bin/bwrap");
        c.env_remove("LD_PRELOAD"); // bwrap is native, LD_PRELOAD can interfere
        c.args(&bwrap_args);

        // Inside bwrap: ulimit wrapper for resource limits, then interpreter
        // Use sh -c to set ulimits before running the script
        c.arg("sh");
        c.arg("-c");

        // Build the inner command with resource limits
        let mut inner = String::new();
        // Max 512MB virtual memory
        inner.push_str("ulimit -v 524288 2>/dev/null; ");
        // Max 1000 open files
        inner.push_str("ulimit -n 1000 2>/dev/null; ");
        // Max 100 processes (prevent fork bombs)
        inner.push_str("ulimit -u 100 2>/dev/null; ");
        // Max 256MB file size
        inner.push_str("ulimit -f 262144 2>/dev/null; ");
        // No core dumps
        inner.push_str("ulimit -c 0 2>/dev/null; ");

        // The actual interpreter + script + args
        inner.push_str(&format!("exec {} ", interpreter));
        if language == "typescript" {
            inner.push_str("ts-node ");
        }
        inner.push_str(&format!("\"{}\"", filepath.display()));
        if !args.is_empty() {
            inner.push(' ');
            inner.push_str(args);
        }

        c.arg(inner);
        c
    } else {
        let mut c = std::process::Command::new(interpreter);
        if language == "typescript" {
            c.arg("ts-node");
        }
        c.arg(filepath);
        if !args.is_empty() {
            c.args(shell_words::split(args).unwrap_or_else(|_|
                args.split_whitespace().map(String::from).collect()
            ));
        }
        c
    };

    // Working directory (only for non-bwrap — bwrap runs from /)
    if !use_bwrap {
        let work_dir = working_dir
            .map(PathBuf::from)
            .unwrap_or_else(|| scripts.clone());
        cmd.current_dir(&work_dir);
    }

    // Environment: inherit + custom + output dir hint
    cmd.env("YANTRIK_OUTPUT_DIR", out_dir.to_str().unwrap_or("/tmp"));
    cmd.env("YANTRIK_SCRIPTS_DIR", scripts.to_str().unwrap_or("/tmp"));
    cmd.env("HOME", "/tmp"); // prevent writes to real home
    for (k, v) in env_vars {
        cmd.env(k, v);
    }

    cmd.stdout(std::process::Stdio::piped());
    cmd.stderr(std::process::Stdio::piped());

    if use_bwrap {
        tracing::info!(
            sandbox = ?sandbox,
            script = %filepath.display(),
            "Executing in bubblewrap sandbox"
        );
    }

    let start = std::time::Instant::now();

    let mut child = match cmd.spawn() {
        Ok(c) => c,
        Err(e) => {
            return ExecutionResult {
                exit_code: -1,
                stdout: String::new(),
                stderr: format!("Failed to spawn {}: {e}", if use_bwrap { "bwrap" } else { interpreter }),
                duration_secs: 0.0,
                timed_out: false,
                generated_files: vec![],
                error_analysis: Some(format!("SPAWN_ERROR: {} not found. Install it with your package manager.", if use_bwrap { "bubblewrap (bwrap)" } else { interpreter })),
                sandboxed: use_bwrap,
            };
        }
    };

    // Real timeout with kill
    let deadline = std::time::Duration::from_secs(timeout_secs);
    let timed_out;
    let output;

    // Poll for completion with timeout
    loop {
        match child.try_wait() {
            Ok(Some(_status)) => {
                timed_out = false;
                output = child.wait_with_output().ok();
                break;
            }
            Ok(None) => {
                if start.elapsed() > deadline {
                    // Kill the process
                    let _ = child.kill();
                    let _ = child.wait(); // Reap
                    timed_out = true;
                    output = None;
                    break;
                }
                std::thread::sleep(std::time::Duration::from_millis(100));
            }
            Err(_) => {
                timed_out = false;
                output = None;
                break;
            }
        }
    }

    let duration = start.elapsed().as_secs_f64();

    let (stdout, stderr, exit_code) = match output {
        Some(o) => (
            String::from_utf8_lossy(&o.stdout).to_string(),
            String::from_utf8_lossy(&o.stderr).to_string(),
            o.status.code().unwrap_or(-1),
        ),
        None if timed_out => (
            String::new(),
            format!("Process killed after {timeout_secs}s timeout"),
            -9,
        ),
        None => (String::new(), "Process failed".into(), -1),
    };

    // Detect generated files
    let mut all_before = before;
    all_before.extend(before_scripts);
    let generated = detect_generated_files(&all_before, &[&out_dir, &scripts]);

    // Parse errors for self-healing
    let error_analysis = if exit_code != 0 {
        match language {
            "python" => parse_python_error(&stderr),
            "node" | "javascript" | "typescript" => parse_node_error(&stderr),
            _ => None,
        }
    } else {
        None
    };

    ExecutionResult {
        exit_code,
        stdout,
        stderr,
        duration_secs: duration,
        timed_out,
        generated_files: generated,
        error_analysis,
        sandboxed: use_bwrap,
    }
}

/// Format execution result for LLM consumption.
fn format_result(result: &ExecutionResult, script_name: &str) -> String {
    let mut out = String::with_capacity(1024);

    // Header
    let status = if result.timed_out {
        "TIMEOUT"
    } else if result.exit_code == 0 {
        "SUCCESS"
    } else {
        "FAILED"
    };
    let sandbox_tag = if result.sandboxed { " | sandboxed" } else { "" };
    out.push_str(&format!("[{status}] {script_name} | exit={} | {:.1}s{sandbox_tag}\n", result.exit_code, result.duration_secs));

    // Generated files (show first — this is often the most important result)
    if !result.generated_files.is_empty() {
        out.push_str("\n--- GENERATED FILES ---\n");
        for f in &result.generated_files {
            let label = file_type_label(f);
            let size = std::fs::metadata(f).map(|m| m.len()).unwrap_or(0);
            out.push_str(&format!("  [{label}] {f} ({} bytes)\n", size));
        }
    }

    // Stdout
    if !result.stdout.is_empty() {
        out.push_str("\n--- OUTPUT ---\n");
        if result.stdout.len() > MAX_OUTPUT {
            let boundary = result.stdout.floor_char_boundary(MAX_OUTPUT);
            out.push_str(&result.stdout[..boundary]);
            out.push_str(&format!("\n... [truncated, {} total chars]\n", result.stdout.len()));
        } else {
            out.push_str(&result.stdout);
            if !result.stdout.ends_with('\n') { out.push('\n'); }
        }
    }

    // Stderr (only on failure or if non-trivial)
    if !result.stderr.is_empty() && (result.exit_code != 0 || result.stderr.len() > 10) {
        out.push_str("\n--- STDERR ---\n");
        let max_err = 2000;
        if result.stderr.len() > max_err {
            let boundary = result.stderr.floor_char_boundary(max_err);
            out.push_str(&result.stderr[..boundary]);
            out.push_str("\n... [truncated]\n");
        } else {
            out.push_str(&result.stderr);
            if !result.stderr.ends_with('\n') { out.push('\n'); }
        }
    }

    // Structured error analysis for self-healing
    if let Some(analysis) = &result.error_analysis {
        out.push_str(&format!("\n--- ERROR ANALYSIS ---\n{analysis}\n"));
        out.push_str("ACTION: Use script_patch to fix the error line, then script_run again.\n");
    }

    if result.stdout.is_empty() && result.stderr.is_empty() && result.generated_files.is_empty() {
        out.push_str("\n(no output, no files generated)\n");
    }

    out
}


// ═══════════════════════════════════════════════════════════════════
// Tools
// ═══════════════════════════════════════════════════════════════════

// ── code_execute: inline code execution (no save, quick one-offs) ──

struct CodeExecuteTool;

impl Tool for CodeExecuteTool {
    fn name(&self) -> &'static str { "code_execute" }
    fn permission(&self) -> PermissionLevel { PermissionLevel::Sensitive }
    fn category(&self) -> &'static str { "coder" }

    fn definition(&self) -> serde_json::Value {
        serde_json::json!({
            "type": "function",
            "function": {
                "name": "code_execute",
                "description": "Execute code inline without saving to a file. Runs inside a bubblewrap sandbox (read-only filesystem, no network by default, PID isolation). Perfect for quick calculations, data analysis, one-off checks. Dependencies are auto-installed before sandboxed execution.\n\nEnvironment variables available:\n- YANTRIK_OUTPUT_DIR: directory for generated files (images, CSVs)\n- YANTRIK_SCRIPTS_DIR: scripts workspace directory\n\nSandbox modes:\n- 'strict' (default): No network, read-only system. For computations, file generation.\n- 'network': Allows network. For API calls, web scraping, data fetching.\n\nExamples:\n- Quick math: code_execute(language='python', code='print(2**256)')\n- Data analysis: code_execute(language='python', code='import pandas as pd; ...')\n- System check: code_execute(language='bash', code='df -h && free -m')\n- Web fetch: code_execute(language='python', code='import requests; ...', sandbox='network')",
                "parameters": {
                    "type": "object",
                    "properties": {
                        "code": {"type": "string", "description": "Code to execute"},
                        "language": {"type": "string", "enum": ["python", "bash", "node", "ruby"], "description": "Language (default: python)"},
                        "timeout": {"type": "integer", "description": "Timeout in seconds (default 120, max 600)"},
                        "sandbox": {"type": "string", "enum": ["strict", "network"], "description": "Sandbox mode. 'strict' (default): no network. 'network': allows network for API/web calls."}
                    },
                    "required": ["code"]
                }
            }
        })
    }

    fn execute(&self, _ctx: &ToolContext, args: &serde_json::Value) -> String {
        let code = args.get("code").and_then(|v| v.as_str()).unwrap_or_default();
        let language = args.get("language").and_then(|v| v.as_str()).unwrap_or("python");
        let timeout = args.get("timeout")
            .and_then(|v| v.as_u64())
            .unwrap_or(DEFAULT_TIMEOUT)
            .min(MAX_TIMEOUT);
        let sandbox = args.get("sandbox")
            .and_then(|v| v.as_str())
            .map(SandboxMode::from_str)
            .unwrap_or(SandboxMode::Strict);

        if code.is_empty() {
            return "Error: code is required".to_string();
        }

        let _ = std::fs::create_dir_all(scripts_dir());
        let _ = std::fs::create_dir_all(output_dir());

        // Write to temp file
        let ext = extension_for(language);
        let tmp_name = format!("_exec_{}{ext}", now_ts() as u64);
        let tmp_path = scripts_dir().join(&tmp_name);

        // Add shebang for Python
        let full_code = if language == "python" && !code.starts_with("#!") {
            format!("#!/usr/bin/env python3\n# -*- coding: utf-8 -*-\nimport os, sys\nsys.path.insert(0, os.environ.get('YANTRIK_SCRIPTS_DIR', '.'))\n{code}")
        } else {
            code.to_string()
        };

        if let Err(e) = std::fs::write(&tmp_path, &full_code) {
            return format!("Error writing temp file: {e}");
        }

        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let _ = std::fs::set_permissions(&tmp_path, std::fs::Permissions::from_mode(0o755));
        }

        // Auto-install deps
        let deps = match language {
            "python" => detect_python_deps(code),
            "node" | "javascript" => detect_node_deps(code),
            _ => vec![],
        };
        if let Some(err) = install_deps(language, &deps) {
            let _ = std::fs::remove_file(&tmp_path);
            return err;
        }

        // Execute in sandbox
        let result = execute_script(&tmp_path, language, "", &HashMap::new(), timeout, None, sandbox);

        // Clean up temp file
        let _ = std::fs::remove_file(&tmp_path);

        format_result(&result, &format!("inline_{language}"))
    }
}

// ── script_write ──

struct ScriptWriteTool;

impl Tool for ScriptWriteTool {
    fn name(&self) -> &'static str { "script_write" }
    fn permission(&self) -> PermissionLevel { PermissionLevel::Standard }
    fn category(&self) -> &'static str { "coder" }

    fn definition(&self) -> serde_json::Value {
        serde_json::json!({
            "type": "function",
            "function": {
                "name": "script_write",
                "description": "Create or update a persistent script in the managed workspace. Scripts are saved with proper permissions and tracked in a registry. Use this for code that should persist and be re-runnable. After writing, use script_run to execute.\n\nThe script has access to:\n- YANTRIK_OUTPUT_DIR env var: save generated files here (charts, CSVs, HTML)\n- YANTRIK_SCRIPTS_DIR env var: the scripts workspace\n- Auto-installed dependencies (pip/npm/gem)\n\nFor visualizations, save output to YANTRIK_OUTPUT_DIR. For web dashboards, create HTML files.",
                "parameters": {
                    "type": "object",
                    "properties": {
                        "name": {"type": "string", "description": "Script name (e.g., 'stock_analyzer', 'backup_photos'). Alphanumeric + underscores."},
                        "content": {"type": "string", "description": "Full script content. Include shebang (e.g., #!/usr/bin/env python3)."},
                        "description": {"type": "string", "description": "Brief description of what the script does"},
                        "language": {"type": "string", "enum": ["python", "bash", "node", "typescript", "ruby", "html"], "description": "Language (auto-detected if omitted)"},
                        "tags": {"type": "array", "items": {"type": "string"}, "description": "Optional tags for categorization (e.g., ['finance', 'visualization'])"}
                    },
                    "required": ["name", "content", "description"]
                }
            }
        })
    }

    fn execute(&self, _ctx: &ToolContext, args: &serde_json::Value) -> String {
        let name = args.get("name").and_then(|v| v.as_str()).unwrap_or_default();
        let content = args.get("content").and_then(|v| v.as_str()).unwrap_or_default();
        let description = args.get("description").and_then(|v| v.as_str()).unwrap_or("");
        let lang_hint = args.get("language").and_then(|v| v.as_str());
        let tags: Vec<String> = args.get("tags")
            .and_then(|v| v.as_array())
            .map(|arr| arr.iter().filter_map(|v| v.as_str().map(String::from)).collect())
            .unwrap_or_default();

        if name.is_empty() || content.is_empty() {
            return "Error: name and content are required".to_string();
        }

        let safe_name = sanitize_name(name);
        let language = lang_hint
            .map(|l| l.to_string())
            .unwrap_or_else(|| detect_language(&safe_name, content));
        let ext = extension_for(&language);
        let filename = format!("{safe_name}{ext}");

        let dir = scripts_dir();
        if let Err(e) = std::fs::create_dir_all(&dir) {
            return format!("Error creating scripts directory: {e}");
        }
        let _ = std::fs::create_dir_all(output_dir());

        let filepath = dir.join(&filename);
        let is_update = filepath.exists();

        if let Err(e) = std::fs::write(&filepath, content) {
            return format!("Error writing script: {e}");
        }

        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let _ = std::fs::set_permissions(&filepath, std::fs::Permissions::from_mode(0o755));
        }

        // Detect dependencies
        let deps = match language.as_str() {
            "python" => detect_python_deps(content),
            "node" | "javascript" | "typescript" => detect_node_deps(content),
            _ => vec![],
        };

        // Update registry
        let mut registry = load_registry();
        let now = now_ts();

        let entry = registry.entry(safe_name.clone()).or_insert(ScriptEntry {
            name: safe_name.clone(),
            filename: filename.clone(),
            language: language.clone(),
            description: description.to_string(),
            created_at: now,
            updated_at: now,
            last_run: None,
            last_run_status: None,
            last_error: None,
            run_count: 0,
            success_count: 0,
            dependencies: vec![],
            generated_files: vec![],
            tags: vec![],
        });
        entry.filename = filename.clone();
        entry.language = language.clone();
        entry.description = description.to_string();
        entry.dependencies = deps.clone();
        entry.updated_at = now;
        entry.tags = tags.clone();
        save_registry(&registry);

        let action = if is_update { "Updated" } else { "Created" };
        let deps_note = if !deps.is_empty() {
            format!("\nDependencies: {} (auto-install on run)", deps.join(", "))
        } else {
            String::new()
        };
        let tags_note = if !tags.is_empty() {
            format!("\nTags: {}", tags.join(", "))
        } else {
            String::new()
        };
        let lines = content.lines().count();

        format!(
            "{action} script: {SCRIPTS_DIR}/{filename}\n\
             Language: {language} | {lines} lines{deps_note}{tags_note}\n\
             Description: {description}\n\n\
             Use script_run(name=\"{safe_name}\") to execute."
        )
    }
}

// ── script_run ──

struct ScriptRunTool;

impl Tool for ScriptRunTool {
    fn name(&self) -> &'static str { "script_run" }
    fn permission(&self) -> PermissionLevel { PermissionLevel::Sensitive }
    fn category(&self) -> &'static str { "coder" }

    fn definition(&self) -> serde_json::Value {
        serde_json::json!({
            "type": "function",
            "function": {
                "name": "script_run",
                "description": "Execute a script from the managed workspace. Scripts run inside a bubblewrap sandbox by default for security.\n\nFeatures:\n- Sandboxed execution (read-only filesystem, no network by default)\n- Auto-installs dependencies (pip/npm/gem) before execution\n- Real timeout with process kill (no zombie processes)\n- Detects generated files (images, CSVs, HTML) and reports them\n- Structured error analysis for self-healing (tells you which line to fix)\n- Environment variables: YANTRIK_OUTPUT_DIR, YANTRIK_SCRIPTS_DIR\n\nSandbox modes:\n- 'strict' (default): No network, read-only system, isolated PIDs. For computations, analysis, file generation.\n- 'network': Filesystem isolation but network allowed. For web portals, API calls, data fetching.\n- 'none': No sandbox (discouraged). Only if sandbox causes issues.\n\nIf execution fails, read the ERROR ANALYSIS section and use script_patch to fix the error, then re-run.",
                "parameters": {
                    "type": "object",
                    "properties": {
                        "name": {"type": "string", "description": "Script name (from script_write)"},
                        "args": {"type": "string", "description": "Command-line arguments (shell-style quoting supported)"},
                        "timeout": {"type": "integer", "description": "Timeout in seconds (default 120, max 600)"},
                        "env": {"type": "object", "description": "Environment variables to set (key-value pairs)"},
                        "working_dir": {"type": "string", "description": "Working directory (default: scripts dir)"},
                        "sandbox": {"type": "string", "enum": ["strict", "network", "none"], "description": "Sandbox mode. 'strict' (default): no network. 'network': allows network for web portals/APIs. 'none': no sandbox."}
                    },
                    "required": ["name"]
                }
            }
        })
    }

    fn execute(&self, _ctx: &ToolContext, args: &serde_json::Value) -> String {
        let name = args.get("name").and_then(|v| v.as_str()).unwrap_or_default();
        let script_args = args.get("args").and_then(|v| v.as_str()).unwrap_or("");
        let timeout = args.get("timeout")
            .and_then(|v| v.as_u64())
            .unwrap_or(DEFAULT_TIMEOUT)
            .min(MAX_TIMEOUT);
        let env_vars: HashMap<String, String> = args.get("env")
            .and_then(|v| v.as_object())
            .map(|obj| {
                obj.iter()
                    .filter_map(|(k, v)| v.as_str().map(|s| (k.clone(), s.to_string())))
                    .collect()
            })
            .unwrap_or_default();
        let working_dir = args.get("working_dir").and_then(|v| v.as_str());
        let sandbox = args.get("sandbox")
            .and_then(|v| v.as_str())
            .map(SandboxMode::from_str)
            .unwrap_or(SandboxMode::Strict);

        if name.is_empty() {
            return "Error: name is required".to_string();
        }

        let safe_name = sanitize_name(name);
        let mut registry = load_registry();

        let entry = match registry.get(&safe_name) {
            Some(e) => e.clone(),
            None => return format!("Error: script '{safe_name}' not found. Use script_list to see available scripts."),
        };

        let filepath = scripts_dir().join(&entry.filename);
        if !filepath.exists() {
            return format!("Error: script file {} not found on disk", entry.filename);
        }

        // Auto-install dependencies (runs OUTSIDE sandbox — needs network + write to site-packages)
        if !entry.dependencies.is_empty() {
            if let Some(err) = install_deps(&entry.language, &entry.dependencies) {
                return err;
            }
        }

        // Execute in sandbox
        let result = execute_script(&filepath, &entry.language, script_args, &env_vars, timeout, working_dir, sandbox);

        // Update registry
        let now = now_ts();
        if let Some(e) = registry.get_mut(&safe_name) {
            e.last_run = Some(now);
            if result.exit_code == 0 {
                e.last_run_status = Some("success".to_string());
                e.last_error = None;
                e.success_count += 1;
            } else {
                e.last_run_status = Some(format!("exit {}", result.exit_code));
                e.last_error = result.error_analysis.clone();
            }
            e.run_count += 1;
            if !result.generated_files.is_empty() {
                e.generated_files = result.generated_files.clone();
            }
        }
        save_registry(&registry);

        format_result(&result, &entry.name)
    }
}

// ── script_patch: edit specific lines without rewriting ──

struct ScriptPatchTool;

impl Tool for ScriptPatchTool {
    fn name(&self) -> &'static str { "script_patch" }
    fn permission(&self) -> PermissionLevel { PermissionLevel::Standard }
    fn category(&self) -> &'static str { "coder" }

    fn definition(&self) -> serde_json::Value {
        serde_json::json!({
            "type": "function",
            "function": {
                "name": "script_patch",
                "description": "Edit specific parts of an existing script without rewriting the whole thing. Supports:\n- Replace exact text: find old_text and replace with new_text\n- Insert at line: add new lines at a specific line number\n- Delete lines: remove a range of lines\n\nThis is the primary tool for fixing errors after script_run fails. Read the ERROR ANALYSIS from script_run, then use script_patch to fix the specific line.",
                "parameters": {
                    "type": "object",
                    "properties": {
                        "name": {"type": "string", "description": "Script name"},
                        "action": {"type": "string", "enum": ["replace", "insert", "delete"], "description": "Patch action"},
                        "old_text": {"type": "string", "description": "Text to find and replace (for 'replace' action)"},
                        "new_text": {"type": "string", "description": "Replacement text (for 'replace' and 'insert' actions)"},
                        "line": {"type": "integer", "description": "Line number for 'insert' (inserts before this line) or start line for 'delete'"},
                        "end_line": {"type": "integer", "description": "End line for 'delete' (inclusive). Defaults to same as line."}
                    },
                    "required": ["name", "action"]
                }
            }
        })
    }

    fn execute(&self, _ctx: &ToolContext, args: &serde_json::Value) -> String {
        let name = args.get("name").and_then(|v| v.as_str()).unwrap_or_default();
        let action = args.get("action").and_then(|v| v.as_str()).unwrap_or_default();
        let old_text = args.get("old_text").and_then(|v| v.as_str());
        let new_text = args.get("new_text").and_then(|v| v.as_str());
        let line = args.get("line").and_then(|v| v.as_u64()).map(|l| l as usize);
        let end_line = args.get("end_line").and_then(|v| v.as_u64()).map(|l| l as usize);

        if name.is_empty() {
            return "Error: name is required".to_string();
        }

        let safe_name = sanitize_name(name);
        let registry = load_registry();

        let entry = match registry.get(&safe_name) {
            Some(e) => e,
            None => return format!("Error: script '{safe_name}' not found"),
        };

        let filepath = scripts_dir().join(&entry.filename);
        let content = match std::fs::read_to_string(&filepath) {
            Ok(c) => c,
            Err(e) => return format!("Error reading script: {e}"),
        };

        let new_content = match action {
            "replace" => {
                let old = match old_text {
                    Some(t) if !t.is_empty() => t,
                    _ => return "Error: old_text is required for replace".to_string(),
                };
                let new = new_text.unwrap_or("");

                let count = content.matches(old).count();
                if count == 0 {
                    return format!("Error: old_text not found in script. Script has {} lines.\nUse script_read to see current content.", content.lines().count());
                }

                let result = content.replacen(old, new, 1);
                if count > 1 {
                    format!("Warning: found {count} occurrences, replaced first one only.");
                }
                result
            }
            "insert" => {
                let ln = match line {
                    Some(l) if l >= 1 => l,
                    _ => return "Error: line number is required for insert (1-based)".to_string(),
                };
                let text = match new_text {
                    Some(t) => t,
                    None => return "Error: new_text is required for insert".to_string(),
                };

                let mut lines: Vec<&str> = content.lines().collect();
                let idx = (ln - 1).min(lines.len());
                // Insert each line of new_text
                let new_lines: Vec<&str> = text.lines().collect();
                for (i, nl) in new_lines.iter().enumerate() {
                    lines.insert(idx + i, nl);
                }
                lines.join("\n") + "\n"
            }
            "delete" => {
                let start = match line {
                    Some(l) if l >= 1 => l,
                    _ => return "Error: line number is required for delete (1-based)".to_string(),
                };
                let end = end_line.unwrap_or(start);

                let lines: Vec<&str> = content.lines().collect();
                if start > lines.len() {
                    return format!("Error: line {start} is past end of file ({} lines)", lines.len());
                }

                let mut result: Vec<&str> = Vec::new();
                for (i, l) in lines.iter().enumerate() {
                    let ln = i + 1;
                    if ln < start || ln > end {
                        result.push(l);
                    }
                }
                result.join("\n") + "\n"
            }
            _ => return "Error: action must be 'replace', 'insert', or 'delete'".to_string(),
        };

        if let Err(e) = std::fs::write(&filepath, &new_content) {
            return format!("Error writing patched script: {e}");
        }

        // Re-detect dependencies
        let mut registry = load_registry();
        if let Some(e) = registry.get_mut(&safe_name) {
            e.dependencies = match e.language.as_str() {
                "python" => detect_python_deps(&new_content),
                "node" | "javascript" | "typescript" => detect_node_deps(&new_content),
                _ => vec![],
            };
            e.updated_at = now_ts();
        }
        save_registry(&registry);

        let new_line_count = new_content.lines().count();
        format!("Patched {}: {action} applied. Script now has {new_line_count} lines.\nUse script_run(name=\"{safe_name}\") to test.", entry.filename)
    }
}

// ── script_list ──

struct ScriptListTool;

impl Tool for ScriptListTool {
    fn name(&self) -> &'static str { "script_list" }
    fn permission(&self) -> PermissionLevel { PermissionLevel::Safe }
    fn category(&self) -> &'static str { "coder" }

    fn definition(&self) -> serde_json::Value {
        serde_json::json!({
            "type": "function",
            "function": {
                "name": "script_list",
                "description": "List all scripts in the managed workspace with metadata (language, description, run history, dependencies, tags, generated files).",
                "parameters": {
                    "type": "object",
                    "properties": {
                        "tag": {"type": "string", "description": "Filter by tag"}
                    }
                }
            }
        })
    }

    fn execute(&self, _ctx: &ToolContext, args: &serde_json::Value) -> String {
        let tag_filter = args.get("tag").and_then(|v| v.as_str());

        let registry = load_registry();
        if registry.is_empty() {
            return "No scripts yet. Use script_write or code_execute to create one.".to_string();
        }

        let mut entries: Vec<_> = registry.values()
            .filter(|e| tag_filter.map(|t| e.tags.iter().any(|et| et == t)).unwrap_or(true))
            .collect();
        entries.sort_by(|a, b| a.name.cmp(&b.name));

        if entries.is_empty() {
            return format!("No scripts matching tag '{}'", tag_filter.unwrap_or(""));
        }

        let mut out = format!("{} scripts", entries.len());
        if let Some(tag) = tag_filter {
            out.push_str(&format!(" (tag: {tag})"));
        }
        out.push_str(":\n\n");

        for e in &entries {
            let status_icon = match e.last_run_status.as_deref() {
                Some("success") => "OK",
                Some(_) => "FAIL",
                None => "--",
            };
            out.push_str(&format!(
                "  [{status_icon}] {} [{}] — {}\n",
                e.name, e.language, e.description
            ));
            out.push_str(&format!(
                "        runs: {} (success: {}) | deps: {}\n",
                e.run_count, e.success_count,
                if e.dependencies.is_empty() { "none".into() } else { e.dependencies.join(", ") }
            ));
            if !e.tags.is_empty() {
                out.push_str(&format!("        tags: {}\n", e.tags.join(", ")));
            }
            if !e.generated_files.is_empty() {
                let file_names: Vec<&str> = e.generated_files.iter()
                    .filter_map(|f| Path::new(f).file_name()?.to_str())
                    .collect();
                out.push_str(&format!("        outputs: {}\n", file_names.join(", ")));
            }
            if let Some(err) = &e.last_error {
                let first_line = err.lines().next().unwrap_or(err);
                out.push_str(&format!("        last error: {first_line}\n"));
            }
        }
        out
    }
}

// ── script_read ──

struct ScriptReadTool;

impl Tool for ScriptReadTool {
    fn name(&self) -> &'static str { "script_read" }
    fn permission(&self) -> PermissionLevel { PermissionLevel::Safe }
    fn category(&self) -> &'static str { "coder" }

    fn definition(&self) -> serde_json::Value {
        serde_json::json!({
            "type": "function",
            "function": {
                "name": "script_read",
                "description": "Read a script's contents with line numbers. Useful for understanding existing scripts or preparing a script_patch.",
                "parameters": {
                    "type": "object",
                    "properties": {
                        "name": {"type": "string", "description": "Script name"},
                        "start_line": {"type": "integer", "description": "Start line (1-based, default: 1)"},
                        "end_line": {"type": "integer", "description": "End line (default: entire file)"}
                    },
                    "required": ["name"]
                }
            }
        })
    }

    fn execute(&self, _ctx: &ToolContext, args: &serde_json::Value) -> String {
        let name = args.get("name").and_then(|v| v.as_str()).unwrap_or_default();
        let start = args.get("start_line").and_then(|v| v.as_u64()).unwrap_or(1) as usize;
        let end = args.get("end_line").and_then(|v| v.as_u64()).map(|l| l as usize);

        if name.is_empty() {
            return "Error: name is required".to_string();
        }

        let safe_name = sanitize_name(name);
        let registry = load_registry();

        let entry = match registry.get(&safe_name) {
            Some(e) => e,
            None => return format!("Error: script '{safe_name}' not found"),
        };

        let filepath = scripts_dir().join(&entry.filename);
        let content = match std::fs::read_to_string(&filepath) {
            Ok(c) => c,
            Err(e) => return format!("Error reading script: {e}"),
        };

        let lines: Vec<&str> = content.lines().collect();
        let total = lines.len();
        let end_line = end.unwrap_or(total).min(total);
        let start_line = start.max(1).min(total);

        let mut out = format!(
            "Script: {} [{}] | {} lines | deps: {}\n\n",
            entry.name, entry.language, total,
            if entry.dependencies.is_empty() { "none".into() } else { entry.dependencies.join(", ") }
        );

        for (i, line) in lines.iter().enumerate() {
            let ln = i + 1;
            if ln >= start_line && ln <= end_line {
                out.push_str(&format!("{ln:4} | {line}\n"));
            }
        }

        if end_line < total {
            out.push_str(&format!("\n... ({} more lines)\n", total - end_line));
        }

        out
    }
}

// ── script_delete ──

struct ScriptDeleteTool;

impl Tool for ScriptDeleteTool {
    fn name(&self) -> &'static str { "script_delete" }
    fn permission(&self) -> PermissionLevel { PermissionLevel::Standard }
    fn category(&self) -> &'static str { "coder" }

    fn definition(&self) -> serde_json::Value {
        serde_json::json!({
            "type": "function",
            "function": {
                "name": "script_delete",
                "description": "Delete a script and its registry entry. Generated output files are preserved.",
                "parameters": {
                    "type": "object",
                    "properties": {
                        "name": {"type": "string", "description": "Script name to delete"}
                    },
                    "required": ["name"]
                }
            }
        })
    }

    fn execute(&self, _ctx: &ToolContext, args: &serde_json::Value) -> String {
        let name = args.get("name").and_then(|v| v.as_str()).unwrap_or_default();
        if name.is_empty() {
            return "Error: name is required".to_string();
        }

        let safe_name = sanitize_name(name);
        let mut registry = load_registry();

        match registry.remove(&safe_name) {
            Some(entry) => {
                let filepath = scripts_dir().join(&entry.filename);
                let _ = std::fs::remove_file(&filepath);
                save_registry(&registry);
                format!("Deleted script '{}' ({} runs, {} success)", entry.name, entry.run_count, entry.success_count)
            }
            None => format!("Error: script '{safe_name}' not found"),
        }
    }
}

// ═══════════════════════════════════════════════════════════════════
// shell_words — minimal shell argument parser
// ═══════════════════════════════════════════════════════════════════

mod shell_words {
    /// Split a string into shell-style words, respecting quotes.
    pub fn split(s: &str) -> Result<Vec<String>, &'static str> {
        let mut words = Vec::new();
        let mut current = String::new();
        let mut chars = s.chars().peekable();
        let mut in_single = false;
        let mut in_double = false;

        while let Some(c) = chars.next() {
            match c {
                '\'' if !in_double => in_single = !in_single,
                '"' if !in_single => in_double = !in_double,
                '\\' if !in_single => {
                    if let Some(next) = chars.next() {
                        current.push(next);
                    }
                }
                ' ' | '\t' if !in_single && !in_double => {
                    if !current.is_empty() {
                        words.push(std::mem::take(&mut current));
                    }
                }
                _ => current.push(c),
            }
        }

        if !current.is_empty() {
            words.push(current);
        }

        if in_single || in_double {
            Err("Unclosed quote")
        } else {
            Ok(words)
        }
    }
}
