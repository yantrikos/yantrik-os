//! A companion on Yantrik models sends its tools natively, and follows the backend it is moved
//! to and from (#1286).

use std::sync::Arc;

use yantrikdb_core::YantrikDB;
use yantrik_ml::{ApiLLM, ModelCapabilityProfile, ModelFamily};

use crate::companion::CompanionService;
use crate::config::CompanionConfig;

const URL: &str = "http://127.0.0.1:9/v1";

fn gateway() -> Arc<ApiLLM> {
    Arc::new(ApiLLM::new(URL, Some("fake-token".into()), "picked").for_gateway())
}

/// A direct backend whose model has no native-tool profile: a `Generic` one.
fn direct() -> Arc<ApiLLM> {
    Arc::new(ApiLLM::new(URL, None, "some-local-model").for_provider("custom"))
}

fn companion() -> CompanionService {
    let db = YantrikDB::new(":memory:", 384).expect("in-memory database");
    let mut config = CompanionConfig::default();
    config.tools.enabled = true;
    CompanionService::new(db, Arc::new(yantrik_ml::not_set_up::NotSetUpLLM), config)
}

#[test]
fn picked_is_a_generic_model_and_that_is_not_what_decides() {
    assert_eq!(ModelCapabilityProfile::from_model_name("picked").family, ModelFamily::Generic);
    assert!(!ModelCapabilityProfile::from_model_name("picked").uses_native_tools());
    assert!(super::wants_native_tools(&*gateway(), &ModelCapabilityProfile::from_model_name("picked")));
}

#[test]
fn a_gateway_backend_is_native_with_an_empty_prefix_and_a_direct_generic_one_is_not() {
    let mut c = companion();
    c.swap_llm(gateway());
    assert_eq!(c.capability_profile.family, ModelFamily::Generic, "the profile is still picked's");
    assert!(c.use_native_tools && c.tools_system_message.is_empty() && !c.native_core_tools.is_empty());

    let mut d = companion();
    d.swap_llm(direct());
    assert!(!d.use_native_tools, "a direct Generic backend describes its tools");
    assert!(!d.tools_system_message.is_empty());
}

#[test]
fn a_swap_gateway_to_direct_to_gateway_follows_each_time() {
    let mut c = companion();
    for _ in 0..2 {
        c.swap_llm(gateway());
        assert!(c.use_native_tools && c.tools_system_message.is_empty());
        c.swap_llm(direct());
        assert!(!c.use_native_tools && !c.tools_system_message.is_empty());
    }
}
