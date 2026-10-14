use crate::{
    model::*,
    size::{estimate_account, SizeEstimate},
};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::collections::{BTreeMap, BTreeSet};

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct MigrationHint {
    pub category: String,
    pub target: String,
    pub reason: String,
    pub suggested_test: String,
    pub severity: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct SuggestedTest {
    pub area: String,
    pub target: String,
    pub action: String,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Finding {
    pub id: String,
    pub category: String,
    pub target_type: String,
    pub target_name: String,
    pub path: String,
    pub before: Value,
    pub after: Value,
    pub classification: String,
    pub explanation: String,
    pub migration_hints: Vec<MigrationHint>,
    pub suggested_tests: Vec<SuggestedTest>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct AnalysisReport {
    pub format_version: u32,
    pub project: String,
    pub from: ProgramMetadata,
    pub to: ProgramMetadata,
    pub findings: Vec<Finding>,
    pub normalized_changes: Vec<NormalizedChange>,
    pub migration_hints: Vec<MigrationHint>,
    pub suggested_runtime_tests: Vec<SuggestedTest>,
    pub summary: BTreeMap<String, usize>,
    pub account_sizes: BTreeMap<String, [Option<SizeEstimate>; 2]>,
    pub runtime_executed: bool,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct NormalizedChange {
    pub classification: String,
    pub target_type: String,
    pub target_name: String,
    pub path: String,
    pub before: Value,
    pub after: Value,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProgramMetadata {
    pub name: String,
    pub version: String,
    pub address: Option<String>,
}

impl From<&ProgramModel> for ProgramMetadata {
    fn from(p: &ProgramModel) -> Self {
        Self {
            name: p.name.clone(),
            version: p.version.clone(),
            address: p.address.clone(),
        }
    }
}

struct Collector {
    findings: Vec<Finding>,
}

impl Collector {
    // These arguments map directly to the existing finding fields.
    #[allow(clippy::too_many_arguments)]
    fn add(
        &mut self,
        classification: &str,
        target_type: &str,
        target: &str,
        path: impl Into<String>,
        before: Value,
        after: Value,
        explanation: impl Into<String>,
    ) {
        let path = path.into();
        let explanation = explanation.into();
        let category = match classification {
            "PDA_DERIVATION_CHANGE" => "pda",
            "DISCRIMINATOR_CHANGE" => "discriminator",
            "ACCOUNT_SIZE_CHANGE" => "size",
            "SERIALIZATION_COMPATIBILITY_CHANGE" => "serialization",
            "EVENT_SCHEMA_CHANGE" => "event",
            "ERROR_DEFINITION_CHANGE" | "ERROR_CODE_CHANGE" | "ERROR_MESSAGE_CHANGE" => "error",
            "TYPE_COMPATIBILITY_CHANGE" => "type",
            "ACCOUNT_SCHEMA_CHANGE" => "account",
            "INFORMATIONAL_CHANGE" => "informational",
            _ => "interface",
        };
        let (severity, test) = match classification {
            "DISCRIMINATOR_CHANGE" => ("critical-test", format!("Test old {target_type} discriminator compatibility for {target}")),
            "PDA_DERIVATION_CHANGE" => ("high-test", format!("Verify existing PDA address continuity for {target}")),
            "ACCOUNT_SIZE_CHANGE" => ("high-test", format!("Migrate existing {target} accounts; verify data size and rent backing")),
            "SERIALIZATION_COMPATIBILITY_CHANGE" => ("high-test", format!("Replay existing {target} values through V2 decoding")),
            "ACCOUNT_SCHEMA_CHANGE" => ("high-test", format!("Deserialize and migrate existing {target} accounts; check preserved fields and initialization")),
            "TYPE_COMPATIBILITY_CHANGE" => ("high-test", if before.get("value") == Some(&json!("u64")) && after.get("value") == Some(&json!("u32")) {
                format!("Test existing {target} values above u32::MAX")
            } else { format!("Test existing {target} values against the V2 type") }),
            "EVENT_SCHEMA_CHANGE" => ("medium-test", format!("Check {target} event consumers and indexers")),
            "INTERFACE_CHANGE" => ("medium-test", format!("Replay existing client calls to {target}")),
            "ERROR_CODE_CHANGE" => ("medium-test", format!("Check client error-code handling for {target}")),
            "ERROR_MESSAGE_CHANGE" => ("informational", format!("Review message consumers for {target}")),
            _ => ("informational", format!("Review {target} compatibility")),
        };
        let is_hint = !matches!(
            classification,
            "INFORMATIONAL_CHANGE" | "ERROR_MESSAGE_CHANGE"
        );
        let hint = MigrationHint {
            category: classification.into(),
            target: target.into(),
            reason: explanation.clone(),
            suggested_test: test.clone(),
            severity: severity.into(),
        };
        let suggested = SuggestedTest {
            area: category.into(),
            target: target.into(),
            action: test,
        };
        self.findings.push(Finding {
            id: String::new(),
            category: category.into(),
            target_type: target_type.into(),
            target_name: target.into(),
            path,
            before,
            after,
            classification: classification.into(),
            explanation,
            migration_hints: if is_hint { vec![hint] } else { Vec::new() },
            suggested_tests: if is_hint { vec![suggested] } else { Vec::new() },
        });
    }
}

fn names<'a, T>(a: &'a BTreeMap<String, T>, b: &'a BTreeMap<String, T>) -> BTreeSet<&'a String> {
    a.keys().chain(b.keys()).collect()
}

fn diff_discriminator(
    c: &mut Collector,
    target_type: &str,
    target: &str,
    a: &Option<Vec<u8>>,
    b: &Option<Vec<u8>>,
) {
    if a == b && a.is_some() {
        return;
    }
    c.add(
        if a != b {
            "DISCRIMINATOR_CHANGE"
        } else {
            "INFORMATIONAL_CHANGE"
        },
        target_type,
        target,
        format!("{target}.discriminator"),
        json!(a),
        json!(b),
        if a.is_none() || b.is_none() {
            "Discriminator metadata is missing; static compatibility is unknown"
        } else {
            "Serialized discriminator changed"
        },
    );
}

fn field_names(fields: &[FieldModel]) -> Vec<&str> {
    fields.iter().map(|f| f.name.as_str()).collect()
}

fn diff_fields(
    c: &mut Collector,
    class: &str,
    target_type: &str,
    target: &str,
    prefix: &str,
    a: &[FieldModel],
    b: &[FieldModel],
) {
    let old: BTreeMap<_, _> = a.iter().map(|f| (f.name.clone(), &f.ty)).collect();
    let new: BTreeMap<_, _> = b.iter().map(|f| (f.name.clone(), &f.ty)).collect();
    for name in names(&old, &new) {
        let path = format!("{prefix}.{name}");
        match (old.get(name.as_str()), new.get(name.as_str())) {
            (None, Some(ty)) => c.add(
                class,
                target_type,
                target,
                path,
                Value::Null,
                json!(ty),
                "Field or argument added",
            ),
            (Some(ty), None) => c.add(
                class,
                target_type,
                target,
                path,
                json!(ty),
                Value::Null,
                "Field or argument removed",
            ),
            (Some(x), Some(y)) if x != y => c.add(
                if class == "EVENT_SCHEMA_CHANGE" {
                    class
                } else {
                    "TYPE_COMPATIBILITY_CHANGE"
                },
                target_type,
                target,
                path,
                json!(x),
                json!(y),
                "Field or argument type changed; nested definitions should also be reviewed",
            ),
            _ => {}
        }
    }
    let common_a: Vec<_> = field_names(a)
        .into_iter()
        .filter(|n| new.contains_key(*n))
        .collect();
    let common_b: Vec<_> = field_names(b)
        .into_iter()
        .filter(|n| old.contains_key(*n))
        .collect();
    if common_a != common_b {
        c.add(
            if class == "ACCOUNT_SCHEMA_CHANGE" {
                "SERIALIZATION_COMPATIBILITY_CHANGE"
            } else {
                class
            },
            target_type,
            target,
            format!("{prefix}.order"),
            json!(common_a),
            json!(common_b),
            "Existing field or argument order changed",
        );
    }
}

fn diff_pda(
    c: &mut Collector,
    instruction: &str,
    account: &str,
    a: &Option<PdaModel>,
    b: &Option<PdaModel>,
) {
    if a == b {
        return;
    }
    let target = format!("{instruction}.{account}");
    match (a, b) {
        (Some(x), Some(y)) => {
            if x.seeds.len() != y.seeds.len() {
                c.add(
                    "PDA_DERIVATION_CHANGE",
                    "pda",
                    &target,
                    format!("{target}.seed_count"),
                    json!(x.seeds.len()),
                    json!(y.seeds.len()),
                    "PDA seed count changed",
                );
            }
            for i in 0..x.seeds.len().min(y.seeds.len()) {
                if x.seeds[i] != y.seeds[i] {
                    c.add(
                        "PDA_DERIVATION_CHANGE",
                        "pda",
                        &target,
                        format!("{target}.seeds[{i}]"),
                        json!(x.seeds[i]),
                        json!(y.seeds[i]),
                        "PDA seed order, source, or value changed",
                    );
                }
            }
            if x.program != y.program {
                c.add(
                    "PDA_DERIVATION_CHANGE",
                    "pda",
                    &target,
                    format!("{target}.program"),
                    json!(x.program),
                    json!(y.program),
                    "PDA program override changed",
                );
            }
        }
        _ => c.add(
            "PDA_DERIVATION_CHANGE",
            "pda",
            &target,
            format!("{target}.pda"),
            json!(a),
            json!(b),
            "PDA definition added or removed",
        ),
    }
}

fn diff_instruction(c: &mut Collector, a: &InstructionModel, b: &InstructionModel) {
    let name = &a.name;
    diff_discriminator(c, "instruction", name, &a.discriminator, &b.discriminator);
    diff_fields(
        c,
        "INTERFACE_CHANGE",
        "instruction",
        name,
        &format!("{name}.args"),
        &a.args,
        &b.args,
    );
    let old: BTreeMap<_, _> = a.accounts.iter().map(|v| (v.name.clone(), v)).collect();
    let new: BTreeMap<_, _> = b.accounts.iter().map(|v| (v.name.clone(), v)).collect();
    for account in names(&old, &new) {
        let path = format!("{name}.accounts.{account}");
        match (old.get(account.as_str()), new.get(account.as_str())) {
            (None, Some(v)) => c.add(
                "INTERFACE_CHANGE",
                "instruction",
                name,
                path,
                Value::Null,
                json!(v),
                "Instruction account added",
            ),
            (Some(v), None) => c.add(
                "INTERFACE_CHANGE",
                "instruction",
                name,
                path,
                json!(v),
                Value::Null,
                "Instruction account removed",
            ),
            (Some(x), Some(y)) => {
                for (field, before, after) in [
                    ("signer", x.signer, y.signer),
                    ("writable", x.writable, y.writable),
                    ("optional", x.optional, y.optional),
                ] {
                    if before != after {
                        c.add(
                            "INTERFACE_CHANGE",
                            "instruction",
                            name,
                            format!("{path}.{field}"),
                            json!(before),
                            json!(after),
                            format!("Account {field} requirement changed"),
                        );
                    }
                }
                diff_pda(c, name, account, &x.pda, &y.pda);
            }
            _ => {}
        }
    }
    let common_a: Vec<_> = a
        .accounts
        .iter()
        .map(|v| v.name.as_str())
        .filter(|n| new.contains_key(*n))
        .collect();
    let common_b: Vec<_> = b
        .accounts
        .iter()
        .map(|v| v.name.as_str())
        .filter(|n| old.contains_key(*n))
        .collect();
    if common_a != common_b {
        c.add(
            "INTERFACE_CHANGE",
            "instruction",
            name,
            format!("{name}.accounts.order"),
            json!(common_a),
            json!(common_b),
            "Instruction account order changed",
        );
    }
}

fn diff_enum(c: &mut Collector, name: &str, a: &[EnumVariantModel], b: &[EnumVariantModel]) {
    let old: BTreeMap<_, _> = a
        .iter()
        .enumerate()
        .map(|(i, v)| (v.name.clone(), (i, v)))
        .collect();
    let new: BTreeMap<_, _> = b
        .iter()
        .enumerate()
        .map(|(i, v)| (v.name.clone(), (i, v)))
        .collect();
    for variant in names(&old, &new) {
        let path = format!("{name}.{variant}");
        match (old.get(variant.as_str()), new.get(variant.as_str())) {
            (None, Some((index, v))) => c.add(
                "SERIALIZATION_COMPATIBILITY_CHANGE",
                "enum",
                name,
                path,
                Value::Null,
                json!({"index":index,"fields":v.fields}),
                "Enum variant added",
            ),
            (Some((index, v)), None) => c.add(
                "SERIALIZATION_COMPATIBILITY_CHANGE",
                "enum",
                name,
                path,
                json!({"index":index,"fields":v.fields}),
                Value::Null,
                "Enum variant removed",
            ),
            (Some((i, x)), Some((j, y))) => {
                if i != j {
                    c.add(
                        "SERIALIZATION_COMPATIBILITY_CHANGE",
                        "enum",
                        name,
                        format!("{path}.index"),
                        json!(i),
                        json!(j),
                        "Serialized variant index changed",
                    );
                }
                diff_fields(
                    c,
                    "SERIALIZATION_COMPATIBILITY_CHANGE",
                    "enum",
                    name,
                    &path,
                    &x.fields,
                    &y.fields,
                );
            }
            _ => {}
        }
    }
}

pub fn analyze(
    project: &str,
    from: &ProgramModel,
    to: &ProgramModel,
    mappings: &[(String, String)],
) -> AnalysisReport {
    let mut c = Collector {
        findings: Vec::new(),
    };
    if from.address != to.address {
        c.add(
            "INTERFACE_CHANGE",
            "program",
            project,
            "program.address",
            json!(from.address),
            json!(to.address),
            "Program ID changed; existing accounts remain owned by the old program",
        );
    }
    for name in names(&from.instructions, &to.instructions) {
        match (from.instructions.get(name), to.instructions.get(name)) {
            (Some(a), Some(b)) => diff_instruction(&mut c, a, b),
            (a, b) => c.add(
                "INTERFACE_CHANGE",
                "instruction",
                name,
                name.as_str(),
                json!(a),
                json!(b),
                "Instruction added or removed",
            ),
        }
    }
    let paired: BTreeMap<_, _> = mappings.iter().cloned().collect();
    let mapped_old: BTreeSet<_> = paired.keys().cloned().collect();
    let mapped_new: BTreeSet<_> = paired.values().cloned().collect();
    let mut pairs = paired;
    for name in from.accounts.keys().filter(|n| {
        !mapped_old.contains(*n) && to.accounts.contains_key(*n) && !mapped_new.contains(*n)
    }) {
        pairs.insert(name.clone(), name.clone());
    }
    let mut sizes = BTreeMap::new();
    for (old_name, new_name) in &pairs {
        let (Some(a), Some(b)) = (from.accounts.get(old_name), to.accounts.get(new_name)) else {
            continue;
        };
        let target = if old_name == new_name {
            old_name.clone()
        } else {
            format!("{old_name} -> {new_name}")
        };
        diff_discriminator(
            &mut c,
            "account",
            &target,
            &a.discriminator,
            &b.discriminator,
        );
        diff_fields(
            &mut c,
            "ACCOUNT_SCHEMA_CHANGE",
            "account",
            &target,
            &format!("account.{target}"),
            &a.fields,
            &b.fields,
        );
        let sa = estimate_account(a, &from.types);
        let sb = estimate_account(b, &to.types);
        if sa != sb {
            c.add("ACCOUNT_SIZE_CHANGE","account",&target,format!("account.{target}.size"),json!(sa),json!(sb),
            match (sa.bytes,sb.bytes) { (Some(x),Some(y)) if y>x => "Exact serialized size increased; existing accounts may require reallocation", (Some(x),Some(y)) if y<x => "Exact serialized size decreased; check trailing bytes and migration policy", _ => "Static size estimate changed; exact allocation needs runtime validation" });
        }
        sizes.insert(target, [Some(sa), Some(sb)]);
    }
    for name in from.accounts.keys().filter(|n| !pairs.contains_key(*n)) {
        c.add(
            "ACCOUNT_SCHEMA_CHANGE",
            "account",
            name,
            name.as_str(),
            json!(from.accounts.get(name)),
            Value::Null,
            "Account type removed",
        );
    }
    for name in to
        .accounts
        .keys()
        .filter(|n| !pairs.values().any(|v| v == *n))
    {
        c.add(
            "ACCOUNT_SCHEMA_CHANGE",
            "account",
            name,
            name.as_str(),
            Value::Null,
            json!(to.accounts.get(name)),
            "Account type added",
        );
    }
    for name in names(&from.types, &to.types) {
        if pairs.contains_key(name) || pairs.values().any(|mapped| mapped == name) {
            continue;
        }
        match (from.types.get(name), to.types.get(name)) {
            (Some(TypeModel::Enum { variants: a }), Some(TypeModel::Enum { variants: b })) => {
                diff_enum(&mut c, name, a, b)
            }
            (Some(a), Some(b)) if a != b => c.add(
                "TYPE_COMPATIBILITY_CHANGE",
                "type",
                name,
                name.as_str(),
                json!(a),
                json!(b),
                "Custom type definition changed",
            ),
            (None, Some(_)) | (Some(_), None) => c.add(
                "TYPE_COMPATIBILITY_CHANGE",
                "type",
                name,
                name.as_str(),
                json!(from.types.get(name)),
                json!(to.types.get(name)),
                format!(
                    "Custom type {}",
                    if from.types.contains_key(name) {
                        "removed"
                    } else {
                        "added"
                    }
                ),
            ),
            _ => {}
        }
    }
    for name in names(&from.events, &to.events) {
        match (from.events.get(name), to.events.get(name)) {
            (Some(a), Some(b)) => {
                diff_discriminator(&mut c, "event", name, &a.discriminator, &b.discriminator);
                diff_fields(
                    &mut c,
                    "EVENT_SCHEMA_CHANGE",
                    "event",
                    name,
                    &format!("event.{name}"),
                    &a.fields,
                    &b.fields,
                );
            }
            (a, b) => c.add(
                "EVENT_SCHEMA_CHANGE",
                "event",
                name,
                name.as_str(),
                json!(a),
                json!(b),
                "Event added or removed",
            ),
        }
    }
    for name in names(&from.errors, &to.errors) {
        match (from.errors.get(name), to.errors.get(name)) {
            (Some(a), Some(b)) => {
                if a.code != b.code {
                    c.add(
                        "ERROR_CODE_CHANGE",
                        "error",
                        name,
                        format!("error.{name}.code"),
                        json!(a.code),
                        json!(b.code),
                        "Custom error code changed",
                    );
                }
                if a.message != b.message {
                    c.add(
                        "ERROR_MESSAGE_CHANGE",
                        "error",
                        name,
                        format!("error.{name}.message"),
                        json!(a.message),
                        json!(b.message),
                        "Custom error message changed",
                    );
                }
            }
            (a, b) => c.add(
                "ERROR_DEFINITION_CHANGE",
                "error",
                name,
                name.as_str(),
                json!(a),
                json!(b),
                "Custom error added or removed",
            ),
        }
    }
    c.findings.sort_by(|a, b| {
        (&a.category, &a.target_type, &a.target_name, &a.path).cmp(&(
            &b.category,
            &b.target_type,
            &b.target_name,
            &b.path,
        ))
    });
    for (i, f) in c.findings.iter_mut().enumerate() {
        f.id = format!("STATIC-{:04}", i + 1);
    }
    let mut hints = BTreeMap::new();
    let mut tests = BTreeMap::new();
    let mut summary = BTreeMap::new();
    for f in &c.findings {
        *summary.entry(f.category.clone()).or_insert(0) += 1;
        for h in &f.migration_hints {
            hints.insert(
                (
                    h.category.clone(),
                    h.target.clone(),
                    h.suggested_test.clone(),
                ),
                h.clone(),
            );
        }
        for t in &f.suggested_tests {
            tests.insert(
                (t.area.clone(), t.target.clone(), t.action.clone()),
                t.clone(),
            );
        }
    }
    let normalized_changes = c
        .findings
        .iter()
        .map(|f| NormalizedChange {
            classification: f.classification.clone(),
            target_type: f.target_type.clone(),
            target_name: f.target_name.clone(),
            path: f.path.clone(),
            before: f.before.clone(),
            after: f.after.clone(),
        })
        .collect();
    AnalysisReport {
        format_version: 1,
        project: project.into(),
        from: from.into(),
        to: to.into(),
        findings: c.findings,
        normalized_changes,
        migration_hints: hints.into_values().collect(),
        suggested_runtime_tests: tests.into_values().collect(),
        summary,
        account_sizes: sizes,
        runtime_executed: false,
    }
}
