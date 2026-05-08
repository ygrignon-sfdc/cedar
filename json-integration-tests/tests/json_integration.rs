//! End-to-end JSON integration tests for cedar-policy.
//!
//! Reads scenarios from `$CEDAR_JSON_INTEGRATION_TESTS/tests/<category>/<scenario>/`
//! and asserts the round-trip described in that repo's `SCHEMA.md`:
//! `text/json input -> internal1 -> json (== expected.json) -> internal2`,
//! and `internal1 == internal2`.
//!
//! Skips with a console message when the env var is unset, so this crate is
//! safe to leave wired into the workspace test command.

use cedar_policy::{
    Authorizer, Context, Entities, EntityTypeName, EntityUid, Policy, PolicyId, PolicySet, Request,
    Schema, SchemaFragment, SlotId,
};
use std::collections::HashMap;
use serde::Deserialize;
use std::path::{Path, PathBuf};
use std::str::FromStr;

mod canonical;
mod normalize;

const ENV_VAR: &str = "CEDAR_JSON_INTEGRATION_TESTS";

fn corpus_root() -> Option<PathBuf> {
    std::env::var(ENV_VAR).ok().map(PathBuf::from)
}

#[test]
fn policies_roundtrip() {
    let Some(root) = corpus_root() else {
        eprintln!("skipping: {ENV_VAR} not set");
        return;
    };
    run_category(&root.join("tests/policies"), check_policy_scenario);
}

#[test]
fn entities_roundtrip() {
    let Some(root) = corpus_root() else {
        eprintln!("skipping: {ENV_VAR} not set");
        return;
    };
    run_category(&root.join("tests/entities"), check_entities_scenario);
}

#[test]
fn schemas_roundtrip() {
    let Some(root) = corpus_root() else {
        eprintln!("skipping: {ENV_VAR} not set");
        return;
    };
    run_category(&root.join("tests/schemas"), check_schema_scenario);
}

#[test]
fn residuals_roundtrip() {
    let Some(root) = corpus_root() else {
        eprintln!("skipping: {ENV_VAR} not set");
        return;
    };
    run_category(&root.join("tests/residuals"), check_residual_scenario);
}

fn run_category<F>(category_dir: &Path, check: F)
where
    F: Fn(&Path) -> Result<(), String>,
{
    if !category_dir.is_dir() {
        eprintln!("skipping: {} does not exist", category_dir.display());
        return;
    }
    let mut failures: Vec<(PathBuf, String)> = Vec::new();
    let mut count = 0usize;
    for entry in std::fs::read_dir(category_dir).expect("read category dir") {
        let entry = entry.expect("dir entry");
        let path = entry.path();
        if !path.is_dir() {
            continue;
        }
        if !path.join("expected.json").is_file() {
            continue;
        }
        count += 1;
        if let Err(err) = check(&path) {
            failures.push((path, err));
        }
    }
    assert!(count > 0, "no scenarios found under {}", category_dir.display());
    if !failures.is_empty() {
        let mut msg = format!(
            "{} of {} scenarios failed under {}:\n",
            failures.len(),
            count,
            category_dir.display()
        );
        for (path, err) in failures {
            msg.push_str(&format!("  {}: {}\n", path.display(), err));
        }
        panic!("{msg}");
    }
}

fn read_text(path: &Path) -> Result<String, String> {
    std::fs::read_to_string(path).map_err(|e| format!("reading {}: {e}", path.display()))
}

fn read_canonical_expected(scenario_dir: &Path) -> Result<String, String> {
    let path = scenario_dir.join("expected.json");
    let text = read_text(&path)?;
    canonical::canonicalize_text(&text).map_err(|e| format!("parsing {}: {e}", path.display()))
}

// ---- policies ----

