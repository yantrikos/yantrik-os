//! Built-in task templates for common life-assistant scenarios.
//!
//! Each function returns a fully configured [`TaskTemplate`] with realistic
//! data sources, extraction schemas, and ranking rules. These serve as the
//! default set loaded into [`TaskTemplateRegistry`] on construction.

use super::{
    DataSource, FieldSpec, FieldType, RankingConfig, RankingFactor, SortOrder, TaskTemplate,
};

/// Returns all built-in task templates.
pub fn builtin_templates() -> Vec<TaskTemplate> {
    vec![
        find_restaurant_template(),
        find_person_template(),
        find_product_template(),
        find_job_template(),
        find_hotel_template(),
        find_service_template(),
    ]
}

// ── Find Restaurant ─────────────────────────────────────────────────────────

fn find_restaurant_template() -> TaskTemplate {
    TaskTemplate {
        task_type: "find_restaurant".into(),
        name: "Find Restaurant".into(),
        description: "Search for restaurants by cuisine, location, price range, or dietary \
                       requirements. Aggregates results from multiple review platforms and \
                       normalizes ratings for comparison."
            .into(),
        sources: vec![
            DataSource {
                name: "google_maps".into(),
                search_url: "https://www.google.com/maps/search/{query}+restaurants".into(),
                result_selector: Some("div[role='article']".into()),
                priority: 1,
                enabled: true,
            },
            DataSource {
                name: "yelp".into(),
                search_url: "https://www.yelp.com/search?find_desc={query}&find_loc=".into(),
                result_selector: Some("li[class*='container']".into()),
                priority: 2,
                enabled: true,
            },
            DataSource {
                name: "tripadvisor".into(),
                search_url: "https://www.tripadvisor.com/Search?q={query}+restaurant".into(),
                result_selector: Some("div[data-test-target='restaurants-list']".into()),
                priority: 3,
                enabled: false,
            },
        ],
        extraction_schema: vec![
            FieldSpec {
                name: "name".into(),
                field_type: FieldType::Text,
                required: true,
                description: "Restaurant name".into(),
            },
            FieldSpec {
                name: "rating".into(),
                field_type: FieldType::Rating,
                required: true,
                description: "Average customer rating, normalized to 0.0-5.0".into(),
            },
            FieldSpec {
                name: "price_level".into(),
                field_type: FieldType::Text,
                required: false,
                description: "Price level indicator (e.g., $, $$, $$$, $$$$)".into(),
            },
            FieldSpec {
                name: "cuisine".into(),
                field_type: FieldType::Text,
                required: false,
                description: "Cuisine type (e.g., Italian, Mexican, Japanese)".into(),
            },
            FieldSpec {
                name: "address".into(),
                field_type: FieldType::Address,
                required: true,
                description: "Full street address of the restaurant".into(),
            },
            FieldSpec {
                name: "phone".into(),
                field_type: FieldType::Phone,
                required: false,
                description: "Contact phone number".into(),
            },
            FieldSpec {
                name: "hours".into(),
                field_type: FieldType::Text,
                required: false,
                description: "Operating hours (today or full week)".into(),
            },
            FieldSpec {
                name: "review_count".into(),
                field_type: FieldType::Number,
                required: false,
                description: "Total number of customer reviews".into(),
            },
            FieldSpec {
                name: "url".into(),
                field_type: FieldType::Url,
                required: false,
                description: "Link to the restaurant's listing or website".into(),
            },
        ],
        required_fields: vec!["cuisine".into(), "location".into()],
        optional_fields: vec![
            "price_range".into(),
            "dietary".into(),
            "open_now".into(),
            "distance".into(),
        ],
        ranking: RankingConfig {
            factors: vec![
                RankingFactor {
                    field: "rating".into(),
                    weight: 0.5,
                    order: SortOrder::Descending,
                },
                RankingFactor {
                    field: "review_count".into(),
                    weight: 0.3,
                    order: SortOrder::Descending,
                },
                RankingFactor {
                    field: "price_level".into(),
                    weight: 0.2,
                    order: SortOrder::Ascending,
                },
            ],
            default_sort: "rating_desc".into(),
        },
        max_results: 10,
    }
}

