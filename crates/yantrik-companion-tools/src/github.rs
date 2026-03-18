//! GitHub tools — fetch repos, stars, and profile data via GitHub REST API.
//!
//! Uses the public GitHub API (no auth required for public repos).
//! Rate limit: 60 requests/hour unauthenticated, 5000/hour with token.

use serde_json::Value;
use super::{Tool, ToolContext, ToolRegistry, PermissionLevel};

pub fn register(reg: &mut ToolRegistry, token: Option<&str>) {
    let token = token.map(|t| t.to_string());
    reg.register(Box::new(GitHubReposTool { token: token.clone() }));
    reg.register(Box::new(GitHubStarsTool { token: token.clone() }));
    reg.register(Box::new(GitHubProfileTool { token }));
}

/// Helper: make a GitHub API request.
fn github_api(endpoint: &str, token: &Option<String>) -> Result<Value, String> {
    let url = if endpoint.starts_with("https://") {
        endpoint.to_string()
    } else {
        format!("https://api.github.com{}", endpoint)
    };

    let mut req = ureq::get(&url)
        .set("Accept", "application/vnd.github+json")
        .set("User-Agent", "yantrik-os/0.1");

    if let Some(tok) = token {
        req = req.set("Authorization", &format!("Bearer {}", tok));
    }

    let response = req.call().map_err(|e| format!("GitHub API error: {e}"))?;
    let status = response.status();
    if status == 404 {
        return Err("GitHub user/repo not found".to_string());
    }
    if status == 403 {
        return Err("GitHub API rate limit exceeded. Try again later or add a token.".to_string());
    }

    response.into_json::<Value>()
        .map_err(|e| format!("Failed to parse GitHub response: {e}"))
}

/// Helper: extract username from GitHub URL or plain username.
fn extract_username(input: &str) -> String {
    let input = input.trim().trim_end_matches('/');
    // Handle full URLs: https://github.com/username or https://github.com/username/
    if let Some(rest) = input.strip_prefix("https://github.com/") {
        rest.split('/').next().unwrap_or(rest).to_string()
    } else if let Some(rest) = input.strip_prefix("http://github.com/") {
        rest.split('/').next().unwrap_or(rest).to_string()
    } else {
        // Assume it's a plain username
        input.to_string()
    }
}

// ── GitHub Repos Tool ──

struct GitHubReposTool {
    token: Option<String>,
}

impl Tool for GitHubReposTool {
    fn name(&self) -> &'static str { "github_repos" }
    fn permission(&self) -> PermissionLevel { PermissionLevel::Safe }
    fn category(&self) -> &'static str { "github" }

    fn definition(&self) -> Value {
        serde_json::json!({
            "type": "function",
            "function": {
                "name": "github_repos",
                "description": "List a GitHub user's public repositories with star counts",
                "parameters": {
                    "type": "object",
                    "properties": {
                        "username": {
                            "type": "string",
                            "description": "GitHub username or profile URL (e.g. 'torvalds' or 'https://github.com/torvalds')"
                        },
                        "min_stars": {
                            "type": "integer",
                            "description": "Only show repos with at least this many stars (default: 0)"
                        },
                        "sort": {
                            "type": "string",
                            "enum": ["stars", "updated", "name"],
                            "description": "Sort by: stars (most starred first), updated (most recent), name (alphabetical). Default: stars"
                        }
                    },
                    "required": ["username"]
                }
            }
        })
    }