fn check_policy_scenario(scenario_dir: &Path) -> Result<(), String> {
    let policy_text = read_text(&scenario_dir.join("policy.cedar"))?;
    let mut internal1: PolicySet = policy_text.parse().map_err(|e| format!("parse Cedar: {e}"))?;
    apply_links(&mut internal1, scenario_dir)?;
    let mut json1 = internal1
        .clone()
        .to_json()
        .map_err(|e| format!("to_json: {e}"))?;
    normalize::normalize_policy(&mut json1);
    let canonical1 = canonical::to_canonical_string(&json1);

    let canonical_expected = read_canonical_expected(scenario_dir)?;
    if canonical1 != canonical_expected {
        return Err(diff_message("emitted JSON", &canonical1, &canonical_expected));
    }

    let internal2 = PolicySet::from_json_str(&canonical_expected)
        .map_err(|e| format!("PolicySet::from_json_str: {e}"))?;
    if internal1 != internal2 {
        return Err("internal1 != internal2 after round-trip".to_string());
    }
    Ok(())
}

// ---- entities ----

fn check_entities_scenario(scenario_dir: &Path) -> Result<(), String> {
    let input_text = read_text(&scenario_dir.join("entities.input.json"))?;

    // The canonical form for entities is the user-authored declared form
    // (see SCHEMA.md). Cedar Rust's `Entities::to_json_value` emits the
    // transitive parent closure, which is lossy and disagrees with the
    // canonical form, so we don't route the comparison through it.
    //
    // This harness instead checks that:
    //   1. The input parses successfully (`Entities::from_json_str`).
    //   2. Canonicalizing the input matches the committed `expected.json`
    //      (drift detection on the canonicalizer + author-time normalization).
    //   3. `parse(input) deep_eq parse(expected)` (the parser collapses both
    //      to the same internal store).
    let internal1 = Entities::from_json_str(&input_text, None)
        .map_err(|e| format!("from_json_str: {e}"))?;

    let mut input_value: serde_json::Value =
        serde_json::from_str(&input_text).map_err(|e| format!("parse input json: {e}"))?;
    canonical::canonicalize_entities(&mut input_value);
    let canonical_from_input = canonical::to_canonical_string(&input_value);

    let canonical_expected = read_canonical_expected(scenario_dir)?;
    if canonical_from_input != canonical_expected {
        return Err(diff_message(
            "canonicalized input",
            &canonical_from_input,
            &canonical_expected,
        ));
    }

    let internal2 = Entities::from_json_str(&canonical_expected, None)
        .map_err(|e| format!("re-parse expected: {e}"))?;
    if !internal1.deep_eq(&internal2) {
        return Err("Entities::deep_eq returned false".to_string());
    }
    Ok(())
}

// ---- schemas ----

fn check_schema_scenario(scenario_dir: &Path) -> Result<(), String> {
    let schema_text = read_text(&scenario_dir.join("schema.cedarschema"))?;
    let (fragment1, _warnings) = SchemaFragment::from_cedarschema_str(&schema_text)
        .map_err(|e| format!("from_cedarschema_str: {e}"))?;
    let mut json1 = fragment1
        .clone()
        .to_json_value()
        .map_err(|e| format!("to_json_value: {e}"))?;
    normalize::normalize_schema(&mut json1);
    let canonical1 = canonical::to_canonical_string(&json1);

    let canonical_expected = read_canonical_expected(scenario_dir)?;
    if canonical1 != canonical_expected {
        return Err(diff_message("emitted JSON", &canonical1, &canonical_expected));
    }

    let fragment2 = SchemaFragment::from_json_str(&canonical_expected)
        .map_err(|e| format!("re-parse expected: {e}"))?;
    let mut json2 = fragment2
        .to_json_value()
        .map_err(|e| format!("to_json_value (round-trip): {e}"))?;
    normalize::normalize_schema(&mut json2);
    let canonical2 = canonical::to_canonical_string(&json2);
    if canonical1 != canonical2 {
        return Err("schema fragment did not survive JSON round-trip".to_string());
    }
    Ok(())
}

// ---- residuals ----

#[derive(Deserialize)]
struct RequestSpec {
    principal: Slot,
    action: Slot,
    resource: Slot,
    context: Slot,
}

