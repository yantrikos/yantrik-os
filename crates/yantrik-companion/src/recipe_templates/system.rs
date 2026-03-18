//! System administration, security, and development recipe templates.

use super::RecipeTemplate;
use crate::recipe::RecipeStep;

fn tool(name: &str, args: serde_json::Value, store: &str) -> RecipeStep {
    RecipeStep::Tool {
        tool_name: name.to_string(),
        args,
        store_as: store.to_string(),
        on_error: Default::default(),
    }
}

fn think(prompt: &str, store: &str) -> RecipeStep {
    RecipeStep::Think {
        prompt: prompt.to_string(),
        store_as: store.to_string(),
    }
}

fn notify(msg: &str) -> RecipeStep {
    RecipeStep::Notify {
        message: msg.to_string(),
    }
}

pub fn templates() -> Vec<RecipeTemplate> {
    vec![
        // 28. Disk Cleanup
        RecipeTemplate {
            id: "builtin_disk_cleanup",
            name: "Disk Cleanup",
            description: "Analyze disk usage and suggest files to clean up",
            category: "system",
            keywords: &[
                "disk", "storage", "cleanup", "space", "disk full",
                "free space", "disk usage", "clean up",
            ],
            required_vars: &[],
            steps: || {
                vec![
                    tool("disk_usage", serde_json::json!({}), "usage"),
                    tool(
                        "analyze_disk",
                        serde_json::json!({}),
                        "analysis",
                    ),
                    think(
                        "Analyze disk usage and suggest cleanup:\n\
                         Usage: {{usage}}\n\
                         Analysis: {{analysis}}\n\n\
                         Report:\n\
                         - Overall disk usage and free space\n\
                         - Largest directories or files\n\
                         - Safe cleanup suggestions (caches, temp files, old logs)\n\
                         - Estimated space recoverable\n\
                         Do NOT suggest deleting user files without confirmation.",
                        "report",
                    ),
                    notify("Disk Cleanup Report:\n\n{{report}}"),
                ]
            },
            trigger: None,
        },
        // 29. System Health Check
        RecipeTemplate {
            id: "builtin_system_health",
            name: "System Health Check",
            description: "Check CPU, memory, disk, and running processes",
            category: "system",
            keywords: &[
                "system health", "system status", "how is system",
                "performance", "system check", "health check",
            ],
            required_vars: &[],
            steps: || {
                vec![
                    tool("system_info", serde_json::json!({}), "sysinfo"),
                    tool("disk_usage", serde_json::json!({}), "disk"),
                    tool(
                        "list_processes",
                        serde_json::json!({"sort": "cpu", "limit": 10}),
                        "top_processes",
                    ),
                    think(
                        "Create a system health report:\n\
                         System info: {{sysinfo}}\n\
                         Disk: {{disk}}\n\
                         Top processes: {{top_processes}}\n\n\
                         Report:\n\
                         - Overall health (Healthy / Warning / Critical)\n\
                         - CPU load and memory usage\n\
                         - Disk space status\n\
                         - Any processes using unusual resources\n\
                         - Recommendations if any issues found",
                        "health",
                    ),
                    notify("System Health:\n\n{{health}}"),
                ]
            },
            trigger: None,
        },
        // 30. Security Audit
        RecipeTemplate {
            id: "builtin_security_audit",
            name: "Security Audit",
            description: "Check firewall, open ports, and antivirus status",
            category: "system",
            keywords: &[
                "security", "audit", "secure", "firewall",
                "ports", "vulnerability", "security check",
            ],
            required_vars: &[],
            steps: || {
                vec![
                    tool(
                        "firewall_status",
                        serde_json::json!({}),
                        "firewall",
                    ),
                    tool(
                        "network_ports",
                        serde_json::json!({}),
                        "ports",
                    ),
                    tool(
                        "antivirus_status",
                        serde_json::json!({}),
                        "av_status",
                    ),
                    think(
                        "Security audit report:\n\
                         Firewall: {{firewall}}\n\
                         Open ports: {{ports}}\n\
                         Antivirus: {{av_status}}\n\n\
                         Assess:\n\
                         - Firewall status and configuration\n\
                         - Unexpected open ports\n\
                         - Antivirus update status\n\
                         - Overall security posture (Good / Needs Attention / At Risk)\n\
                         - Specific recommendations",
                        "audit",
                    ),
                    notify("Security Audit:\n\n{{audit}}"),
                ]
            },
            trigger: None,
        },
        // 31. Network Diagnostics
        RecipeTemplate {
            id: "builtin_network_diagnose",
            name: "Network Diagnostics",
            description: "Diagnose network connectivity issues",
            category: "system",
            keywords: &[
                "network", "internet", "connection", "connectivity",
                "wifi problem", "can't connect", "network issue", "slow internet",
            ],
            required_vars: &[],
            steps: || {
                vec![
                    tool(
                        "network_interfaces",
                        serde_json::json!({}),
                        "interfaces",
                    ),
                    tool(
                        "network_ping",
                        serde_json::json!({"host": "8.8.8.8"}),
                        "ping_result",
                    ),
                    tool(
                        "network_dns",
                        serde_json::json!({"domain": "google.com"}),
                        "dns_result",
                    ),
                    tool(
                        "wifi_status",
                        serde_json::json!({}),
                        "wifi",
                    ),
                    think(
                        "Diagnose network connectivity:\n\
                         Interfaces: {{interfaces}}\n\
                         Ping to 8.8.8.8: {{ping_result}}\n\
                         DNS lookup: {{dns_result}}\n\
                         WiFi: {{wifi}}\n\n\
                         Diagnosis:\n\
                         - Connection status (Connected / Partial / Down)\n\
                         - Where the problem is (local / DNS / internet)\n\
                         - Specific fix recommendations\n\
                         Be practical and actionable.",
                        "diagnosis",
                    ),
                    notify("Network Diagnostics:\n\n{{diagnosis}}"),
                ]
            },
            trigger: None,
        },
        // 32. Log Analysis
        RecipeTemplate {
            id: "builtin_log_analysis",
            name: "Log Analysis",
            description: "Search system logs for errors and summarize issues",
            category: "system",
            keywords: &[
                "logs", "errors", "log analysis", "what went wrong",
                "error log", "system log", "check logs",
            ],
            required_vars: &[],
            steps: || {
                vec![
                    tool(
                        "run_command",
                        serde_json::json!({"command": "journalctl --no-pager -p err -n 50 2>/dev/null || tail -50 /var/log/syslog 2>/dev/null || echo 'No logs accessible'"}),
                        "error_logs",
                    ),
                    tool(
                        "run_command",
                        serde_json::json!({"command": "dmesg --level=err,warn -T 2>/dev/null | tail -20 || echo 'No dmesg access'"}),
                        "kernel_logs",
                    ),
                    think(
                        "Analyze system logs for issues:\n\
                         Error logs: {{error_logs}}\n\
                         Kernel messages: {{kernel_logs}}\n\n\
                         Summary:\n\
                         - Critical errors (if any)\n\
                         - Recurring warnings\n\
                         - Root cause analysis for each issue\n\
                         - Suggested fixes\n\
                         If no errors found, report system is healthy.",
                        "analysis",
                    ),
                    notify("Log Analysis:\n\n{{analysis}}"),
                ]
            },
            trigger: None,
        },
        // 33. Package Updates
        RecipeTemplate {
            id: "builtin_package_updates",
            name: "Check Package Updates",
            description: "List available system package updates",
            category: "system",
            keywords: &[
                "update", "upgrade", "packages", "outdated",
                "software update", "system update", "patch",
            ],
            required_vars: &[],
            steps: || {
                vec![
                    tool(
                        "package_list",
                        serde_json::json!({"upgradeable": true}),
                        "updates",
                    ),
                    think(
                        "Available package updates: {{updates}}\n\n\
                         Summarize:\n\
                         - Total packages needing updates\n\
                         - Security updates (if identifiable)\n\
                         - Major version upgrades\n\
                         - Recommendation: update now or wait\n\
                         If no updates available, say the system is up to date.",
                        "summary",
                    ),
                    notify("Package Updates:\n\n{{summary}}"),
                ]
            },
            trigger: None,
        },
        // 34. Process Investigation
        RecipeTemplate {
            id: "builtin_process_investigation",
            name: "Process Investigation",
            description: "Investigate high-resource processes and suggest fixes",
            category: "system",
            keywords: &[
                "process", "high cpu", "high memory", "slow",
                "what's using", "resource hog", "investigate process",
            ],
            required_vars: &[],
            steps: || {
                vec![
                    tool(
                        "list_processes",
                        serde_json::json!({"sort": "cpu", "limit": 10}),
                        "by_cpu",
                    ),
                    tool(
                        "list_processes",
                        serde_json::json!({"sort": "memory", "limit": 10}),
                        "by_mem",
                    ),
                    think(
                        "Investigate resource usage:\n\
                         Top by CPU: {{by_cpu}}\n\
                         Top by Memory: {{by_mem}}\n\n\
                         Report:\n\
                         - Processes using unusual amounts of CPU or memory\n\
                         - Whether each is expected (e.g., browser, IDE) or suspicious\n\
                         - Recommendations (kill, restart, investigate further)\n\
                         Do NOT suggest killing system-critical processes.",
                        "investigation",
                    ),
                    notify("Process Investigation:\n\n{{investigation}}"),
                ]
            },
            trigger: None,
        },
        // 35. Code Review Prep
        RecipeTemplate {
            id: "builtin_code_review_prep",
            name: "Code Review Prep",
            description: "Analyze git changes and prepare review notes",
            category: "development",
            keywords: &[
                "code review", "review", "changes", "diff",
                "what changed", "review prep", "pr review",
            ],
            required_vars: &[],
            steps: || {
                vec![
                    tool("git_status", serde_json::json!({}), "status"),
                    tool(
                        "git_diff",
                        serde_json::json!({}),
                        "diff",
                    ),
                    tool(
                        "git_log",
                        serde_json::json!({"limit": 5}),
                        "recent_commits",
                    ),
                    think(
                        "Prepare code review notes:\n\
                         Git status: {{status}}\n\
                         Diff: {{diff}}\n\
                         Recent commits: {{recent_commits}}\n\n\
                         Review:\n\
                         - Summary of changes (what was changed and why)\n\
                         - Potential issues (bugs, edge cases, performance)\n\
                         - Code quality observations\n\
                         - Questions for the author\n\
                         Focus on substance, not style.",
                        "review_notes",
                    ),
                    notify("Code Review Notes:\n\n{{review_notes}}"),
                ]
            },
            trigger: None,
        },
        // 36. Bug Investigation
        RecipeTemplate {
            id: "builtin_bug_investigation",
            name: "Bug Investigation",
            description: "Investigate a bug by checking logs, processes, and recent changes",
            category: "development",
            keywords: &[
                "bug", "debug", "error", "broken", "not working",
                "investigate bug", "fix", "crash",
            ],
            required_vars: &[("description", "Description of the bug or error")],
            steps: || {
                vec![
                    tool(
                        "recall",
                        serde_json::json!({"query": "{{description}} error bug fix"}),
                        "past_fixes",
                    ),
                    tool(
                        "detect_terminal_errors",
                        serde_json::json!({}),
                        "terminal_errors",
                    ),
                    tool(
                        "git_log",
                        serde_json::json!({"limit": 10}),
                        "recent_changes",
                    ),
                    think(
                        "Investigate bug: {{description}}\n\
                         Past related fixes: {{past_fixes}}\n\
                         Terminal errors: {{terminal_errors}}\n\
                         Recent changes: {{recent_changes}}\n\n\
                         Analysis:\n\
                         - Most likely cause based on evidence\n\
                         - Whether recent changes could have introduced this\n\
                         - Suggested fix or next investigation steps\n\
                         Be specific and evidence-based.",
                        "analysis",
                    ),
                    notify("Bug Investigation — {{description}}:\n\n{{analysis}}"),
                ]
            },
            trigger: None,
        },
        // 37. Deploy Checklist
        RecipeTemplate {
            id: "builtin_deploy_checklist",
            name: "Deploy Checklist",
            description: "Pre-deployment verification checklist",
            category: "development",
            keywords: &[
                "deploy", "deployment", "release", "ship",
                "go live", "pre-deploy", "deploy checklist",
            ],
            required_vars: &[],
            steps: || {
                vec![
                    tool("git_status", serde_json::json!({}), "git_status"),
                    tool("git_log", serde_json::json!({"limit": 5}), "recent_commits"),
                    tool("system_info", serde_json::json!({}), "sys"),
                    tool("disk_usage", serde_json::json!({}), "disk"),
                    think(
                        "Pre-deployment checklist:\n\
                         Git status: {{git_status}}\n\
                         Recent commits: {{recent_commits}}\n\
                         System: {{sys}}\n\
                         Disk: {{disk}}\n\n\
                         Checklist:\n\
                         - [ ] Clean git status (no uncommitted changes)\n\
                         - [ ] Recent commits look correct\n\
                         - [ ] System resources adequate\n\
                         - [ ] Sufficient disk space\n\
                         Mark each as PASS or FAIL with details.",
                        "checklist",
                    ),
                    notify("Deploy Checklist:\n\n{{checklist}}"),
                ]
            },
            trigger: None,
        },
        // 38. Git Cleanup
        RecipeTemplate {
            id: "builtin_git_cleanup",
            name: "Git Cleanup",
            description: "Clean up stale git branches and review repository state",
            category: "development",
            keywords: &[
                "git cleanup", "branches", "stale branches",
                "clean git", "git housekeeping", "prune",
            ],
            required_vars: &[],
            steps: || {
                vec![
                    tool(
                        "git_branch",
                        serde_json::json!({"all": true}),
                        "branches",
                    ),
                    tool("git_status", serde_json::json!({}), "status"),
                    tool(
                        "git_stash",
                        serde_json::json!({"action": "list"}),
                        "stashes",
                    ),
                    think(
                        "Review git repository for cleanup:\n\
                         Branches: {{branches}}\n\
                         Status: {{status}}\n\
                         Stashes: {{stashes}}\n\n\
                         Suggest:\n\
                         - Branches safe to delete (merged, old)\n\
                         - Stale stashes to review or drop\n\
                         - Any uncommitted work that should be addressed\n\
                         Do NOT delete anything without confirmation.",
                        "cleanup_plan",
                    ),
                    notify("Git Cleanup Suggestions:\n\n{{cleanup_plan}}"),
                ]
            },
            trigger: None,
        },
        // 39. Malware Scan
        RecipeTemplate {
            id: "builtin_malware_scan",
            name: "Malware Scan",
            description: "Run a full system malware scan and report findings",
            category: "system",
            keywords: &[
                "malware", "virus", "scan", "antivirus",
                "malware scan", "virus scan", "infected",
            ],
            required_vars: &[],
            steps: || {
                vec![
                    tool(
                        "antivirus_update",
                        serde_json::json!({}),
                        "av_update",
                    ),
                    tool(
                        "antivirus_scan",
                        serde_json::json!({"path": "/", "quick": false}),
                        "scan_result",
                    ),
                    think(
                        "Malware scan report:\n\
                         AV update: {{av_update}}\n\
                         Scan result: {{scan_result}}\n\n\
                         Report:\n\
                         - Scan status (clean / threats found)\n\
                         - Any detected threats with details\n\
                         - Actions taken (quarantined, etc.)\n\
                         - Recommendations",
                        "report",
                    ),
                    notify("Malware Scan Report:\n\n{{report}}"),
                ]
            },
            trigger: None,
        },
        // 40. Password Audit
        RecipeTemplate {
            id: "builtin_password_audit",
            name: "Password Audit",
            description: "Review vault entries for weak or duplicate passwords",
            category: "system",
            keywords: &[
                "password", "vault", "passwords", "weak password",
                "password audit", "credential", "security check passwords",
            ],
            required_vars: &[],
            steps: || {
                vec![
                    tool("vault_list", serde_json::json!({}), "vault_entries"),
                    think(
                        "Audit password vault entries: {{vault_entries}}\n\n\
                         Check for:\n\
                         - Entries with weak or short passwords\n\
                         - Duplicate passwords across services\n\
                         - Old entries that may need rotation\n\
                         - Missing entries for common services\n\
                         Do NOT display actual passwords. Only report metadata \
                         and security observations.",
                        "audit",
                    ),
                    notify("Password Audit:\n\n{{audit}}"),
                ]
            },
            trigger: None,
        },
        // 41. Service Status Check
        RecipeTemplate {
            id: "builtin_service_status",
            name: "Service Status",
            description: "Check status of all system services",
            category: "system",
            keywords: &[
                "service", "services", "daemon", "running services",
                "service status", "systemctl", "what's running",
            ],
            required_vars: &[],
            steps: || {
                vec![
                    tool("service_list", serde_json::json!({}), "services"),
                    think(
                        "System services status: {{services}}\n\n\
                         Report:\n\
                         - Running services count\n\
                         - Failed services (if any) with details\n\
                         - Services that might not be needed\n\
                         - Any services that should be running but aren't",
                        "report",
                    ),
                    notify("Service Status:\n\n{{report}}"),
                ]
            },
            trigger: None,
        },
    ]
}