// ── Find Person ─────────────────────────────────────────────────────────────

fn find_person_template() -> TaskTemplate {
    TaskTemplate {
        task_type: "find_person".into(),
        name: "Find Person".into(),
        description: "Search for a person by name, company, or role. Aggregates publicly \
                       available professional and social profiles to build a summary."
            .into(),
        sources: vec![
            DataSource {
                name: "google".into(),
                search_url: "https://www.google.com/search?q={query}".into(),
                result_selector: Some("div.g".into()),
                priority: 1,
                enabled: true,
            },
            DataSource {
                name: "linkedin".into(),
                search_url:
                    "https://www.linkedin.com/search/results/all/?keywords={query}".into(),
                result_selector: Some("li.reusable-search__result-container".into()),
                priority: 2,
                enabled: true,
            },
            DataSource {
                name: "github".into(),
                search_url: "https://github.com/search?q={query}&type=users".into(),
                result_selector: Some("div.user-list-item".into()),
                priority: 3,
                enabled: false,
            },
        ],
        extraction_schema: vec![
            FieldSpec {
                name: "full_name".into(),
                field_type: FieldType::Text,
                required: true,
                description: "Full name of the person".into(),
            },
            FieldSpec {
                name: "title".into(),
                field_type: FieldType::Text,
                required: false,
                description: "Current job title or professional headline".into(),
            },
            FieldSpec {
                name: "company".into(),
                field_type: FieldType::Text,
                required: false,
                description: "Current employer or organization".into(),
            },
            FieldSpec {
                name: "location".into(),
                field_type: FieldType::Address,
                required: false,
                description: "City, state, or country of residence".into(),
            },
            FieldSpec {
                name: "profile_url".into(),
                field_type: FieldType::Url,
                required: false,
                description: "URL to the person's primary professional profile".into(),
            },
            FieldSpec {
                name: "social_links".into(),
                field_type: FieldType::Text,
                required: false,
                description: "Other social media or portfolio links (comma-separated)".into(),
            },
            FieldSpec {
                name: "bio".into(),
                field_type: FieldType::Text,
                required: false,
                description: "Short biography or summary from their profile".into(),
            },
        ],
        required_fields: vec!["name".into()],
        optional_fields: vec![
            "company".into(),
            "title".into(),
            "location".into(),
        ],
        ranking: RankingConfig {
            factors: vec![RankingFactor {
                field: "relevance".into(),
                weight: 1.0,
                order: SortOrder::Descending,
            }],
            default_sort: "relevance".into(),
        },
        max_results: 5,
    }
}

// ── Find Product ────────────────────────────────────────────────────────────

