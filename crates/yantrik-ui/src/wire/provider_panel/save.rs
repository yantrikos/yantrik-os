//! The Providers page's Save: the form, the key into the vault, and the list.

use std::sync::{Arc, Mutex};

use crate::wire::settings::{uuid_short, ProviderStore, ProviderStoreEntry};

/// Save the form: the key into the vault, then the list. Returns the entry as saved, or the
/// sentence for the person (a shut vault among them) with the list and the file as they were.
/// The vault and the way the file is written are given, so the tests can supply their own.
///
/// The vault is asked without the list locked, so the form is applied to the list as it is when
/// the vault has answered, not to a copy taken before: a migration or a removal that happened
/// meanwhile is kept. A migration pass that is moving the same entry's older key waits for this
/// save, or this one waits for it (`provider_vault::key_writes`), so the new key is not written
/// over by the old.
pub(crate) fn save_form_with(
    providers: &Arc<Mutex<ProviderStore>>,
    editing: &str,
    form: Form,
    vault: Option<Arc<dyn crate::provider_vault::KeyVault>>,
    save: &mut dyn FnMut(&mut ProviderStore) -> Result<(), String>,
) -> Result<ProviderStoreEntry, String> {
    let busy = || "The providers could not be read just now.".to_string();
    let _writing = crate::provider_vault::key_writes();
    let id = {
        let store = providers.lock().map_err(|_| busy())?;
        match store.entries.iter().find(|e| !editing.is_empty() && e.id == editing) {
            Some(e) => e.id.clone(),
            None => format!("{}-{}", form.provider_type.to_lowercase(), uuid_short()),
        }
    };
    let typed = form.api_key.clone().filter(|k| !k.trim().is_empty());
    if let Some(key) = &typed {
        let vault = vault.as_deref().ok_or("The credential vault is not ready yet. Try again in a moment.")?;
        crate::provider_vault::keep(vault, &crate::provider_vault::vault_id(&id), key)
            .map_err(|why| crate::provider_vault::KeyError::from(why).to_change(&form.name))?;
    }
    let form = Form { api_key: None, ..form };
    let mut store = providers.lock().map_err(|_| busy())?;
    let before = store.clone();
    if !editing.is_empty() && !store.entries.iter().any(|e| e.id == editing) {
        // Removed while the key was being kept: the key goes too.
        drop(store);
        if let (Some(_), Some(vault)) = (&typed, vault.as_deref()) {
            let _ = vault.remove(&crate::provider_vault::vault_id(&id));
        }
        return Err("That provider was removed while you were editing it.".into());
    }
    let had_ref = store.entries.iter().find(|e| e.id == id).and_then(|e| e.vault_key_id());
    apply_form_as(&mut store, editing, &id, form);
    if let Some(e) = store.entries.iter_mut().find(|e| e.id == id) {
        if typed.is_some() {
            e.api_key_ref = Some(crate::provider_vault::reference(&id));
            e.api_key = None;
            e.unmigrated_key = None;
        }
    }
    if let Err(why) = save(&mut store) {
        *store = before;
        return Err(format!("Not saved: {why}"));
    }
    let saved = store.entries.iter().find(|e| e.id == id).cloned().ok_or_else(busy)?;
    drop(store);
    // A key dropped because the address moved to another host is not left in the vault.
    if let (Some(old), None, Some(vault)) = (had_ref, saved.vault_key_id(), vault.as_deref()) {
        if vault.remove(&old).is_err() {
            tracing::warn!("a provider's old vault entry could not be removed");
        }
    }
    Ok(saved)
}

/// What the panel's form holds when Save is pressed.
pub(crate) struct Form {
    pub(crate) name: String,
    pub(crate) provider_type: String,
    pub(crate) base_url: String,
    pub(crate) api_key: Option<String>,
    pub(crate) auth_type: String,
    pub(crate) model: String,
}

/// Whether a saved key may go to `new_url`: only while it is on the host the key was saved for.
/// Changing the path or the port's scheme on the same host keeps it; a new host does not, so an
/// edited address is never handed a key its person did not type for it.
pub(crate) fn key_follows(saved_url: &str, new_url: &str) -> bool {
    crate::provider_handoff::host(saved_url).eq_ignore_ascii_case(&crate::provider_handoff::host(new_url))
}

/// Write the form into the store: over the entry being edited (keeping its
/// saved key when the key field was left empty and the host is the same one),
/// or as a new entry. A new entry is never the companion's by being the first one
/// saved (#524): adding a provider is adding an option, and "Use for the companion"
/// is the person's own choice. Returns the entry as saved.
pub(super) fn apply_form(store: &mut ProviderStore, editing: &str, form: Form) -> ProviderStoreEntry {
    let id = format!("{}-{}", form.provider_type.to_lowercase(), uuid_short());
    apply_form_as(store, editing, &id, form)
}

/// [`apply_form`], with the id a new entry takes given (the vault entry is already under it).
fn apply_form_as(store: &mut ProviderStore, editing: &str, new_id: &str, form: Form) -> ProviderStoreEntry {
    if let Some(e) = store.entries.iter_mut().find(|e| !editing.is_empty() && e.id == editing) {
        let follows = key_follows(&e.base_url, &form.base_url);
        e.name = form.name;
        e.provider_type = form.provider_type;
        e.base_url = form.base_url;
        match form.api_key {
            Some(k) => e.api_key = Some(k),
            // The key was saved for another host: it is dropped, from the file and the vault.
            None if !follows => {
                e.api_key = None;
                e.api_key_ref = None;
                e.unmigrated_key = None;
            }
            None => {}
        }
        e.auth_type = form.auth_type;
        e.model = form.model;
        return e.clone();
    }
    let entry = ProviderStoreEntry {
        id: new_id.to_string(),
        name: form.name,
        provider_type: form.provider_type,
        base_url: form.base_url,
        api_key: form.api_key,
        api_key_ref: None,
        unmigrated_key: None,
        auth_type: form.auth_type,
        is_primary: false,
        is_fallback: false,
        model: form.model,
    };
    store.entries.push(entry.clone());
    entry
}
