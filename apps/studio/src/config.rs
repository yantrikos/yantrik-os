//! Which backend makes the pixels, and what that choice costs the person's privacy.
//!
//! One file decides it — `~/.config/yantrik/studio.json` — and the decision is not only about
//! where a picture comes from. It is also about whether the prompt leaves the machine, which is
//! why the grade Studio publishes for `generate` is read off this configuration rather than
//! written into the source. The same sentence is an ordinary action on a desktop with a GPU and
//! an action that has to be asked about on a laptop that must hand the prompt to a hosted
//! service, and a grade decided once at startup would keep promising the first after the setting
//! had become the second.

use std::net::IpAddr;
use std::path::{Path, PathBuf};

use serde_json::json;

/// The three backends, spelled the way the config file and `set_backend` spell them.
pub const KINDS: [&str; 3] = ["comfyui", "openai-images", "fake"];

/// Where a ComfyUI server is assumed to be until the person says otherwise. Loopback, because
/// "your own GPU" most often means the machine this is running on.
pub const DEFAULT_COMFY_URL: &str = "http://127.0.0.1:8188";

/// Where an OpenAI-compatible images endpoint is assumed to be.
pub const DEFAULT_OPENAI_URL: &str = "https://api.openai.com/v1";

/// The checkpoint the shipped workflow names. A machine holding a different one says so in the
/// config's `model`, and the workflow is templated with whatever this holds.
pub const DEFAULT_CHECKPOINT: &str = "sd_xl_base_1.0.safetensors";

/// The name of the file that decides. Named in the errors, because "no backend configured" is
/// useless without saying which file would configure one.
pub const FILE_NAME: &str = "studio.json";

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    /// HTTP to a ComfyUI server: `POST /prompt`, poll `/history/<id>`, fetch `/view`.
    ComfyUi,
    /// Any OpenAI-compatible `/v1/images/generations`.
    OpenAiImages,
    /// Draws a deterministic PNG from the prompt's hash. What an unconfigured machine gets.
    Fake,
}

impl Kind {
    /// What a person or a mind would type. Underscores and case are forgiven, because
    /// `openai_images` is what the config looks like when it came from a YAML-ish habit and
    /// refusing it teaches nothing.
    pub fn parse(word: &str) -> Option<Kind> {
        match word.trim().to_ascii_lowercase().replace('_', "-").as_str() {
            "comfyui" | "comfy" | "comfy-ui" => Some(Kind::ComfyUi),
            "openai-images" | "openai" | "openai-image" | "openai-images-api" => {
                Some(Kind::OpenAiImages)
            }
            "fake" | "none" | "placeholder" | "off" => Some(Kind::Fake),
            _ => None,
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Kind::ComfyUi => "comfyui",
            Kind::OpenAiImages => "openai-images",
            Kind::Fake => "fake",
        }
    }
}

/// One backend's settings. The API key is *not* here: only the name of the environment variable
/// that holds it, so the key never lands in a config file, a sidecar, a log line or a `describe`
/// response. It is read at call time and dropped.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Backend {
    pub kind: Kind,
    pub base_url: String,
    pub model: String,
    pub api_key_env: String,
    /// An optional replacement for the shipped ComfyUI graph, for a machine whose sampler or
    /// checkpoint needs a different one. Empty means "use the built-in".
    pub workflow: String,
}

impl Backend {
    pub fn defaults_for(kind: Kind) -> Backend {
        match kind {
            Kind::ComfyUi => Backend {
                kind,
                base_url: DEFAULT_COMFY_URL.to_string(),
                model: DEFAULT_CHECKPOINT.to_string(),
                api_key_env: String::new(),
                workflow: String::new(),
            },
            Kind::OpenAiImages => Backend {
                kind,
                base_url: DEFAULT_OPENAI_URL.to_string(),
                model: String::new(),
                // The variable nearly every OpenAI-compatible service's own docs name. It is a
                // default for the *name*, not for a key.
                api_key_env: "OPENAI_API_KEY".to_string(),
                workflow: String::new(),
            },
            Kind::Fake => Backend {
                kind,
                base_url: String::new(),
                model: "fake".to_string(),
                api_key_env: String::new(),
                workflow: String::new(),
            },
        }
    }

