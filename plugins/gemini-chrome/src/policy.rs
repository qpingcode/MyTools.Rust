//! The values Chrome checks before it shows Gemini in Chrome (internally "Glic"). Chrome
//! decides eligibility from the country its variations seed was fetched for, from labs
//! flags, and from per-profile account state; all of them live in plain JSON files that
//! Chrome only reads at startup, so they are rewritten while Chrome is closed.
use serde_json::{Map, Value, json};

const COUNTRY: &str = "us";
const GLIC_ACTION: &str = "kActionSidePanelShowGlic";
const REQUIRED_LABS: &[&str] = &["glic@1", "glic-button-alt-label@1"];
/// Older experiments that break the current Glic UI when left enabled.
const REMOVED_LABS: &[&str] = &[
    "glic-disable-actor-safety-checks",
    "glic-side-panel@1",
    "glic-entrypoint-variations@1",
    "glic-unified-fre-screen@1",
];

/// `User Data/Local State`: browser-wide country, labs flags and cached eligibility.
pub fn patch_local_state(state: &mut Value) {
    let Some(root) = state.as_object_mut() else {
        return;
    };
    // Only a Chinese seed country is replaced; Chrome refreshes this field itself.
    if root.get("variations_country").and_then(Value::as_str) == Some("cn") {
        root.insert("variations_country".into(), json!(COUNTRY));
    }
    // `[chrome version, country]`: the country studies are pinned to.
    let consistency = array(slot(state, &["variations_permanent_consistency_country"]));
    if consistency.len() < 2 {
        consistency.resize(2, json!(""));
    }
    consistency[1] = json!(COUNTRY);
    for key in [
        "variations_safe_seed_permanent_consistency_country",
        "variations_safe_seed_session_consistency_country",
    ] {
        *slot(state, &[key]) = json!(COUNTRY);
    }
    let labs = array(slot(state, &["browser", "enabled_labs_experiments"]));
    labs.retain(|flag| {
        !flag
            .as_str()
            .is_some_and(|flag| REMOVED_LABS.contains(&flag))
    });
    for flag in REQUIRED_LABS {
        push_unique(labs, flag);
    }
    mark_glic_eligible(state);
}

/// `User Data/<profile>/Preferences`: the profile's own and its synced account values.
pub fn patch_preferences(preferences: &mut Value) {
    if !preferences.is_object() {
        return;
    }
    patch_preference_scope(preferences);
    patch_preference_scope(slot(preferences, &["account_values"]));
}

fn patch_preference_scope(scope: &mut Value) {
    // 0 is the `GeminiSettings` policy value that allows Gemini.
    *slot(scope, &["browser", "gemini_settings"]) = json!(0);
    *slot(scope, &["sync", "glic_rollout_eligibility"]) = json!(true);
    for key in [
        "pinned_to_tabstrip",
        "tab_context_enabled",
        "default_tab_context_enabled",
    ] {
        *slot(scope, &["glic", key]) = json!(true);
    }
    // The first-run screen is done and the account is not marked ineligible.
    *slot(scope, &["glic", "completed_fre"]) = json!(1);
    *slot(scope, &["glic", "user_status", "user_status"]) = json!(0);
    push_unique(
        array(slot(scope, &["toolbar", "pinned_actions"])),
        GLIC_ACTION,
    );
}

fn mark_glic_eligible(value: &mut Value) {
    match value {
        Value::Object(map) => {
            for (key, child) in map {
                if key == "is_glic_eligible" {
                    *child = json!(true);
                } else {
                    mark_glic_eligible(child);
                }
            }
        }
        Value::Array(items) => items.iter_mut().for_each(mark_glic_eligible),
        _ => {}
    }
}

/// The value at `path`, creating objects along the way (and `null` at the end).
fn slot<'a>(value: &'a mut Value, path: &[&str]) -> &'a mut Value {
    path.iter().fold(value, |current, key| {
        if !current.is_object() {
            *current = Value::Object(Map::new());
        }
        let map = current.as_object_mut().expect("normalized to an object");
        map.entry(*key).or_insert(Value::Null)
    })
}

fn array(value: &mut Value) -> &mut Vec<Value> {
    if !value.is_array() {
        *value = json!([]);
    }
    value.as_array_mut().expect("normalized to an array")
}

fn push_unique(values: &mut Vec<Value>, expected: &str) {
    if !values.iter().any(|value| value.as_str() == Some(expected)) {
        values.push(json!(expected));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn local_state_targets_the_us_and_glic_flags() {
        let mut state = json!({
            "variations_country": "cn",
            "variations_permanent_consistency_country": ["140.0", "cn"],
            "browser": {"enabled_labs_experiments": ["other@1", "glic-side-panel@1"]},
            "profile": {"info_cache": {"Default": {"is_glic_eligible": false}}},
        });
        patch_local_state(&mut state);
        assert_eq!(state["variations_country"], "us");
        assert_eq!(
            state["variations_permanent_consistency_country"],
            json!(["140.0", "us"])
        );
        assert_eq!(
            state["variations_safe_seed_session_consistency_country"],
            "us"
        );
        assert_eq!(
            state["browser"]["enabled_labs_experiments"],
            json!(["other@1", "glic@1", "glic-button-alt-label@1"])
        );
        assert_eq!(
            state["profile"]["info_cache"]["Default"]["is_glic_eligible"],
            true
        );
        // A second pass changes nothing, so files are only written once.
        let patched = state.clone();
        patch_local_state(&mut state);
        assert_eq!(state, patched);
    }

    #[test]
    fn local_state_keeps_other_seed_countries_and_fills_missing_fields() {
        let mut state = json!({"variations_country": "jp"});
        patch_local_state(&mut state);
        assert_eq!(state["variations_country"], "jp");
        assert_eq!(
            state["variations_permanent_consistency_country"],
            json!(["", "us"])
        );
    }

    #[test]
    fn preferences_enable_glic_for_the_profile_and_the_account() {
        let mut preferences = json!({
            "browser": {"gemini_settings": 1},
            "toolbar": {"pinned_actions": ["kActionShowChromeLabs"]},
        });
        patch_preferences(&mut preferences);
        for scope in [&preferences, &preferences["account_values"]] {
            assert_eq!(scope["browser"]["gemini_settings"], 0);
            assert_eq!(scope["glic"]["completed_fre"], 1);
            assert_eq!(scope["glic"]["user_status"]["user_status"], 0);
            assert_eq!(scope["sync"]["glic_rollout_eligibility"], true);
        }
        assert_eq!(
            preferences["toolbar"]["pinned_actions"],
            json!(["kActionShowChromeLabs", GLIC_ACTION])
        );
        let patched = preferences.clone();
        patch_preferences(&mut preferences);
        assert_eq!(preferences, patched);
    }

    #[test]
    fn files_that_are_not_objects_are_left_alone() {
        let mut value = json!([1, 2]);
        patch_preferences(&mut value);
        patch_local_state(&mut value);
        assert_eq!(value, json!([1, 2]));
    }
}
