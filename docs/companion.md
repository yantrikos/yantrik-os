# The AI Companion

The companion is not a chatbot — it's a proactive AI agent woven into every layer of the OS. It watches your system, learns your patterns, remembers your preferences, and helps before you ask.

## How It Works

The companion runs on a 3-layer architecture:

1. **yantrik-ml** — AI inference layer. Multiple LLM backends (Ollama, OpenAI API, Claude CLI, llama.cpp) with automatic fallback. Also handles embeddings (MiniLM) and voice (Whisper).

2. **yantrikdb-core** — Memory layer. SQLite + HNSW vector search. Stores conversations, facts, preferences, and memories with semantic search. Everything is local.

3. **yantrik-companion** — Agent layer. The brain: instincts, tools, bond system, personality, proactive pipeline, cortex (pattern recognition), and learning.

## Proactive Pipeline

The companion doesn't just respond to you — it proactively surfaces relevant information through a 4-stage pipeline:

```
Detect → Generate → Score → Deliver
```

1. **Detect** — Subscribes to system events (battery low, new email, idle timeout, process crash) and instinct triggers
2. **Generate** — Converts triggers into candidate interventions (what *could* the companion say?)
3. **Score** — Multi-axis scoring: urgency, confidence, interruptibility, novelty, expected value, annoyance risk, historical acceptance rate
4. **Deliver** — Applies delivery policy:
   - **Silence** — Score too low, suppress entirely
   - **Ambient** — Low-priority, show in quiet queue only
   - **Whisper** — Medium-priority, subtle notification card
   - **Badge** — High-priority, persistent badge
   - **Interrupt** — Critical, floating card with sound

The pipeline learns from your feedback: dismissing a notification lowers future scores for that category. Acting on one raises them.

## Instincts

Instincts are the companion's proactive behaviors — things it checks and surfaces on its own. There are 65+ instincts organized by category:

### Core Instincts
| Instinct | What It Does |
|----------|--------------|
| **Check-in** | Morning/evening check-ins based on your schedule |
| **Email Watch** | Monitors email for important messages, triages and alerts |
| **Open Loops Guardian** | Tracks unfinished tasks and conversations, reminds you |
| **Follow-up** | Follows up on topics from earlier conversations |
| **Reminder** | Tracks commitments and deadlines you've mentioned |
| **Morning Brief** | Generates a summary of what's ahead today |
| **Evening Reflection** | End-of-day review of what was accomplished |

### Awareness Instincts
| Instinct | What It Does |
|----------|--------------|
| **Emotional Awareness** | Detects sentiment shifts in your messages |
| **Cognitive Load** | Notices when you seem overwhelmed and suggests breaks |
| **Energy Map** | Learns your energy patterns throughout the day |
| **Self Awareness** | Monitors the companion's own behavior quality |
| **Pattern Surfacing** | Identifies recurring patterns in your behavior |
| **Conflict Alerting** | Flags contradictions between your stated goals and actions |

### Life & Productivity
| Instinct | What It Does |
|----------|--------------|
| **Goal Keeper** | Tracks long-term goals and progress |
| **Decision Lab** | Helps with structured decision-making |
| **Money Mind** | Financial awareness from spending patterns |
| **Health Pulse** | Gentle health and wellness check-ins |
| **Cooking Companion** | Meal suggestions based on preferences and time |
| **Activity Recommender** | Suggests activities based on mood and context |
| **Scheduler** | Helps organize and optimize your schedule |
| **Predictive Workflow** | Anticipates what you'll need next based on patterns |

### Growth & Reflection
| Instinct | What It Does |
|----------|--------------|
| **Growth Mirror** | Reflects personal growth over time |
| **Skill Forge** | Suggests skill development opportunities |
| **Future Self** | Connects current actions to long-term aspirations |
| **Mentor Match** | Surfaces relevant advice from past conversations |
| **Philosophy Companion** | Occasional deeper philosophical reflections |
| **Devil's Advocate** | Challenges assumptions when appropriate |
| **Identity Thread** | Tracks evolving identity and values |

### Social & Relationships
| Instinct | What It Does |
|----------|--------------|
| **Relationship Radar** | Reminds about important people and contacts |
| **Connection Weaver** | Finds connections between different areas of your life |
| **Tradition Keeper** | Remembers and reminds about personal traditions |
| **Local Pulse** | Awareness of local events and community |