    /// The value of the key this backend would use, read from the environment every time. An
    /// `Err` names the variable rather than its contents.
    pub fn api_key(&self) -> Result<String, String> {
        if self.api_key_env.trim().is_empty() {
            return Err("no environment variable is named to hold the API key".to_string());
        }
        match std::env::var(self.api_key_env.trim()) {
            Ok(value) if !value.trim().is_empty() => Ok(value),
            Ok(_) => Err(format!(
                "the environment variable {} is set but empty",
                self.api_key_env
            )),
            Err(_) => Err(format!(
                "the environment variable {} is not set, so nothing can be sent to {}",
                self.api_key_env, self.base_url
            )),
        }
    }
}

/// The whole configuration, and whether it came from a file at all. `configured` is kept rather
/// than guessed from the kind, because "the fake backend because you chose it" and "the fake
/// backend because nothing was configured" are different things to tell a person.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Config {
    pub backend: Backend,
    pub configured: bool,
    /// Where the pictures go. Empty means the default, `~/Pictures/Studio`.
    pub output_folder: String,
}

impl Config {
    /// What an unconfigured machine runs: the fake backend, honestly labelled. A Studio that
    /// refused to start without a GPU or an API key would be a worse app than one that draws a
    /// placeholder and says that is what it is doing.
    pub fn unconfigured() -> Config {
        Config {
            backend: Backend::defaults_for(Kind::Fake),
            configured: false,
            output_folder: String::new(),
        }
    }

    pub fn load() -> Config {
        let Some(path) = config_path() else { return Config::unconfigured() };
        Config::read(&path)
    }

    /// Parse one config file. Split out from `load` so the parsing can be tested without a home
    /// directory or an environment, which is where every interesting failure is.
    pub fn read(path: &Path) -> Config {
        let Ok(text) = std::fs::read_to_string(path) else {
            // Missing is the normal case, not an error. Unreadable is rare enough that it is
            // not worth distinguishing here; `describe` reports the fallback either way.
            return Config::unconfigured();
        };
        match Config::parse(&text) {
            Ok(config) => config,
            Err(problem) => {
                // The file exists and does not say what it means. Falling back quietly would
                // leave a person who wrote `"kind": "openai"` wondering why their prompts are
                // going nowhere; a log line they can find is the least that is owed.
                tracing::warn!("{} is not usable ({problem}); Studio drew placeholders instead", path.display());
                Config::unconfigured()
            }
        }
    }

    /// The parsing rules, as one answer for a string. A malformed file names the key that is
    /// wrong and the values that would work, because this file is written by hand.
    pub fn parse(text: &str) -> Result<Config, String> {
        let value: serde_json::Value =
            serde_json::from_str(text).map_err(|e| format!("it is not valid JSON ({e})"))?;
        let root = value.as_object().ok_or("the top level is not an object")?;

        // `backend` is the documented shape. A person who wrote the keys at the top level gets
        // the same answer, because both spellings say the same thing and refusing the second
        // one only costs them a round trip.
        let backend = root
            .get("backend")
            .cloned()
            .unwrap_or_else(|| json!(root));
        let backend = backend.as_object().ok_or("`backend` is not an object")?;

        let kind_word = text_of(backend, "kind").or(text_of(backend, "type")).unwrap_or_default();
        let kind = if kind_word.is_empty() {
            Kind::Fake
        } else {
            Kind::parse(&kind_word).ok_or_else(|| {
                format!(
                    "`kind` is \"{kind_word}\", which is not one of {}",
                    KINDS.join(", ")
                )
            })?
        };

        let mut config = Config {
            backend: Backend::defaults_for(kind),
            configured: true,
            output_folder: text_of(root, "output_folder")
                .or(text_of(root, "output"))
                .unwrap_or_default(),
        };
        if let Some(url) = text_of(backend, "base_url").or(text_of(backend, "url")) {
            config.backend.base_url = url.trim_end_matches('/').to_string();
        }
        if let Some(model) = text_of(backend, "model").or(text_of(backend, "checkpoint")) {
            config.backend.model = model;
        }
        if let Some(env_name) = text_of(backend, "api_key_env").or(text_of(backend, "key_env")) {
            config.backend.api_key_env = env_name;
        }
        if let Some(workflow) = text_of(backend, "workflow").or(text_of(backend, "graph")) {
            config.backend.workflow = workflow;
        }

        // A hosted endpoint with no model cannot be asked for anything, and finding that out
        // from a 404 later is worse than finding it out here.
        if kind == Kind::OpenAiImages && config.backend.model.trim().is_empty() {
            return Err("`model` is required for the openai-images backend (for example \"gpt-image-1\")".to_string());
        }
        if kind != Kind::Fake && config.backend.base_url.trim().is_empty() {
            return Err(format!(
                "`base_url` is required for the {} backend",
                kind.as_str()
            ));
        }
        Ok(config)
    }

