//! Which Mind is calling, as a type (#1033): matched on its variant, never on the spelling of a key.

/// A Mind the shell can name, as a type: matched on its variant, never on the spelling of its key.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Identity {
    /// An agent whose token the shell believed.
    Agent(String),
    /// A caller whose ancestry belongs to an attached mind.
    Mind(String),
    /// The built-in companion: every turn of it is this one caller.
    Companion,
}

impl Identity {
    /// The key a rule that tells minds apart stores: `agent:<id>`, `mind:<name>`, `companion`.
    pub fn key(&self) -> String {
        match self {
            Identity::Agent(id) => format!("agent:{id}"),
            Identity::Mind(name) => format!("mind:{name}"),
            Identity::Companion => "companion".to_string(),
        }
    }

    /// The key, when this caller can be told from every other: not the companion, whose turns
    /// share one identity.
    pub fn named_key(&self) -> Option<String> {
        (!matches!(self, Identity::Companion)).then(|| self.key())
    }
}

/// [`mind_id_now`] before it is spelled.
pub fn identity_now() -> Option<Identity> {
    if let Some(agent) = crate::control_agent_terminal::calling_agent() {
        return agent.ok().map(|id| Identity::Agent(id.0));
    }
    let caller = yantrik_app_runtime::control::caller();
    let pid = caller.as_ref().and_then(|c| u32::try_from(c.pid).ok()).filter(|p| *p > 0)?;
    if pid == std::process::id() {
        return Some(Identity::Companion);
    }
    crate::caller_identity::resolve(pid as i32).attached_mind.map(Identity::Mind)
}