#[derive(Deserialize)]
#[serde(untagged)]
enum Slot {
    Unknown { unknown: bool },
    Concrete(SlotValue),
}

#[derive(Deserialize)]
#[serde(untagged)]
enum SlotValue {
    Entity {
        r#type: String,
        id: String,
    },
    Context(serde_json::Map<String, serde_json::Value>),
}

fn check_residual_scenario(scenario_dir: &Path) -> Result<(), String> {
    let policies_text = read_text(&scenario_dir.join("policies.cedar"))?;
    let pset: PolicySet = policies_text.parse().map_err(|e| format!("parse policies: {e}"))?;

    let schema_text = read_text(&scenario_dir.join("schema.cedarschema"))?;
    let (schema, _) =
        Schema::from_cedarschema_str(&schema_text).map_err(|e| format!("parse schema: {e}"))?;

    let request_text = read_text(&scenario_dir.join("request.json"))?;
    let spec: RequestSpec =
        serde_json::from_str(&request_text).map_err(|e| format!("parse request.json: {e}"))?;

    let entities_path = scenario_dir.join("entities.json");
    let entities = if entities_path.exists() {
        let text = read_text(&entities_path)?;
        Entities::from_json_str(&text, Some(&schema))
            .map_err(|e| format!("parse entities: {e}"))?
    } else {
        Entities::empty()
    };

    let request = build_request(&spec).map_err(|e| format!("build request: {e}"))?;

    let authorizer = Authorizer::new();
    let response = authorizer.is_authorized_partial(&request, &pset, &entities);
    // Use non-trivial residuals to match Cedar Java's
    // `PartialResponse.asPolicySet()` semantics. See the corpus generator's
    // residuals.rs for rationale.
    let residuals: Vec<Policy> = response.nontrivial_residuals().collect();
    let residual_set =
        PolicySet::from_policies(residuals).map_err(|e| format!("residual PolicySet: {e}"))?;
    let mut json1 = residual_set
        .clone()
        .to_json()
        .map_err(|e| format!("to_json: {e}"))?;
    normalize::normalize_policy(&mut json1);
    let canonical1 = canonical::to_canonical_string(&json1);

    let canonical_expected = read_canonical_expected(scenario_dir)?;
    if canonical1 != canonical_expected {
        return Err(diff_message("emitted JSON", &canonical1, &canonical_expected));
    }

    // Round-trip equivalence for residuals is checked at the JSON level
    // rather than via `PolicySet::==`. Reason: Cedar Rust's residual AST
    // uses a first-class `Expr::Unknown` node that prints to JSON as
    // `{"unknown": [...]}`, but parsing that JSON back via
    // `PolicySet::from_json_str` produces an `ExtFuncCall("unknown", ...)`
    // node — semantically equivalent, structurally different. So
    // `from_json(to_json(residual))` is not AST-equal to the original even
    // though it is JSON-equal. We therefore assert that the JSON survives
    // a full parse-and-reemit cycle byte-for-byte, which is what users
    // actually need from a serialization round-trip.
    let internal2 = PolicySet::from_json_str(&canonical_expected)
        .map_err(|e| format!("re-parse expected: {e}"))?;
    let mut json2 = internal2
        .to_json()
        .map_err(|e| format!("to_json (round-trip): {e}"))?;
    normalize::normalize_policy(&mut json2);
    let canonical2 = canonical::to_canonical_string(&json2);
    if canonical1 != canonical2 {
        return Err(format!(
            "residual JSON did not survive round-trip\n--- before ---\n{canonical1}\n--- after ---\n{canonical2}\n"
        ));
    }
    Ok(())
}