    /// The JSON this configuration writes back. Round-tripping is tested rather than assumed,
    /// because `set_backend` and hand-editing have to agree.
    pub fn to_json(&self) -> serde_json::Value {
        let mut backend = serde_json::Map::new();
        backend.insert("kind".into(), json!(self.backend.kind.as_str()));
        if !self.backend.base_url.is_empty() {
            backend.insert("base_url".into(), json!(self.backend.base_url));
        }
        if !self.backend.model.is_empty() {
            backend.insert("model".into(), json!(self.backend.model));
        }
        if !self.backend.api_key_env.is_empty() {
            backend.insert("api_key_env".into(), json!(self.backend.api_key_env));
        }
        if !self.backend.workflow.is_empty() {
            backend.insert("workflow".into(), json!(self.backend.workflow));
        }
        let mut root = serde_json::Map::new();
        root.insert("backend".into(), json!(backend));
        if !self.output_folder.is_empty() {
            root.insert("output_folder".into(), json!(self.output_folder));
        }
        json!(root)
    }

    /// Save, atomically, to the path given.
    ///
    /// Written to a temporary file in the same directory and renamed, because a half-written config
    /// is a config that parses as nothing and takes the app back to placeholders on the next start.
    ///
    /// The path arrives rather than being looked up here, for two reasons. `set_backend` has to
    /// write to the place this instance was started with and be able to say when there is nowhere
    /// to write, and a test can point it at a temporary file: `XDG_CONFIG_HOME` is process-wide and
    /// tests in one binary run at the same time, so a test that moved it to check something would
    /// be changing what every other test read.
    pub fn save_to(&self, path: &Path) -> Result<PathBuf, String> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)
                .map_err(|e| format!("{} could not be created ({e})", parent.display()))?;
        }
        let temporary = path.with_extension("json.new");
        let body = serde_json::to_vec_pretty(&self.to_json())
            .map_err(|e| format!("the configuration could not be written ({e})"))?;
        std::fs::write(&temporary, body)
            .map_err(|e| format!("{} could not be written ({e})", temporary.display()))?;
        std::fs::rename(&temporary, &path)
            .map_err(|e| format!("{} could not be put in place ({e})", path.display()))?;
        Ok(path.to_path_buf())
    }
}

fn text_of(object: &serde_json::Map<String, serde_json::Value>, key: &str) -> Option<String> {
    match object.get(key) {
        Some(serde_json::Value::String(text)) => Some(text.clone()),
        // A number in a string's place is what a hand-written config produces often enough to
        // forgive: `{"port": 8188}` is not what is being read here, but `{"model": 1}` should
        // not be silently dropped either.
        Some(serde_json::Value::Number(number)) => Some(number.to_string()),
        Some(serde_json::Value::Null) | None => None,
        Some(other) => Some(other.to_string()),
    }
}

/// `~/.config/yantrik/studio.json`, honouring `XDG_CONFIG_HOME` the way the rest of the OS does.
pub fn config_path() -> Option<PathBuf> {
    let base = match std::env::var_os("XDG_CONFIG_HOME") {
        Some(value) if !value.is_empty() => PathBuf::from(value),
        _ => PathBuf::from(std::env::var_os("HOME")?).join(".config"),
    };
    Some(base.join("yantrik").join(FILE_NAME))
}

/// Everything `describe` and the approval card need in order to say honestly what would happen.
/// Built as data rather than assembled at each call site, so the words in the card and the words
/// in the state cannot drift apart.
#[derive(Clone, Debug)]
pub struct Facts {
    pub kind: &'static str,
    /// Where the pixels are made, in a form a person can check: a URL, or "this machine".
    pub place: String,
    pub model: String,
    pub configured: bool,
    /// What `generate` is graded right now, given this configuration.
    pub grade: &'static str,
    /// Whether the prompt crosses a network the person does not own.
    pub prompt_leaves: bool,
    /// The caveat, when there is one. Empty when the configuration is complete and honest.
    pub note: String,
}

