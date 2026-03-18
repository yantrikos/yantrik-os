//! Research, travel, and information-gathering recipe templates.

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
        // 18. Deep Research
        RecipeTemplate {
            id: "builtin_deep_research",
            name: "Deep Research",
            description: "Multi-source research on any topic with synthesis",
            category: "research",
            keywords: &[
                "research", "deep dive", "investigate", "learn about",
                "tell me about", "find out", "explore topic",
            ],
            required_vars: &[("topic", "Topic to research")],
            steps: || {
                vec![
                    tool(
                        "recall",
                        serde_json::json!({"query": "{{topic}}"}),
                        "existing_knowledge",
                    ),
                    tool(
                        "web_search",
                        serde_json::json!({"query": "{{topic}}"}),
                        "search1",
                    ),
                    tool(
                        "web_search",
                        serde_json::json!({"query": "{{topic}} latest developments 2026"}),
                        "search2",
                    ),
                    think(
                        "Synthesize research on '{{topic}}':\n\
                         Existing knowledge: {{existing_knowledge}}\n\
                         Search 1: {{search1}}\n\
                         Search 2: {{search2}}\n\n\
                         Provide a comprehensive summary covering:\n\
                         - Key facts and current state\n\
                         - Recent developments\n\
                         - Different perspectives or debates\n\
                         Cite sources where possible. Be factual, not speculative.",
                        "synthesis",
                    ),
                    tool(
                        "remember",
                        serde_json::json!({"content": "Research on {{topic}}: {{synthesis}}"}),
                        "_saved",
                    ),
                    notify("Research — {{topic}}:\n\n{{synthesis}}"),
                ]
            },
            trigger: None,
        },
        // 19. Product Comparison
        RecipeTemplate {
            id: "builtin_product_comparison",
            name: "Product Comparison",
            description: "Compare products or services with pros/cons",
            category: "research",
            keywords: &[
                "compare", "comparison", "versus", "vs", "which is better",
                "product comparison", "should i get", "which one",
            ],
            required_vars: &[("products", "Products or services to compare (e.g., 'iPhone vs Samsung Galaxy')")],
            steps: || {
                vec![
                    tool(
                        "web_search",
                        serde_json::json!({"query": "{{products}} comparison review 2026"}),
                        "comparison",
                    ),
                    tool(
                        "web_search",
                        serde_json::json!({"query": "{{products}} pros cons reddit"}),
                        "opinions",
                    ),
                    think(
                        "Compare: {{products}}\n\
                         Reviews: {{comparison}}\n\
                         User opinions: {{opinions}}\n\n\
                         Create a comparison table with:\n\
                         - Key features side by side\n\
                         - Pros and cons of each\n\
                         - Price comparison\n\
                         - Best for (use case recommendation)\n\
                         Be objective. Use actual data, not speculation.",
                        "analysis",
                    ),
                    notify("Product Comparison — {{products}}:\n\n{{analysis}}"),
                ]
            },
            trigger: None,
        },
        // 20. News Briefing
        RecipeTemplate {
            id: "builtin_news_briefing",
            name: "News Briefing",
            description: "Get latest news on a topic summarized",
            category: "research",
            keywords: &[
                "news", "latest", "headlines", "what's new",
                "current events", "news briefing", "updates on",
            ],
            required_vars: &[("topic", "News topic to search")],
            steps: || {
                vec![
                    tool(
                        "web_search",
                        serde_json::json!({"query": "{{topic}} news today"}),
                        "news1",
                    ),
                    tool(
                        "web_search",
                        serde_json::json!({"query": "{{topic}} latest updates"}),
                        "news2",
                    ),
                    think(
                        "Summarize the latest news on '{{topic}}':\n\
                         Sources 1: {{news1}}\n\
                         Sources 2: {{news2}}\n\n\
                         List the top 3-5 stories with:\n\
                         - Headline\n\
                         - One-sentence summary\n\
                         - Source\n\
                         Only include verified news, not speculation.",
                        "briefing",
                    ),
                    notify("News — {{topic}}:\n\n{{briefing}}"),
                ]
            },
            trigger: None,
        },
        // 21. Tech Evaluation
        RecipeTemplate {
            id: "builtin_tech_evaluation",
            name: "Tech Evaluation",
            description: "Evaluate a technology, framework, or tool for a use case",
            category: "research",
            keywords: &[
                "evaluate", "tech", "framework", "technology",
                "should i use", "tech stack", "tool evaluation",
            ],
            required_vars: &[
                ("technology", "Technology to evaluate"),
                ("use_case", "What you want to use it for"),
            ],
            steps: || {
                vec![
                    tool(
                        "web_search",
                        serde_json::json!({"query": "{{technology}} review pros cons {{use_case}}"}),
                        "reviews",
                    ),
                    tool(
                        "web_search",
                        serde_json::json!({"query": "{{technology}} alternatives comparison"}),
                        "alternatives",
                    ),
                    think(
                        "Evaluate '{{technology}}' for '{{use_case}}':\n\
                         Reviews: {{reviews}}\n\
                         Alternatives: {{alternatives}}\n\n\
                         Cover:\n\
                         - Fit for the use case (good fit / acceptable / poor fit)\n\
                         - Strengths for this use case\n\
                         - Weaknesses / risks\n\
                         - Top 2 alternatives and why they might be better\n\
                         - Recommendation\n\
                         Be practical and specific.",
                        "evaluation",
                    ),
                    notify("Tech Evaluation — {{technology}}:\n\n{{evaluation}}"),
                ]
            },
            trigger: None,
        },
        // 22. Price Check
        RecipeTemplate {
            id: "builtin_price_check",
            name: "Price Check",
            description: "Search current prices for a specific item",
            category: "research",
            keywords: &[
                "price", "cost", "how much", "price check",
                "pricing", "buy", "shop", "deal",
            ],
            required_vars: &[("item", "Item to check prices for")],
            steps: || {
                vec![
                    tool(
                        "web_search",
                        serde_json::json!({"query": "{{item}} price buy 2026"}),
                        "prices",
                    ),
                    think(
                        "Find current prices for '{{item}}':\n\
                         Search results: {{prices}}\n\n\
                         Report:\n\
                         - Price range (low to high)\n\
                         - Where to buy (best deals)\n\
                         - Any ongoing sales or discounts\n\
                         Only report prices you found in search results. \
                         Do NOT make up or guess prices.",
                        "report",
                    ),
                    notify("Price Check — {{item}}:\n\n{{report}}"),
                ]
            },
            trigger: None,
        },
        // 23. Trip Planning
        RecipeTemplate {
            id: "builtin_trip_planning",
            name: "Trip Planning",
            description: "Plan a trip with weather, attractions, restaurants, and packing suggestions",
            category: "research",
            keywords: &[
                "trip", "travel", "vacation", "plan trip", "visit",
                "going to", "travel plan", "itinerary",
            ],
            required_vars: &[
                ("destination", "Where you're going"),
                ("dates", "Travel dates or duration"),
            ],
            steps: || {
                vec![
                    tool(
                        "get_weather",
                        serde_json::json!({"location": "{{destination}}"}),
                        "weather",
                    ),
                    tool(
                        "web_search",
                        serde_json::json!({"query": "{{destination}} top things to do attractions"}),
                        "attractions",
                    ),
                    tool(
                        "web_search",
                        serde_json::json!({"query": "{{destination}} best restaurants local food"}),
                        "restaurants",
                    ),
                    tool(
                        "web_search",
                        serde_json::json!({"query": "{{destination}} travel tips hidden gems"}),
                        "tips",
                    ),
                    think(
                        "Create a trip plan for {{destination}} ({{dates}}):\n\
                         Weather: {{weather}}\n\
                         Attractions: {{attractions}}\n\
                         Restaurants: {{restaurants}}\n\
                         Tips: {{tips}}\n\n\
                         Include:\n\
                         - Weather overview and packing suggestions\n\
                         - Day-by-day itinerary with top attractions\n\
                         - Restaurant recommendations (mix of must-try and local gems)\n\
                         - Practical tips (transport, customs, safety)\n\
                         Keep it actionable.",
                        "plan",
                    ),
                    tool(
                        "remember",
                        serde_json::json!({"content": "Trip plan to {{destination}} ({{dates}}): {{plan}}"}),
                        "_saved",
                    ),
                    notify("Trip Plan — {{destination}}:\n\n{{plan}}"),
                ]
            },
            trigger: None,
        },
        // 24. Restaurant Finder
        RecipeTemplate {
            id: "builtin_restaurant_finder",
            name: "Restaurant Finder",
            description: "Find and recommend restaurants based on cuisine and location",
            category: "research",
            keywords: &[
                "restaurant", "food", "eat", "dinner", "lunch",
                "where to eat", "cuisine", "place to eat",
            ],
            required_vars: &[("cuisine_or_location", "Cuisine type or location (e.g., 'Italian near downtown')")],
            steps: || {
                vec![
                    tool(
                        "web_search",
                        serde_json::json!({"query": "best {{cuisine_or_location}} restaurant recommendations"}),
                        "results",
                    ),
                    tool(
                        "recall",
                        serde_json::json!({"query": "restaurant preferences dietary restrictions favorites"}),
                        "preferences",
                    ),
                    think(
                        "Recommend restaurants for: {{cuisine_or_location}}\n\
                         Search results: {{results}}\n\
                         User preferences: {{preferences}}\n\n\
                         Recommend 3-5 restaurants with:\n\
                         - Name and brief description\n\
                         - Why it's recommended\n\
                         - Price range\n\
                         - Any relevance to user's known preferences\n\
                         Prioritize highly-rated, non-chain options.",
                        "recommendations",
                    ),
                    notify("Restaurant Recommendations:\n\n{{recommendations}}"),
                ]
            },
            trigger: None,
        },
        // 25. Book Recommendation
        RecipeTemplate {
            id: "builtin_book_recommendation",
            name: "Book Recommendation",
            description: "Get personalized book recommendations based on interests",
            category: "research",
            keywords: &[
                "book", "read", "reading", "recommend book",
                "what to read", "book suggestion", "novel",
            ],
            required_vars: &[("interest", "Genre, topic, or what kind of book you're looking for")],
            steps: || {
                vec![
                    tool(
                        "recall",
                        serde_json::json!({"query": "books read liked favorites genres"}),
                        "reading_history",
                    ),
                    tool(
                        "web_search",
                        serde_json::json!({"query": "best {{interest}} books recommended 2025 2026"}),
                        "recommendations",
                    ),
                    think(
                        "Recommend books for: {{interest}}\n\
                         Reading history: {{reading_history}}\n\
                         Search results: {{recommendations}}\n\n\
                         Suggest 3-5 books with:\n\
                         - Title and author\n\
                         - One-sentence pitch (why this book)\n\
                         - Why it matches the user's taste\n\
                         Avoid books the user has already read.",
                        "picks",
                    ),
                    notify("Book Recommendations — {{interest}}:\n\n{{picks}}"),
                ]
            },
            trigger: None,
        },
        // 26. Learning Roadmap
        RecipeTemplate {
            id: "builtin_learning_roadmap",
            name: "Learning Roadmap",
            description: "Create a structured learning plan for a new skill",
            category: "research",
            keywords: &[
                "learn", "roadmap", "how to learn", "study plan",
                "skill", "tutorial", "course", "learning path",
            ],
            required_vars: &[("skill", "Skill or topic to learn")],
            steps: || {
                vec![
                    tool(
                        "recall",
                        serde_json::json!({"query": "{{skill}} experience level knowledge"}),
                        "current_level",
                    ),
                    tool(
                        "web_search",
                        serde_json::json!({"query": "{{skill}} learning roadmap beginner to advanced"}),
                        "roadmap_info",
                    ),
                    tool(
                        "web_search",
                        serde_json::json!({"query": "{{skill}} best free resources tutorials 2026"}),
                        "resources",
                    ),
                    think(
                        "Create a learning roadmap for '{{skill}}':\n\
                         Current level: {{current_level}}\n\
                         Roadmap: {{roadmap_info}}\n\
                         Resources: {{resources}}\n\n\
                         Structure as:\n\
                         1. Foundation (week 1-2): core concepts + resources\n\
                         2. Practice (week 3-4): hands-on projects\n\
                         3. Intermediate (week 5-8): deeper topics\n\
                         4. Advanced (month 3+): mastery topics\n\
                         Include specific free resources (courses, tutorials, docs). \
                         Adjust starting point based on user's current level.",
                        "roadmap",
                    ),
                    tool(
                        "remember",
                        serde_json::json!({"content": "Learning roadmap for {{skill}}: {{roadmap}}"}),
                        "_saved",
                    ),
                    notify("Learning Roadmap — {{skill}}:\n\n{{roadmap}}"),
                ]
            },
            trigger: None,
        },
        // 27. How-To Guide
        RecipeTemplate {
            id: "builtin_how_to_guide",
            name: "How-To Guide",
            description: "Get a step-by-step guide for any task",
            category: "research",
            keywords: &[
                "how to", "how do i", "guide", "tutorial",
                "step by step", "instructions", "walkthrough",
            ],
            required_vars: &[("task", "What you want to learn how to do")],
            steps: || {
                vec![
                    tool(
                        "web_search",
                        serde_json::json!({"query": "how to {{task}} step by step guide"}),
                        "guide_info",
                    ),
                    think(
                        "Create a step-by-step guide for: {{task}}\n\
                         Research: {{guide_info}}\n\n\
                         Write clear, numbered steps. Include:\n\
                         - Prerequisites (what you need before starting)\n\
                         - Step-by-step instructions\n\
                         - Common mistakes to avoid\n\
                         - Expected result\n\
                         Keep each step concise and actionable.",
                        "guide",
                    ),
                    notify("How-To: {{task}}\n\n{{guide}}"),
                ]
            },
            trigger: None,
        },
    ]
}