    fn execute(&self, _ctx: &ToolContext, args: &Value) -> String {
        let username_raw = args.get("username").and_then(|v| v.as_str()).unwrap_or_default();
        let min_stars = args.get("min_stars").and_then(|v| v.as_u64()).unwrap_or(0);
        let sort = args.get("sort").and_then(|v| v.as_str()).unwrap_or("stars");

        if username_raw.is_empty() {
            return "Error: username is required".to_string();
        }

        let username = extract_username(username_raw);

        // Fetch repos (paginated, up to 100 per page)
        let sort_param = match sort {
            "updated" => "updated",
            "name" => "full_name",
            _ => "created", // We'll sort by stars ourselves
        };

        let mut all_repos: Vec<Value> = Vec::new();
        for page in 1..=3 {
            let endpoint = format!(
                "/users/{}/repos?per_page=100&page={}&sort={}&direction=desc",
                username, page, sort_param
            );
            match github_api(&endpoint, &self.token) {
                Ok(Value::Array(repos)) => {
                    if repos.is_empty() {
                        break;
                    }
                    all_repos.extend(repos);
                }
                Ok(_) => break,
                Err(e) => return e,
            }
        }

        if all_repos.is_empty() {
            return format!("No public repositories found for '{}'", username);
        }

        // Filter by min_stars
        let mut repos: Vec<_> = all_repos.iter()
            .filter(|r| {
                let stars = r.get("stargazers_count").and_then(|s| s.as_u64()).unwrap_or(0);
                stars >= min_stars
            })
            .collect();

        // Sort by stars descending (default)
        if sort == "stars" {
            repos.sort_by(|a, b| {
                let sa = b.get("stargazers_count").and_then(|s| s.as_u64()).unwrap_or(0);
                let sb = a.get("stargazers_count").and_then(|s| s.as_u64()).unwrap_or(0);
                sa.cmp(&sb)
            });
        }

        if repos.is_empty() {
            return format!(
                "{} has {} repos but none with {} or more stars.",
                username, all_repos.len(), min_stars
            );
        }

        let total_stars: u64 = all_repos.iter()
            .map(|r| r.get("stargazers_count").and_then(|s| s.as_u64()).unwrap_or(0))
            .sum();

        let mut output = format!(
            "GitHub repos for {} ({} shown, {} total repos, {} total stars):\n\n",
            username, repos.len(), all_repos.len(), total_stars
        );

        for repo in repos.iter().take(30) {
            let name = repo.get("name").and_then(|n| n.as_str()).unwrap_or("?");
            let stars = repo.get("stargazers_count").and_then(|s| s.as_u64()).unwrap_or(0);
            let forks = repo.get("forks_count").and_then(|f| f.as_u64()).unwrap_or(0);
            let lang = repo.get("language").and_then(|l| l.as_str()).unwrap_or("-");
            let desc = repo.get("description").and_then(|d| d.as_str()).unwrap_or("");
            let fork = repo.get("fork").and_then(|f| f.as_bool()).unwrap_or(false);
            let updated = repo.get("updated_at").and_then(|u| u.as_str()).unwrap_or("")
                .split('T').next().unwrap_or("");

            let fork_marker = if fork { " [fork]" } else { "" };
            let desc_short = if desc.len() > 80 { &desc[..desc.floor_char_boundary(80)] } else { desc };

            output.push_str(&format!(
                "  {} — ⭐{} 🍴{} [{}] updated:{}{}\n    {}\n",
                name, stars, forks, lang, updated, fork_marker, desc_short
            ));
        }

        if repos.len() > 30 {
            output.push_str(&format!("  ... and {} more\n", repos.len() - 30));
        }

        output
    }
}

// ── GitHub Stars Tool ──

struct GitHubStarsTool {
    token: Option<String>,
}

impl Tool for GitHubStarsTool {
    fn name(&self) -> &'static str { "github_stars" }
    fn permission(&self) -> PermissionLevel { PermissionLevel::Safe }
    fn category(&self) -> &'static str { "github" }

    fn definition(&self) -> Value {
        serde_json::json!({
            "type": "function",
            "function": {
                "name": "github_stars",
                "description": "Get the star count and details for a specific GitHub",
                "parameters": {
                    "type": "object",
                    "properties": {
                        "repo": {
                            "type": "string",
                            "description": "Repository in 'owner/repo' format or full URL (e.g. 'torvalds/linux' or 'https://github.com/torvalds/linux')"
                        }
                    },
                    "required": ["repo"]
                }
            }
        })
    }