fn build_request(spec: &RequestSpec) -> Result<Request, String> {
    let mut builder = Request::builder();
    let mut any_unknown = false;

    match &spec.principal {
        Slot::Unknown { unknown: true } => any_unknown = true,
        Slot::Unknown { unknown: false } => return Err("principal: \"unknown\": false".into()),
        Slot::Concrete(SlotValue::Entity { r#type, id }) => {
            builder = builder.principal(make_uid(r#type, id)?);
        }
        Slot::Concrete(SlotValue::Context(_)) => {
            return Err("principal must be an entity or unknown".into())
        }
    }
    match &spec.action {
        Slot::Unknown { unknown: true } => any_unknown = true,
        Slot::Unknown { unknown: false } => return Err("action: \"unknown\": false".into()),
        Slot::Concrete(SlotValue::Entity { r#type, id }) => {
            builder = builder.action(make_uid(r#type, id)?);
        }
        Slot::Concrete(SlotValue::Context(_)) => {
            return Err("action must be an entity or unknown".into())
        }
    }
    match &spec.resource {
        Slot::Unknown { unknown: true } => any_unknown = true,
        Slot::Unknown { unknown: false } => return Err("resource: \"unknown\": false".into()),
        Slot::Concrete(SlotValue::Entity { r#type, id }) => {
            builder = builder.resource(make_uid(r#type, id)?);
        }
        Slot::Concrete(SlotValue::Context(_)) => {
            return Err("resource must be an entity or unknown".into())
        }
    }
    match &spec.context {
        Slot::Unknown { unknown: true } => any_unknown = true,
        Slot::Unknown { unknown: false } => return Err("context: \"unknown\": false".into()),
        Slot::Concrete(SlotValue::Context(map)) => {
            let value = serde_json::Value::Object(map.clone());
            let ctx = Context::from_json_value(value, None)
                .map_err(|e| format!("context: {e}"))?;
            builder = builder.context(ctx);
        }
        Slot::Concrete(SlotValue::Entity { .. }) => {
            return Err("context must be an object or unknown".into())
        }
    }

    if !any_unknown {
        return Err("at least one slot must be unknown".into());
    }
    Ok(builder.build())
}

fn make_uid(ty: &str, id: &str) -> Result<EntityUid, String> {
    let type_name = EntityTypeName::from_str(ty).map_err(|e| format!("invalid type {ty}: {e}"))?;
    let entity_id =
        cedar_policy::EntityId::from_str(id).map_err(|e| format!("invalid id {id}: {e}"))?;
    Ok(EntityUid::from_type_name_and_id(type_name, entity_id))
}

#[derive(Deserialize)]
struct LinkSpec {
    #[serde(rename = "templateId")]
    template_id: String,
    #[serde(rename = "newId")]
    new_id: String,
    values: HashMap<String, LinkEntityRef>,
}

#[derive(Deserialize)]
struct LinkEntityRef {
    r#type: String,
    id: String,
}

/// If the scenario has a `links.json` file, apply each link to the parsed
/// `PolicySet`. See `tools/generator/src/categories/policies.rs` for the
/// schema. Mirrors the generator so the harness can reproduce template-link
/// scenarios.
fn apply_links(pset: &mut PolicySet, scenario_dir: &Path) -> Result<(), String> {
    let links_path = scenario_dir.join("links.json");
    if !links_path.exists() {
        return Ok(());
    }
    let text = read_text(&links_path)?;
    let specs: Vec<LinkSpec> =
        serde_json::from_str(&text).map_err(|e| format!("parse links.json: {e}"))?;
    for spec in specs {
        let mut vals: HashMap<SlotId, EntityUid> = HashMap::new();
        for (slot, eref) in spec.values {
            let slot_id = match slot.as_str() {
                "?principal" => SlotId::principal(),
                "?resource" => SlotId::resource(),
                other => return Err(format!("unknown slot {other:?}")),
            };
            vals.insert(slot_id, make_uid(&eref.r#type, &eref.id)?);
        }
        pset.link(
            PolicyId::new(&spec.template_id),
            PolicyId::new(&spec.new_id),
            vals,
        )
        .map_err(|e| format!("link template {}: {e}", spec.template_id))?;
    }
    Ok(())
}

fn diff_message(label: &str, actual: &str, expected: &str) -> String {
    format!(
        "{label} did not match expected.\n--- expected ---\n{expected}\n--- actual ---\n{actual}\n"
    )
}