fn find_product_template() -> TaskTemplate {
    TaskTemplate {
        task_type: "find_product".into(),
        name: "Find Product".into(),
        description: "Search for products by name, category, or features. Compares prices \
                       and ratings across multiple shopping platforms."
            .into(),
        sources: vec![
            DataSource {
                name: "google_shopping".into(),
                search_url: "https://www.google.com/search?tbm=shop&q={query}".into(),
                result_selector: Some("div.sh-dgr__content".into()),
                priority: 1,
                enabled: true,
            },
            DataSource {
                name: "amazon".into(),
                search_url: "https://www.amazon.com/s?k={query}".into(),
                result_selector: Some("div[data-component-type='s-search-result']".into()),
                priority: 2,
                enabled: true,
            },
            DataSource {
                name: "ebay".into(),
                search_url: "https://www.ebay.com/sch/i.html?_nkw={query}".into(),
                result_selector: Some("li.s-item".into()),
                priority: 3,
                enabled: false,
            },
        ],
        extraction_schema: vec![
            FieldSpec {
                name: "name".into(),
                field_type: FieldType::Text,
                required: true,
                description: "Product name / title".into(),
            },
            FieldSpec {
                name: "price".into(),
                field_type: FieldType::Price,
                required: true,
                description: "Current price with currency".into(),
            },
            FieldSpec {
                name: "rating".into(),
                field_type: FieldType::Rating,
                required: false,
                description: "Average customer rating (0.0-5.0)".into(),
            },
            FieldSpec {
                name: "review_count".into(),
                field_type: FieldType::Number,
                required: false,
                description: "Number of customer reviews".into(),
            },
            FieldSpec {
                name: "url".into(),
                field_type: FieldType::Url,
                required: true,
                description: "Direct link to the product listing".into(),
            },
            FieldSpec {
                name: "availability".into(),
                field_type: FieldType::Text,
                required: false,
                description: "Stock status (e.g., In Stock, Out of Stock, Ships in 3 days)"
                    .into(),
            },
            FieldSpec {
                name: "seller".into(),
                field_type: FieldType::Text,
                required: false,
                description: "Seller or retailer name".into(),
            },
            FieldSpec {
                name: "image_url".into(),
                field_type: FieldType::Url,
                required: false,
                description: "URL to the product image".into(),
            },
        ],
        required_fields: vec!["product_name".into()],
        optional_fields: vec![
            "category".into(),
            "max_price".into(),
            "min_rating".into(),
            "brand".into(),
        ],
        ranking: RankingConfig {
            factors: vec![
                RankingFactor {
                    field: "price".into(),
                    weight: 0.4,
                    order: SortOrder::Ascending,
                },
                RankingFactor {
                    field: "rating".into(),
                    weight: 0.4,
                    order: SortOrder::Descending,
                },
                RankingFactor {
                    field: "review_count".into(),
                    weight: 0.2,
                    order: SortOrder::Descending,
                },
            ],
            default_sort: "relevance".into(),
        },
        max_results: 10,
    }
}

// ── Find Job ────────────────────────────────────────────────────────────────

fn find_job_template() -> TaskTemplate {
    TaskTemplate {
        task_type: "find_job".into(),
        name: "Find Job".into(),
        description: "Search for job listings by title, skills, location, or company. \
                       Aggregates postings from major job boards with salary and \
                       requirements data."
            .into(),
        sources: vec![
            DataSource {
                name: "linkedin_jobs".into(),
                search_url: "https://www.linkedin.com/jobs/search/?keywords={query}".into(),
                result_selector: Some("li.jobs-search-results__list-item".into()),
                priority: 1,
                enabled: true,
            },
            DataSource {
                name: "indeed".into(),
                search_url: "https://www.indeed.com/jobs?q={query}".into(),
                result_selector: Some("div.job_seen_beacon".into()),
                priority: 2,
                enabled: true,
            },
            DataSource {
                name: "glassdoor".into(),
                search_url: "https://www.glassdoor.com/Job/jobs.htm?sc.keyword={query}".into(),
                result_selector: Some("li[data-test='jobListing']".into()),
                priority: 3,
                enabled: false,
            },
        ],
        extraction_schema: vec![
            FieldSpec {
                name: "title".into(),
                field_type: FieldType::Text,
                required: true,
                description: "Job title".into(),
            },
            FieldSpec {
                name: "company".into(),
                field_type: FieldType::Text,
                required: true,
                description: "Hiring company name".into(),
            },
            FieldSpec {
                name: "salary_range".into(),
                field_type: FieldType::Text,
                required: false,
                description: "Salary range (e.g., $80,000-$120,000/year)".into(),
            },
            FieldSpec {
                name: "location".into(),
                field_type: FieldType::Address,
                required: false,
                description: "Job location (city/state or Remote)".into(),
            },
            FieldSpec {
                name: "requirements".into(),
                field_type: FieldType::Text,
                required: false,
                description: "Key requirements or qualifications (summarized)".into(),
            },
            FieldSpec {
                name: "url".into(),
                field_type: FieldType::Url,
                required: true,
                description: "Direct link to the job posting".into(),
            },
            FieldSpec {
                name: "posted_date".into(),
                field_type: FieldType::DateTime,
                required: false,
                description: "When the job was posted".into(),
            },
            FieldSpec {
                name: "employment_type".into(),
                field_type: FieldType::Text,
                required: false,
                description: "Full-time, Part-time, Contract, etc.".into(),
            },
            FieldSpec {
                name: "remote".into(),
                field_type: FieldType::Boolean,
                required: false,
                description: "Whether the position supports remote work".into(),
            },
        ],
        required_fields: vec!["job_title".into()],
        optional_fields: vec![
            "skills".into(),
            "location".into(),
            "salary_min".into(),
            "remote_only".into(),
            "experience_level".into(),
        ],
        ranking: RankingConfig {
            factors: vec![
                RankingFactor {
                    field: "relevance".into(),
                    weight: 0.5,
                    order: SortOrder::Descending,
                },
                RankingFactor {
                    field: "posted_date".into(),
                    weight: 0.3,
                    order: SortOrder::Descending,
                },
                RankingFactor {
                    field: "salary_range".into(),
                    weight: 0.2,
                    order: SortOrder::Descending,
                },
            ],
            default_sort: "relevance".into(),
        },
        max_results: 15,
    }
}

