//! Live checks for typed `[@][kind:]user` references, mirroring the GUI's `userKindMismatch`.

use std::collections::BTreeMap;

use wsid_core::{DEFAULT_USER_KIND, parse_user_ref};

/// Warns when `input` types a kind for an existing user whose stored kind differs.
///
/// Returns `None` for untyped refs, unknown users (they are created on save), unparsable input
/// (the core validator reports that), and matching kinds.
pub fn user_kind_mismatch(input: &str, kinds: &BTreeMap<String, String>) -> Option<String> {
    let user_ref = parse_user_ref(input).ok()?;
    let requested = user_ref.kind?;
    let actual = kinds.get(&user_ref.username)?;
    (actual != &requested).then(|| {
        format!(
            "warning: @{} is an existing {} user; \"{requested}:\" would be ignored",
            user_ref.username,
            if actual.is_empty() { DEFAULT_USER_KIND } else { actual }
        )
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn kinds() -> BTreeMap<String, String> {
        BTreeMap::from([("rob".to_string(), "jira".to_string()), ("amy".to_string(), "human".to_string())])
    }

    #[test]
    fn untyped_unknown_and_matching_refs_are_quiet() {
        let k = kinds();
        assert_eq!(user_kind_mismatch("rob", &k), None);
        assert_eq!(user_kind_mismatch("@jira:rob", &k), None);
        assert_eq!(user_kind_mismatch("claude:newbie", &k), None);
        assert_eq!(user_kind_mismatch("takenote:rob", &k), None);
        assert_eq!(user_kind_mismatch("claude:", &k), None);
        assert_eq!(user_kind_mismatch("", &k), None);
    }

    #[test]
    fn differing_kind_on_existing_user_warns() {
        let msg = user_kind_mismatch("@human:rob", &kinds()).unwrap();
        assert_eq!(msg, "warning: @rob is an existing jira user; \"human:\" would be ignored");
        assert!(
            user_kind_mismatch("Jira:amy", &kinds())
                .unwrap()
                .contains("existing human user")
        );
    }
}