impl Config {
    /// The honest account of this configuration, including the ways it is incomplete.
    pub fn facts(&self, key_is_present: bool) -> Facts {
        let kind = self.backend.kind;
        let on_a_network_you_own = match kind {
            Kind::Fake => true,
            Kind::ComfyUi => stays_on_a_network_you_own(&self.backend.base_url),
            Kind::OpenAiImages => false,
        };
        let prompt_leaves = !on_a_network_you_own;

        let place = match kind {
            Kind::Fake => "this machine".to_string(),
            Kind::ComfyUi if on_a_network_you_own => {
                format!("a ComfyUI server you reach at {}", self.backend.base_url)
            }
            Kind::ComfyUi => {
                format!("a ComfyUI server away from your network at {}", self.backend.base_url)
            }
            Kind::OpenAiImages => format!("a hosted service at {}", self.backend.base_url),
        };

        // The grade follows one rule: does the prompt leave a network the person owns? If it
        // does, the action sends words off this machine and may spend money doing it, and that
        // is `sensitive` however cheap or trustworthy the service is.
        let grade = if prompt_leaves { "sensitive" } else { "standard" };

        let note = if !self.configured {
            format!(
                "No backend is configured, so Studio drew a placeholder picture from your prompt's hash and said so. Write {} or run set_backend kind=comfyui base_url=… to make real images.",
                config_path().map(|p| p.display().to_string()).unwrap_or_else(|| "~/.config/yantrik/studio.json".into())
            )
        } else if kind == Kind::Fake {
            "The placeholder backend is chosen, so every picture is drawn from the prompt's hash. Nothing leaves this machine.".to_string()
        } else if kind == Kind::OpenAiImages && !key_is_present {
            match self.backend.api_key() {
                Ok(_) => String::new(),
                Err(problem) => format!("Prompts cannot be sent yet: {problem}."),
            }
        } else if kind == Kind::OpenAiImages && self.backend.model.trim().is_empty() {
            "No model is named, so the service will be asked for its default.".to_string()
        } else if kind == Kind::ComfyUi {
            let reachable = match self.backend.workflow.as_str() {
                "" => true,
                path => Path::new(path).is_file(),
            };
            if reachable {
                String::new()
            } else {
                format!(
                    "The workflow file {} cannot be read, so the built-in SDXL graph will be used instead.",
                    self.backend.workflow
                )
            }
        } else {
            String::new()
        };

        Facts {
            kind: kind.as_str(),
            place,
            model: self.backend.model.clone(),
            configured: self.configured,
            grade,
            prompt_leaves,
            note,
        }
    }

    /// The state fragment for `describe`. Keys are only included when they say something, so a
    /// glance is a glance: an empty `note` is not a fact worth carrying.
    pub fn state(&self, key_is_present: bool) -> serde_json::Value {
        let facts = self.facts(key_is_present);
        let mut state = serde_json::Map::new();
        state.insert("kind".into(), json!(facts.kind));
        state.insert("where".into(), json!(facts.place));
        state.insert("configured".into(), json!(facts.configured));
        state.insert("generate_is_graded".into(), json!(facts.grade));
        state.insert("prompt_leaves_this_machine".into(), json!(facts.prompt_leaves));
        if !facts.model.is_empty() && facts.kind != "fake" {
            state.insert("model".into(), json!(facts.model));
        }
        if !self.backend.workflow.is_empty() {
            state.insert("workflow".into(), json!(self.backend.workflow));
        }
        // Never the key, and never the variable's contents. The variable's *name* is included
        // because a person has to be able to tell which variable to export, and a name is not a
        // secret. A test asserts the value cannot appear here.
        if facts.kind == "openai-images" {
            state.insert("api_key_env".into(), json!(self.backend.api_key_env));
            state.insert("api_key_is_set".into(), json!(key_is_present));
        }
        if !facts.note.is_empty() {
            state.insert("note".into(), json!(facts.note));
        }
        json!(state)
    }
}

/// Whether a base URL reaches a machine on a network the person owns: loopback, RFC 1918, or
/// link-local. The question the grade turns on, so it is answered by parsing the URL rather than
/// by looking for substrings like "localhost" in it — `http://evil.example/?x=localhost` would
/// pass that test and must not pass this one.
pub fn stays_on_a_network_you_own(base_url: &str) -> bool {
    let Ok(parsed) = url::Url::parse(base_url) else {
        // Not a URL this can read. Treated as away, because guessing "local" from something
        // unparseable is the direction that sends a prompt off-machine without asking.
        return false;
    };
    let Some(host) = parsed.host_str() else { return false };
    match host.trim().trim_matches(|c| c == '[' || c == ']').to_ascii_lowercase().as_str() {
        "" => false,
        "localhost" => true,
        // A name that is not a literal address could resolve anywhere, and resolving it here
        // would be a DNS lookup on the person's behalf at the moment of grading. Left as
        // "away", which only over-asks.
        host => host.parse::<IpAddr>().is_ok_and(is_on_a_network_you_own),
    }
}