// ── Find Hotel ──────────────────────────────────────────────────────────────

fn find_hotel_template() -> TaskTemplate {
    TaskTemplate {
        task_type: "find_hotel".into(),
        name: "Find Hotel".into(),
        description: "Search for hotels and accommodation by location, dates, and preferences. \
                       Compares prices, ratings, and amenities across booking platforms."
            .into(),
        sources: vec![
            DataSource {
                name: "google_hotels".into(),
                search_url: "https://www.google.com/travel/hotels?q={query}".into(),
                result_selector: Some("div[data-resultid]".into()),
                priority: 1,
                enabled: true,
            },
            DataSource {
                name: "booking_com".into(),
                search_url: "https://www.booking.com/searchresults.html?ss={query}".into(),
                result_selector: Some("div[data-testid='property-card']".into()),
                priority: 2,
                enabled: true,
            },
            DataSource {
                name: "tripadvisor_hotels".into(),
                search_url: "https://www.tripadvisor.com/Search?q={query}+hotels".into(),
                result_selector: Some("div[data-test-target='hotels-list']".into()),
                priority: 3,
                enabled: false,
            },
        ],
        extraction_schema: vec![
            FieldSpec {
                name: "name".into(),
                field_type: FieldType::Text,
                required: true,
                description: "Hotel name".into(),
            },
            FieldSpec {
                name: "price_per_night".into(),
                field_type: FieldType::Price,
                required: true,
                description: "Price per night with currency".into(),
            },
            FieldSpec {
                name: "rating".into(),
                field_type: FieldType::Rating,
                required: false,
                description: "Guest rating normalized to 0.0-5.0".into(),
            },
            FieldSpec {
                name: "amenities".into(),
                field_type: FieldType::Text,
                required: false,
                description: "Key amenities (WiFi, pool, parking, breakfast, etc.)".into(),
            },
            FieldSpec {
                name: "address".into(),
                field_type: FieldType::Address,
                required: false,
                description: "Hotel address or neighborhood".into(),
            },
            FieldSpec {
                name: "url".into(),
                field_type: FieldType::Url,
                required: true,
                description: "Booking or detail page link".into(),
            },
            FieldSpec {
                name: "review_count".into(),
                field_type: FieldType::Number,
                required: false,
                description: "Number of guest reviews".into(),
            },
            FieldSpec {
                name: "star_class".into(),
                field_type: FieldType::Number,
                required: false,
                description: "Hotel star classification (1-5)".into(),
            },
            FieldSpec {
                name: "free_cancellation".into(),
                field_type: FieldType::Boolean,
                required: false,
                description: "Whether free cancellation is available".into(),
            },
        ],
        required_fields: vec!["location".into(), "dates".into()],
        optional_fields: vec![
            "max_price".into(),
            "min_rating".into(),
            "amenities".into(),
            "guests".into(),
            "rooms".into(),
        ],
        ranking: RankingConfig {
            factors: vec![
                RankingFactor {
                    field: "price_per_night".into(),
                    weight: 0.4,
                    order: SortOrder::Ascending,
                },
                RankingFactor {
                    field: "rating".into(),
                    weight: 0.4,
                    order: SortOrder::Descending,
                },
                RankingFactor {
                    field: "review_count".into(),
                    weight: 0.2,
                    order: SortOrder::Descending,
                },
            ],
            default_sort: "price_asc".into(),
        },
        max_results: 10,
    }
}

