//! Test the Cognitive Router — embedding-based tool matching.
//!
//! Run: cargo run --example test_router -p yantrik-ml

use std::time::Instant;
use yantrik_ml::{CognitiveRouter, RouteDecision, CandleEmbedder, Embedder};

fn main() {
    println!("Loading MiniLM embedder...");
    let t0 = Instant::now();
    let embedder = CandleEmbedder::from_hub("sentence-transformers/all-MiniLM-L6-v2", None)
        .expect("Failed to load MiniLM");
    println!("Embedder loaded in {:.1}s (dim={})\n", t0.elapsed().as_secs_f64(), embedder.dim());

    let router = CognitiveRouter::new(Box::new(embedder));

    // Register tools (simulating companion registry)
    let t0 = Instant::now();
    router.register_tools(&[
        ("system_info", "Read CPU memory disk and network stats", "system"),
        ("list_processes", "List running processes by CPU or memory use", "system"),
        ("kill_process", "Stop process by PID or name", "system"),
        ("battery_forecast", "Get battery status with time forecast", "system"),
        ("send_notification", "Show local desktop notification to user only", "system"),
        ("disk_usage", "Show disk space usage for all mounted partitions", "disk"),
        ("dir_size", "Calculate the size of a directory and its largest subdirectories", "disk"),
        ("read_file", "Read text file contents", "files"),
        ("write_file", "Write or overwrite text file contents", "files"),
        ("edit_file", "Replace specific text inside a file", "files"),
        ("list_files", "List files in a directory; no content search", "files"),
        ("manage_files", "Move, copy, rename, or delete files", "files"),
        ("search_files", "Search file contents by plain text in a directory", "files"),
        ("glob", "Find files by filename pattern recursively", "files"),
        ("grep", "Search file contents with regex", "files"),
        ("word_count", "Count lines, words, and characters in a file", "text"),
        ("diff_files", "Compare two text files and show differences", "text"),
        ("hash_file", "Compute SHA-256 hash of a file", "text"),
        ("calculate", "Evaluate a mathematical expression", "math"),
        ("unit_convert", "Convert between units: temperature, distance, mass, time", "math"),
        ("date_calc", "Get current date/time or calculate date differences", "time"),
        ("timer", "Start countdown timer for a duration", "time"),
        ("git_status", "Show the working tree status of a git repository", "git"),
        ("git_log", "Show recent git commit history", "git"),
        ("git_diff", "Show uncommitted changes in a git repository", "git"),
        ("git_commit", "Commit changes in a git repository", "git"),
        ("git_branch", "List branches in a git repository", "git"),
        ("git_clone", "Clone a git repository to a local directory", "git"),
        ("network_ping", "Ping host for reachability only", "network"),
        ("network_interfaces", "List network adapters and link status", "network"),
        ("network_ports", "List open or listening local ports", "network"),
        ("network_dns", "Show current DNS server settings", "network"),
        ("network_traceroute", "Trace path packets take to a host", "network"),
        ("network_diagnose", "Run full network health check", "network"),
        ("download_file", "Download URL to a local file; do not open or extract", "network"),
        ("docker_ps", "List running Docker containers", "docker"),
        ("docker_images", "List Docker images", "docker"),
        ("docker_logs", "View logs from a Docker container", "docker"),
        ("docker_start", "Start a stopped Docker container", "docker"),
        ("docker_stop", "Stop a running Docker container", "docker"),
        ("service_list", "List system services", "services"),
        ("service_control", "Start, stop, or restart a system service", "services"),
        ("service_status", "Show status of a system service", "services"),
        ("package_search", "Search available packages by name or keyword", "packages"),
        ("package_install", "Install a package from the system repository", "packages"),
        ("package_remove", "Remove an installed package", "packages"),
        ("package_list", "List installed packages, optionally filtered by name", "packages"),
        ("browse", "Open URL in controlled browser session", "browser"),
        ("web_search", "Search the web by query; snippets only, no page fetch", "browser"),
        ("browser_screenshot", "Save screenshot of current tab as image file", "browser"),
        ("audio_control", "Set volume level or toggle mute", "audio"),
        ("audio_info", "Get current audio volume and device info", "audio"),
        ("screenshot", "Capture full screen image; not browser-only", "media"),
        ("wifi_scan", "Scan for nearby Wi-Fi networks", "wifi"),
        ("wifi_connect", "Connect to Wi-Fi by SSID", "wifi"),
        ("wifi_status", "Show current Wi-Fi connection details", "wifi"),
        ("bluetooth_scan", "Scan for nearby Bluetooth devices", "bluetooth"),
        ("bluetooth_connect", "Connect to a paired Bluetooth device", "bluetooth"),
        ("bluetooth_info", "Get Bluetooth adapter status and paired/connected devices", "bluetooth"),
        ("antivirus_scan", "Scan a file or directory for malware using ClamAV", "security"),
        ("firewall_status", "Check if the firewall is active and show basic info", "security"),
        ("firewall_list_rules", "List all current firewall rules", "security"),
        ("vault_store", "Store credential securely in vault", "vault"),
        ("vault_get", "Retrieve credential from vault", "vault"),
        ("ssh_run", "Run a command on a remote host via SSH", "ssh"),
        ("ssh_list_hosts", "List SSH hosts from SSH config and known_hosts", "ssh"),
        ("base64_encode", "Encode text to Base64", "encoding"),
        ("base64_decode", "Decode Base64 text back to plain text", "encoding"),
        ("json_format", "Pretty-print, minify, or validate JSON text", "encoding"),
        ("archive_create", "Create a tar.gz archive from source files/directories", "archive"),
        ("archive_extract", "Extract a tar.gz archive to a destination directory", "archive"),
        ("get_weather", "Get current weather and forecast for a location", "weather"),
        ("set_wallpaper", "Set the desktop wallpaper to an image file", "desktop"),
        ("set_resolution", "Change display resolution", "desktop"),
        ("open_url", "Open URL in user's default browser/app, outside session", "desktop"),
        ("list_windows", "List desktop app windows; not browser tabs", "window"),
        ("focus_window", "Focus a window by title or app name", "window"),
        ("run_command", "Run shell command only when no specialized tool fits", "terminal"),
        ("explain_last_error", "Explain the most recent terminal error", "terminal"),
        ("code_execute", "Run code now without saving a script", "code"),
        ("script_write", "Create or overwrite a saved script", "code"),
        ("read_clipboard", "Read the current contents of the user's clipboard", "clipboard"),
        ("write_clipboard", "Write text to the user's clipboard", "clipboard"),
        ("remember", "Store save note keep a new memory fact for future reference", "memory"),
        ("recall", "Search retrieve find relevant memories from past conversations", "memory"),
        ("forget_memory", "Delete one memory by ID; permanent", "memory"),
    ]);
    println!("Registered {} tools in {:.0}ms\n", router.tool_count(), t0.elapsed().as_millis());

    let tests = vec![
        ("Hello!", "conversation"),
        ("Thanks!", "conversation"),
        ("Goodbye", "conversation"),
        ("What is 42 * 58?", "calculate"),
        ("Convert 100 miles to km", "unit_convert"),
        ("What time is it?", "date_calc"),
        ("Take a screenshot", "screenshot"),
        ("Show running processes", "list_processes"),
        ("Check disk space", "disk_usage"),
        ("Scan for wifi networks", "wifi_scan"),
        ("Connect to bluetooth speaker", "bluetooth_connect"),
        ("Search the web for AI news", "web_search"),
        ("Read the file config.yaml", "read_file"),
        ("Find all python files", "glob"),
        ("Search for TODO in source code", "grep"),
        ("Show git status", "git_status"),
        ("Commit my changes", "git_commit"),
        ("Check battery level", "battery_forecast"),
        ("Set volume to 50%", "audio_control"),
        ("Send a notification", "send_notification"),
        ("List docker containers", "docker_ps"),
        ("Stop the nginx service", "service_control"),
        ("Ping 8.8.8.8", "network_ping"),
        ("Check firewall rules", "firewall_list_rules"),
        ("Kill process 1234", "kill_process"),
        ("Download this file", "download_file"),
        ("Scan for malware", "antivirus_scan"),
        ("Check the weather", "get_weather"),
        ("Set wallpaper to sunset.jpg", "set_wallpaper"),
        ("SSH into my server", "ssh_run"),
        ("Encode this text to base64", "base64_encode"),
        ("Create a zip archive", "archive_create"),
        ("How much RAM is being used?", "system_info"),
        ("Compare these two files", "diff_files"),
        ("Whats the hash of this file", "hash_file"),
        ("What's the weather forecast?", "get_weather"),
        ("Open google.com in browser", "browse"),
        ("List installed packages", "package_list"),
        ("Translate this to Spanish", "needs_llm"),
        ("Write me a poem about rust", "needs_llm"),
        // Edge cases — paraphrases that keywords might miss
        ("How's the battery doing?", "battery_forecast"),
        ("Any viruses on my system?", "antivirus_scan"),
        ("What repos do I have?", "git_status"),
        ("Show me network connections", "network_interfaces"),
        ("Compress this folder", "archive_create"),
        ("What processes are eating CPU?", "list_processes"),
        ("Is port 8080 open?", "network_ports"),
        ("Save my credentials", "vault_store"),
        ("Whats on my clipboard?", "read_clipboard"),
        ("Remember I like dark mode", "remember"),
    ];

    let total = tests.len();
    let mut correct = 0;

    println!("{:-<80}", "");
    println!("  COGNITIVE ROUTER TEST — {} queries (MiniLM embeddings)", total);
    println!("{:-<80}\n", "");

    for (query, expected) in &tests {
        let t0 = Instant::now();
        let decision = router.route(query);
        let ms = t0.elapsed().as_millis();

        let (got, matched) = match &decision {
            RouteDecision::Conversation { .. } => {
                ("conversation".to_string(), *expected == "conversation")
            }
            RouteDecision::Tool { name, score, .. } => {
                (format!("{} ({:.2})", name, score), name == expected)
            }
            RouteDecision::Recipe { id, score, .. } => {
                (format!("recipe:{} ({:.2})", id, score), id == expected)
            }
            RouteDecision::NeedsLLM => {
                ("needs_llm".to_string(), *expected == "needs_llm")
            }
        };

        let mark = if matched { "OK" } else { "MISS" };
        if matched { correct += 1; }
        println!("  {:<4} {:<45} -> {:<35} [{}ms]", mark, query, got, ms);
    }

    println!("\n{:-<80}", "");
    println!("  Result: {}/{} correct ({:.0}%)", correct, total, correct as f64 / total as f64 * 100.0);
    println!("{:-<80}", "");

    // Show top-3 for a few ambiguous queries
    println!("\n  Top-3 candidates for ambiguous queries:");
    for q in &["Any viruses on my system?", "Is port 8080 open?", "Compress this folder"] {
        let top = router.route_top_n(q, 3);
        println!("  \"{}\"", q);
        for (name, score) in &top {
            println!("    {:.3}  {}", score, name);
        }
    }
}
