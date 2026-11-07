//! Velune-level selection ids for model variants an adapter mints from its
//! catalog: one upstream model offered at several serving tiers becomes one
//! entry per tier. The encoding is a Velune convention so the selection
//! round-trips intact through composer state, turn records and migrations;
//! the wire parameter it maps to stays adapter-owned.
use crate::RuntimeKind;

/// Selection id prefix of a Codex speed variant: `<prefix>["<base>","<tier>"]`.
pub const CODEX_SPEED_PREFIX: &str = "velune:codex-speed:";

/// The selection id of `base` served at `tier`.
pub fn codex_speed_model(base: &str, tier: &str) -> String {
    let pair = [base, tier];
    format!(
        "{CODEX_SPEED_PREFIX}{}",
        serde_json::to_string(&pair).expect("a string pair encodes")
    )
}

/// The base model and tier of a Codex speed variant id; `None` for ordinary
/// ids, including any that merely contain the prefix as data.
pub fn split_codex_speed(model_id: &str) -> Option<(String, String)> {
    let encoded = model_id.strip_prefix(CODEX_SPEED_PREFIX)?;
    let (base, tier) = serde_json::from_str::<(String, String)>(encoded).ok()?;
    if base.is_empty() || tier.is_empty() {
        return None;
    }
    Some((base, tier))
}

/// The human-readable name of a native model selection id: a Codex speed
/// variant reads as its base model and tier, anything else is itself.
pub fn display_model_name(kind: RuntimeKind, model_id: &str) -> String {
    if kind == RuntimeKind::Codex166
        && let Some((base, tier)) = split_codex_speed(model_id)
    {
        return format!("{base} · {tier}");
    }
    model_id.to_owned()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn codex_speed_ids_round_trip_unambiguously() {
        let id = codex_speed_model("gpt-5.1-codex", "priority");
        assert_eq!(
            split_codex_speed(&id),
            Some(("gpt-5.1-codex".to_owned(), "priority".to_owned()))
        );
        // A base model id that itself looks like data stays intact.
        let nested = codex_speed_model("gpt~fast", "fast");
        assert_eq!(
            split_codex_speed(&nested),
            Some(("gpt~fast".to_owned(), "fast".to_owned()))
        );
        for ordinary in [
            "gpt-5.1-codex",
            "gpt-5.1-codex~fast",
            "velune:codex-speed:garbage",
        ] {
            assert_eq!(split_codex_speed(ordinary), None, "{ordinary}");
        }
        assert_eq!(
            display_model_name(RuntimeKind::Codex166, &id),
            "gpt-5.1-codex · priority"
        );
        assert_eq!(display_model_name(RuntimeKind::ClaudeCode, &id), id);
    }
}