// ── Find Service ────────────────────────────────────────────────────────────

fn find_service_template() -> TaskTemplate {
    TaskTemplate {
        task_type: "find_service".into(),
        name: "Find Local Service".into(),
        description: "Search for local service providers (plumbers, doctors, dentists, \
                       contractors, etc.) by type and location. Compares ratings, \
                       availability, and specialties."
            .into(),
        sources: vec![
            DataSource {
                name: "google".into(),
                search_url: "https://www.google.com/search?q={query}+near+me".into(),
                result_selector: Some("div.g".into()),
                priority: 1,
                enabled: true,
            },
            DataSource {
                name: "yelp".into(),
                search_url: "https://www.yelp.com/search?find_desc={query}".into(),
                result_selector: Some("li[class*='container']".into()),
                priority: 2,
                enabled: true,
            },
            DataSource {
                name: "google_maps".into(),
                search_url: "https://www.google.com/maps/search/{query}".into(),
                result_selector: Some("div[role='article']".into()),
                priority: 3,
                enabled: true,
            },
        ],
        extraction_schema: vec![
            FieldSpec {
                name: "name".into(),
                field_type: FieldType::Text,
                required: true,
                description: "Business or provider name".into(),
            },
            FieldSpec {
                name: "rating".into(),
                field_type: FieldType::Rating,
                required: false,
                description: "Customer rating (0.0-5.0)".into(),
            },
            FieldSpec {
                name: "phone".into(),
                field_type: FieldType::Phone,
                required: false,
                description: "Contact phone number".into(),
            },
            FieldSpec {
                name: "address".into(),
                field_type: FieldType::Address,
                required: false,
                description: "Business address".into(),
            },
            FieldSpec {
                name: "hours".into(),
                field_type: FieldType::Text,
                required: false,
                description: "Operating hours".into(),
            },
            FieldSpec {
                name: "specialties".into(),
                field_type: FieldType::Text,
                required: false,
                description: "Areas of specialty or services offered".into(),
            },
            FieldSpec {
                name: "review_count".into(),
                field_type: FieldType::Number,
                required: false,
                description: "Total number of reviews".into(),
            },
            FieldSpec {
                name: "url".into(),
                field_type: FieldType::Url,
                required: false,
                description: "Website or listing URL".into(),
            },
            FieldSpec {
                name: "accepts_insurance".into(),
                field_type: FieldType::Boolean,
                required: false,
                description: "Whether the provider accepts insurance (for medical services)"
                    .into(),
            },
        ],
        required_fields: vec!["service_type".into(), "location".into()],
        optional_fields: vec![
            "specialty".into(),
            "insurance".into(),
            "open_now".into(),
            "distance".into(),
        ],
        ranking: RankingConfig {
            factors: vec![
                RankingFactor {
                    field: "rating".into(),
                    weight: 0.5,
                    order: SortOrder::Descending,
                },
                RankingFactor {
                    field: "review_count".into(),
                    weight: 0.3,
                    order: SortOrder::Descending,
                },
                RankingFactor {
                    field: "distance".into(),
                    weight: 0.2,
                    order: SortOrder::Ascending,
                },
            ],
            default_sort: "rating_desc".into(),
        },
        max_results: 10,
    }
}