### Discovery & Curiosity
| Instinct | What It Does |
|----------|--------------|
| **Curiosity** | Shares interesting facts related to your interests |
| **Wonder Sense** | Cultivates a sense of wonder and exploration |
| **Golden Find** | Surfaces valuable discoveries from your data |
| **Deep Dive** | Suggests deep exploration of topics you've shown interest in |
| **Serendipity** | Creates unexpected but valuable connections |
| **Interest Intelligence** | Tracks and evolves your interest profile |
| **Cultural Radar** | Cultural recommendations and awareness |
| **Myth Buster** | Gently corrects common misconceptions |

### System & Technical
| Instinct | What It Does |
|----------|--------------|
| **News Watch** | Curates news based on your interests |
| **Trend Watch** | Tracks trends relevant to your work/interests |
| **Deal Watch** | Alerts about deals on products you've shown interest in |
| **Smart Updates** | Curates and summarizes system/app updates |
| **Weather Watch** | Proactive weather alerts and forecasts |
| **Automation** | Suggests and manages automated workflows |

### Meta Instincts
| Instinct | What It Does |
|----------|--------------|
| **Silence Reveal** | Explains *why* the companion chose not to speak |
| **Memory Weaver** | Connects old memories with new context |
| **Second Brain** | Externalizes and organizes your knowledge |
| **Legacy Builder** | Captures and preserves important life moments |
| **Dream Keeper** | Tracks aspirations and dreams |
| **Time Capture** | Helps with time awareness and reflection |
| **Context Bridge** | Bridges context between different life domains |
| **Night Owl** | Adjusted behavior during late-night hours |
| **Aftermath** | Post-event reflection and learning |
| **Conversational Callback** | Returns to interesting conversation threads |
| **Question Asking** | Asks thoughtful questions to deepen understanding |
| **Humor** | Appropriate humor based on bond level and context |
| **Debrief Partner** | Structured debriefs after important events |
| **Pattern Breaker** | Challenges unhelpful patterns |
| **Opportunity Scout** | Identifies opportunities based on your goals |

## Tools

The companion has 116+ tools for interacting with the system and the world:

### File & Code
- **files** — Read, write, list, search, move, copy, delete files
- **edit** — Edit file contents with precise changes
- **grep** — Search file contents with regex
- **glob** — Find files by pattern
- **git** — Git operations (status, diff, commit, log, branch)
- **github** — GitHub API integration
- **coder** — Code generation and analysis
- **project** — Project management and scaffolding
- **workspace** — Multi-project workspace management

### System
- **process** — List, kill, monitor processes
- **system** — System info, uptime, hostname
- **disk** — Disk usage, mount points
- **networking** — Network interfaces, connections
- **wifi** — WiFi scanning, connecting
- **bluetooth** — Bluetooth device management
- **firewall** — Firewall rules
- **display** — Screen resolution, brightness
- **package** — Package management (apk)
- **service** — Service management (OpenRC)
- **docker** — Docker container management

### Communication
- **email** — Read, send, search emails (IMAP)
- **telegram** — Telegram messaging
- **whatsapp** — WhatsApp messaging
- **calendar** — Calendar events and scheduling
- **browser** — Browser automation via CDP (Chrome DevTools Protocol)
- **browser_lifecycle** — Browser tab/window management

### AI & Memory
- **memory** — Store and recall memories
- **memory_hygiene** — Clean up and deduplicate memories
- **knowledge** — Knowledge graph queries
- **vault** — Encrypted secret storage
- **vision** — Image analysis and description
- **canvas** — Generate images/diagrams

### Productivity
- **calculator** — Mathematical calculations
- **text** — Text processing (summarize, translate, reformat)
- **encoding** — Base64, URL encoding/decoding
- **archive** — Compress/extract archives
- **clipboard** — Clipboard read/write
- **wallpaper** — Desktop wallpaper management
- **window** — Window management (focus, move, resize)

### Automation & Integration
- **automation** — Create and run automated workflows
- **scheduler** — Schedule tasks for later execution
- **task_queue** — Background task management
- **recipe** — Multi-step workflow recipes
- **spawn_agents** — Delegate subtasks to specialized sub-agents
- **connector** — External service integrations
- **mcp** — Model Context Protocol server integration
- **plugin** — YAML plugin execution
- **discovery** — Service and device discovery

