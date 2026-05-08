//! Minimal repro for cedar-policy issue: `Entities::to_json_value` expands
//! the transitive parent closure rather than preserving the user-declared
//! parents.
//!
//! Expected (per the documented Cedar entity JSON format): each entity's
//! `parents` array contains only the parents the input declared.
//!
//! Actual: `parents` contains the full set of transitive ancestors.
//!
//! Impact: round-trip is lossy — the second serialization can no longer be
//! told apart from a hierarchy where every ancestor was declared directly.
//! This also disagrees with cedar-java, whose serializer preserves declared
//! parents.

use cedar_policy::Entities;

#[test]
fn entities_to_json_expands_transitive_parent_closure() {
    // 3-level hierarchy. Each entity declares exactly ONE parent:
    //   admins
    //     ^
    //   engineers   (parent: admins)
    //     ^
    //   alice       (parent: engineers)
    //
    // `admins` is a transitive ancestor of `alice` but NOT a declared parent.
    let input = r#"[
      { "uid": { "type": "Group", "id": "admins" },    "attrs": {}, "parents": [] },
      { "uid": { "type": "Group", "id": "engineers" }, "attrs": {}, "parents": [ { "type": "Group", "id": "admins" } ] },
      { "uid": { "type": "User",  "id": "alice" },     "attrs": {}, "parents": [ { "type": "Group", "id": "engineers" } ] }
    ]"#;

    let entities = Entities::from_json_str(input, None).expect("parse");
    let output = entities.to_json_value().expect("serialize");

    let alice = output
        .as_array()
        .unwrap()
        .iter()
        .find(|e| e["uid"]["type"] == "User" && e["uid"]["id"] == "alice")
        .expect("alice present in output");

    let parent_ids: Vec<String> = alice["parents"]
        .as_array()
        .unwrap()
        .iter()
        .map(|p| {
            format!(
                "{}::{}",
                p["type"].as_str().unwrap(),
                p["id"].as_str().unwrap()
            )
        })
        .collect();

    // Sort for stable comparison since the underlying collection is a set.
    let mut sorted = parent_ids.clone();
    sorted.sort();

    assert_eq!(
        sorted,
        vec!["Group::engineers".to_string()],
        "Entities::to_json_value should preserve declared parents; \
         instead the transitive ancestor `Group::admins` was added. \
         Got: {sorted:?}"
    );
}