fn is_on_a_network_you_own(ip: IpAddr) -> bool {
    match ip {
        IpAddr::V4(v4) => {
            let [a, b, _, _] = v4.octets();
            v4.is_loopback()                 // 127.0.0.0/8
                || a == 10                   // 10.0.0.0/8
                || (a == 172 && (16..=31).contains(&b)) // 172.16.0.0/12
                || (a == 192 && b == 168)    // 192.168.0.0/16
                || (a == 169 && b == 254)    // 169.254.0.0/16, link-local
        }
        IpAddr::V6(v6) => {
            let first = v6.segments()[0];
            v6.is_loopback()                       // ::1
                || (first & 0xfe00) == 0xfc00      // fc00::/7, unique local
                || (first & 0xffc0) == 0xfe80      // fe80::/10, link-local
                // An IPv4-mapped address is judged as the v4 it carries, because a person who
                // wrote ::ffff:192.168.4.35 means their LAN and not the internet.
                || v6.to_ipv4_mapped().is_some_and(|v4| is_on_a_network_you_own(IpAddr::V4(v4)))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn write(text: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "studio-config-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join(FILE_NAME);
        std::fs::write(&path, text).unwrap();
        path
    }

    #[test]
    fn a_machine_with_no_config_file_runs_the_fake_backend_and_says_so() {
        let config = Config::unconfigured();
        assert!(!config.configured);
        assert_eq!(config.backend.kind, Kind::Fake);
        let facts = config.facts(false);
        assert_eq!(facts.kind, "fake");
        assert_eq!(facts.grade, "standard");
        assert!(!facts.prompt_leaves);
        assert!(facts.note.contains("No backend is configured"), "{facts:?}");
        assert!(facts.note.contains(FILE_NAME), "{facts:?}");
    }

    #[test]
    fn the_config_file_the_documentation_describes_parses() {
        let config = Config::parse(
            r#"{"backend": {"kind": "comfyui", "base_url": "http://192.168.4.35:8188",
                "model": "dreamshaperXL_v21.safetensors"}}"#,
        )
        .unwrap();
        assert_eq!(config.backend.kind, Kind::ComfyUi);
        assert_eq!(config.backend.base_url, "http://192.168.4.35:8188");
        assert_eq!(config.backend.model, "dreamshaperXL_v21.safetensors");
        assert!(config.configured);
    }

    #[test]
    fn a_trailing_slash_does_not_make_the_urls_the_app_builds_wrong() {
        let config = Config::parse(r#"{"backend":{"kind":"comfyui","base_url":"http://10.0.0.5:8188/"}}"#).unwrap();
        assert_eq!(config.backend.base_url, "http://10.0.0.5:8188");
    }

    #[test]
    fn a_hosted_backend_needs_a_model_and_says_which_key_it_wants() {
        let problem = Config::parse(r#"{"backend":{"kind":"openai-images"}}"#).unwrap_err();
        assert!(problem.contains("`model`"), "{problem}");

        let config = Config::parse(
            r#"{"backend":{"kind":"openai-images","model":"gpt-image-1","api_key_env":"MY_KEY"}}"#,
        )
        .unwrap();
        assert_eq!(config.backend.api_key_env, "MY_KEY");
        assert_eq!(config.backend.base_url, DEFAULT_OPENAI_URL);
        assert_eq!(config.facts(false).grade, "sensitive");
        assert!(config.facts(false).prompt_leaves);
    }

    #[test]
    fn an_unknown_kind_names_the_ones_that_would_work() {
        let problem = Config::parse(r#"{"backend":{"kind":"midjourney"}}"#).unwrap_err();
        assert!(problem.contains("midjourney"), "{problem}");
        for kind in KINDS {
            assert!(problem.contains(kind), "{problem} does not name {kind}");
        }
    }

    #[test]
    fn broken_json_is_reported_rather_than_swallowed() {
        let problem = Config::parse("{ not json").unwrap_err();
        assert!(problem.contains("JSON"), "{problem}");
        // `read` is the boundary that has to survive a bad file: the app must still open.
        let path = write("{ not json");
        assert_eq!(Config::read(&path), Config::unconfigured());
    }

    #[test]
    fn the_keys_a_person_writes_by_hand_are_forgiven() {
        let config = Config::parse(
            r#"{"output_folder":"/srv/pictures","backend":{"type":"OpenAI_Images","url":"https://llm.example/v1/","checkpoint":"gpt-image-1"}}"#,
        )
        .unwrap();
        assert_eq!(config.backend.kind, Kind::OpenAiImages);
        assert_eq!(config.backend.base_url, "https://llm.example/v1");
        assert_eq!(config.backend.model, "gpt-image-1");
        assert_eq!(config.output_folder, "/srv/pictures");
    }

    #[test]
    fn what_set_backend_writes_is_what_the_file_reader_reads() {
        let config = Config::parse(
            r#"{"backend":{"kind":"openai-images","model":"gpt-image-1","api_key_env":"K","base_url":"https://x.example/v1"}}"#,
        )
        .unwrap();
        assert_eq!(
            Config::parse(&serde_json::to_string(&config.to_json()).unwrap()).unwrap(),
            config
        );
    }

    #[test]
    fn a_server_on_the_lan_is_yours_and_one_on_the_internet_is_not() {
        for local in [
            "http://127.0.0.1:8188",
            "http://localhost:8188",
            "http://192.168.4.35:8188",
            "http://10.8.0.2:8188",
            "http://172.16.0.9:8188",
            "http://172.31.255.254:8188",
            "http://169.254.7.7:8188",
            "http://[::1]:8188",
            "http://[fc00::5]:8188",
            "http://[::ffff:192.168.1.4]:8188",
        ] {
            assert!(stays_on_a_network_you_own(local), "{local} is your own network");
        }
        for away in [
            "https://api.openai.com/v1",
            "http://8.8.8.8:8188",
            "http://172.32.0.1:8188", // just outside 172.16.0.0/12
            "http://11.0.0.1:8188",
            "http://192.169.4.35:8188",
            "http://[2001:4860:4860::8888]:8188",
        ] {
            assert!(!stays_on_a_network_you_own(away), "{away} is not your own network");
        }
    }

    #[test]
    fn a_url_that_only_looks_local_is_not_believed() {
        assert!(!stays_on_a_network_you_own("http://replica.example/?redirect=localhost"));
        assert!(!stays_on_a_network_you_own("https://127.0.0.1.localhost.example:8188"));
        // Not a URL at all: the answer is "away", which asks rather than sends.
        assert!(!stays_on_a_network_you_own(""));
        assert!(!stays_on_a_network_you_own("192.168.1.1"));
        // A hostname that is not a literal address is not resolved in order to grade it.
        assert!(!stays_on_a_network_you_own("http://gpu.home.arpa:8188"));
    }

    #[test]
    fn the_grade_of_generate_follows_the_backend() {
        let comfy_local =
            Config::parse(r#"{"backend":{"kind":"comfyui","base_url":"http://192.168.4.35:8188"}}"#)
                .unwrap();
        assert_eq!(comfy_local.facts(false).grade, "standard");
        assert!(!comfy_local.facts(false).prompt_leaves);

        // The same app, pointed at a ComfyUI that is not on a network the person owns: now the
        // prompt is leaving, and the grade has to follow it.
        let comfy_away =
            Config::parse(r#"{"backend":{"kind":"comfyui","base_url":"https://gpu.example.com"}}"#)
                .unwrap();
        assert_eq!(comfy_away.facts(false).grade, "sensitive");
        assert!(comfy_away.facts(false).prompt_leaves);

        let hosted =
            Config::parse(r#"{"backend":{"kind":"openai-images","model":"gpt-image-1"}}"#).unwrap();
        assert_eq!(hosted.facts(false).grade, "sensitive");

        let fake = Config::parse(r#"{"backend":{"kind":"fake"}}"#).unwrap();
        assert_eq!(fake.facts(false).grade, "standard");
        assert!(!fake.facts(false).prompt_leaves);
    }

    #[test]
    fn a_comfyui_on_the_lan_is_described_as_being_on_the_lan() {
        let config =
            Config::parse(r#"{"backend":{"kind":"comfyui","base_url":"http://192.168.4.35:8188"}}"#)
                .unwrap();
        let facts = config.facts(false);
        assert!(facts.place.contains("192.168.4.35"), "{facts:?}");
        assert!(facts.place.contains("you reach"), "{facts:?}");
        assert_eq!(facts.note, "");
    }

    #[test]
    fn a_hosted_backend_with_no_key_exported_says_which_variable_is_missing() {
        let config = Config::parse(
            r#"{"backend":{"kind":"openai-images","model":"gpt-image-1","api_key_env":"STUDIO_TEST_ABSENT_KEY"}}"#,
        )
        .unwrap();
        std::env::remove_var("STUDIO_TEST_ABSENT_KEY");
        let facts = config.facts(false);
        // The file was complete enough to parse; it is the environment that is missing something,
        // and those two are reported differently on purpose.
        assert!(facts.configured);
        assert_eq!(facts.grade, "sensitive");
        assert!(facts.note.contains("STUDIO_TEST_ABSENT_KEY"), "{facts:?}");
        // The note names the variable, never a value; there is no value to leak here, and the
        // assertion below is the one that would catch a regression that printed one.
        assert!(!facts.note.contains("api_key="), "{facts:?}");
    }

    #[test]
    fn the_state_names_the_variable_holding_the_key_but_never_its_value() {
        let sentinel = "sk-sentinel-that-must-not-appear";
        std::env::set_var("STUDIO_TEST_KEY", sentinel);
        let config = Config::parse(
            r#"{"backend":{"kind":"openai-images","model":"gpt-image-1","api_key_env":"STUDIO_TEST_KEY"}}"#,
        )
        .unwrap();
        let state = serde_json::to_string(&config.state(true)).unwrap();
        assert!(state.contains("STUDIO_TEST_KEY"), "{state}");
        assert!(!state.contains(sentinel), "the key reached describe: {state}");
        assert!(state.contains("\"api_key_is_set\":true"), "{state}");
        std::env::remove_var("STUDIO_TEST_KEY");
    }

    #[test]
    fn a_key_is_read_from_the_environment_and_only_from_there() {
        let backend = Backend {
            api_key_env: "STUDIO_TEST_KEY_2".into(),
            ..Backend::defaults_for(Kind::OpenAiImages)
        };
        std::env::remove_var("STUDIO_TEST_KEY_2");
        let problem = backend.api_key().unwrap_err();
        assert!(problem.contains("STUDIO_TEST_KEY_2"), "{problem}");
        assert!(!problem.contains('='), "the message must not look like a key: {problem}");

        std::env::set_var("STUDIO_TEST_KEY_2", "   ");
        assert!(backend.api_key().unwrap_err().contains("empty"));

        std::env::set_var("STUDIO_TEST_KEY_2", "sk-abc");
        assert_eq!(backend.api_key().unwrap(), "sk-abc");
        std::env::remove_var("STUDIO_TEST_KEY_2");

        let unnamed = Backend { api_key_env: "".into(), ..Backend::defaults_for(Kind::OpenAiImages) };
        assert!(unnamed.api_key().is_err());
    }

    #[test]
    fn a_saved_config_is_read_back_as_the_same_config() {
        // A private XDG_CONFIG_HOME, so the test does not write into the person's own config and
        // does not read whatever is already there.
        //
        // This is the only test in the binary that moves a process-wide variable, and it moves this
        // one: nothing else here asserts on where the configuration lives, and every other test is
        // handed its places (`engine::Places`) instead of looking them up. `HOME` and
        // `XDG_PICTURES_DIR` are read by tests that do assert on them, which is why those are
        // tested against whatever this machine says rather than against a value set here.
        let dir = std::env::temp_dir().join(format!("studio-save-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        std::env::set_var("XDG_CONFIG_HOME", &dir);
        let path = config_path().unwrap();
        assert!(path.starts_with(&dir));

        let mut config = Config::parse(r#"{"backend":{"kind":"comfyui","base_url":"http://192.168.4.35:8188"}}"#)
            .unwrap();
        config.output_folder = "/srv/pictures".into();
        let saved_at = config.save_to(&path).unwrap();
        assert_eq!(saved_at, path);
        assert!(saved_at.is_file());
        assert!(!saved_at.with_extension("json.new").exists(), "the temporary file was left behind");
        assert_eq!(Config::load(), config);
        std::env::remove_var("XDG_CONFIG_HOME");
    }
}