    fn execute(&self, _ctx: &ToolContext, args: &Value) -> String {
        let repo_raw = args.get("repo").and_then(|v| v.as_str()).unwrap_or_default();
        if repo_raw.is_empty() {
            return "Error: repo is required (e.g. 'torvalds/linux')".to_string();
        }

        // Extract owner/repo from URL or plain format
        let repo_path = repo_raw.trim().trim_end_matches('/');
        let repo_path = if let Some(rest) = repo_path.strip_prefix("https://github.com/") {
            rest.to_string()
        } else if let Some(rest) = repo_path.strip_prefix("http://github.com/") {
            rest.to_string()
        } else {
            repo_path.to_string()
        };

        let endpoint = format!("/repos/{}", repo_path);
        match github_api(&endpoint, &self.token) {
            Ok(repo) => {
                let name = repo.get("full_name").and_then(|n| n.as_str()).unwrap_or("?");
                let stars = repo.get("stargazers_count").and_then(|s| s.as_u64()).unwrap_or(0);
                let forks = repo.get("forks_count").and_then(|f| f.as_u64()).unwrap_or(0);
                let watchers = repo.get("subscribers_count").and_then(|w| w.as_u64()).unwrap_or(0);
                let lang = repo.get("language").and_then(|l| l.as_str()).unwrap_or("-");
                let desc = repo.get("description").and_then(|d| d.as_str()).unwrap_or("No description");
                let open_issues = repo.get("open_issues_count").and_then(|i| i.as_u64()).unwrap_or(0);
                let created = repo.get("created_at").and_then(|c| c.as_str()).unwrap_or("")
                    .split('T').next().unwrap_or("");
                let updated = repo.get("updated_at").and_then(|u| u.as_str()).unwrap_or("")
                    .split('T').next().unwrap_or("");
                let license = repo.get("license")
                    .and_then(|l| l.get("spdx_id"))
                    .and_then(|s| s.as_str())
                    .unwrap_or("None");
                let topics = repo.get("topics")
                    .and_then(|t| t.as_array())
                    .map(|arr| arr.iter()
                        .filter_map(|v| v.as_str())
                        .collect::<Vec<_>>()
                        .join(", "))
                    .unwrap_or_default();

                format!(
                    "Repository: {}\nDescription: {}\nStars: {} | Forks: {} | Watchers: {} | Issues: {}\n\
                     Language: {} | License: {}\nCreated: {} | Updated: {}\nTopics: {}",
                    name, desc, stars, forks, watchers, open_issues,
                    lang, license, created, updated,
                    if topics.is_empty() { "none" } else { &topics }
                )
            }
            Err(e) => e,
        }
    }
}

// ── GitHub Profile Tool ──

struct GitHubProfileTool {
    token: Option<String>,
}

impl Tool for GitHubProfileTool {
    fn name(&self) -> &'static str { "github_profile" }
    fn permission(&self) -> PermissionLevel { PermissionLevel::Safe }
    fn category(&self) -> &'static str { "github" }

    fn definition(&self) -> Value {
        serde_json::json!({
            "type": "function",
            "function": {
                "name": "github_profile",
                "description": "Get a GitHub user's profile info: bio, followers, public",
                "parameters": {
                    "type": "object",
                    "properties": {
                        "username": {
                            "type": "string",
                            "description": "GitHub username or profile URL"
                        }
                    },
                    "required": ["username"]
                }
            }
        })
    }

    fn execute(&self, _ctx: &ToolContext, args: &Value) -> String {
        let username_raw = args.get("username").and_then(|v| v.as_str()).unwrap_or_default();
        if username_raw.is_empty() {
            return "Error: username is required".to_string();
        }

        let username = extract_username(username_raw);
        let endpoint = format!("/users/{}", username);

        match github_api(&endpoint, &self.token) {
            Ok(user) => {
                let name = user.get("name").and_then(|n| n.as_str()).unwrap_or("?");
                let login = user.get("login").and_then(|l| l.as_str()).unwrap_or("?");
                let bio = user.get("bio").and_then(|b| b.as_str()).unwrap_or("No bio");
                let company = user.get("company").and_then(|c| c.as_str()).unwrap_or("-");
                let location = user.get("location").and_then(|l| l.as_str()).unwrap_or("-");
                let followers = user.get("followers").and_then(|f| f.as_u64()).unwrap_or(0);
                let following = user.get("following").and_then(|f| f.as_u64()).unwrap_or(0);
                let public_repos = user.get("public_repos").and_then(|p| p.as_u64()).unwrap_or(0);
                let created = user.get("created_at").and_then(|c| c.as_str()).unwrap_or("")
                    .split('T').next().unwrap_or("");

                format!(
                    "GitHub: {} (@{})\nBio: {}\nCompany: {} | Location: {}\n\
                     Followers: {} | Following: {} | Public repos: {}\nJoined: {}",
                    name, login, bio, company, location,
                    followers, following, public_repos, created
                )
            }
            Err(e) => e,
        }
    }
}