### Specialized
- **antivirus** — File scanning and security checks
- **home_assistant** — Smart home control
- **weather** — Weather queries
- **media** — Media file metadata and control
- **ssh** — SSH connection management
- **terminal** — Terminal command execution
- **terminal_analysis** — Analyze terminal output
- **open_loops** — Track and manage open tasks/threads
- **desktop** — Desktop interactions
- **claude** — Delegate to Claude for complex reasoning
- **artifacts** — Generate and manage rich artifacts

### Permission Levels

Every tool has a permission level:
- **Safe** — Read-only, no side effects (list files, check status)
- **Standard** — Reversible writes (create file, save note)
- **Sensitive** — System changes (install package, modify config)
- **Dangerous** — Destructive/external (delete files, send messages)

The config's `max_permission` setting caps what tools the companion can use without asking.

## Bond System

The relationship between you and the companion evolves over time through 5 levels:

| Level | Name | Description |
|-------|------|-------------|
| 1 | **Stranger** | New relationship. Formal, helpful, cautious. Minimal proactivity. |
| 2 | **Acquaintance** | Getting to know you. Starts remembering preferences. Light proactivity. |
| 3 | **Companion** | Trusted helper. References past conversations. Moderate proactivity. |
| 4 | **Confidant** | Deep trust. Offers unsolicited advice. Challenges assumptions. High proactivity. |
| 5 | **Partner** | Deep partnership. Anticipates needs. Full proactive suite. Humor and personality shine. |

Bond level increases through:
- Quality conversations (not just quantity)
- Following up on commitments
- Useful proactive interventions that are acted on
- Time spent together

Bond level affects:
- How many instincts are active
- Proactive intervention threshold (lower = more proactive at higher bond)
- Personality expression (more humor, more personal at higher bond)
- How much the companion challenges you (only at Confidant+)

## Model-Adaptive Intelligence

The companion automatically detects the capabilities of whatever LLM model you're using and adapts:

| Tier | Model Size | Behavior |
|------|-----------|----------|
| **Tiny** | 0.5-1.5B | Simple tools only, MCQ-style tool selection, minimal context |
| **Small** | 1.5-4B | Family-routed tools, structured JSON tool calls |
| **Medium** | 4-14B | Full tool suite with adaptive selection, structured JSON |
| **Large** | 14B+ | Native function calling, full context, multi-step agent loops |

This means you can run Yantrik OS with anything from the bundled 4B model to a 70B model on a GPU cluster, and it automatically adjusts to get the best results from whatever model is available. GPU acceleration is strongly recommended — even the 4B model runs at 70+ TPS with GPU vs ~1 TPS on CPU-only.

## Memory System

The companion's memory is persistent and grows over time:

- **Conversation memory** — Remembers what you've talked about
- **Fact extraction** — Automatically extracts facts from conversations ("User prefers dark mode", "User's birthday is March 15")
- **Write-time dedup** — Avoids storing duplicate memories (cosine similarity gate)
- **Semantic search** — Finds relevant memories using vector embeddings, not just keyword matching
- **Memory evolution** — Old memories get refined and updated as new information comes in
- **Memory hygiene** — Periodic cleanup of low-confidence or contradicted memories

All memory is stored locally in SQLite with HNSW vector indexes. You can inspect, export, or delete any memory at any time.

## Cortex (Pattern Recognition)

The cortex layer observes patterns across your behavior and creates playbooks:

- **Patterns** — Detected behavioral patterns ("User always checks email first thing Monday morning")
- **Playbooks** — Automated response recipes triggered by patterns
- **Conviction system** — Patterns gain or lose confidence over time based on evidence
- **Reasoner** — Draws conclusions from observed patterns

## YAML Plugins

Extend the companion's capabilities without writing Rust:

```yaml
# ~/.config/yantrik/plugins/devops-tools.yaml
name: "devops-tools"
version: "1.0"
tools:
  - name: "deploy_staging"
    description: "Deploy a branch to staging environment"
    permission: "sensitive"
    category: "devops"
    parameters:
      branch:
        type: "string"
        description: "Git branch to deploy"
        required: true
    command: "cd ~/projects && ./deploy.sh {branch}"

  - name: "check_vpn"
    description: "Check VPN connection status"
    permission: "safe"
    category: "network"
    parameters: {}
    command: "mullvad status"
```

Parameters are automatically sanitized. The companion discovers plugins on startup and makes them available as tools.
