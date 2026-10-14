use crate::{html_escape, project};
use dg_analysis::{analyze, normalize, AnalysisReport};
use std::{
    error::Error,
    fs,
    path::{Path, PathBuf},
};

fn idl_path(path: &Path) -> Result<PathBuf, Box<dyn Error>> {
    if path.is_file() {
        return Ok(path.canonicalize()?);
    }
    if !path.is_dir() {
        return Err(format!("IDL or program directory not found: {}", path.display()).into());
    }
    let direct = path.join("idl.json");
    if direct.is_file() {
        return Ok(direct.canonicalize()?);
    }
    let idl_dir = path.join("target/idl");
    let mut files: Vec<_> = fs::read_dir(&idl_dir)?
        .filter_map(|entry| entry.ok().map(|e| e.path()))
        .filter(|p| p.extension().is_some_and(|e| e == "json"))
        .collect();
    files.sort();
    if files.len() != 1 {
        return Err(format!(
            "expected one IDL JSON in {}; found {}",
            idl_dir.display(),
            files.len()
        )
        .into());
    }
    Ok(files.remove(0).canonicalize()?)
}

fn stable_run_id(project: &str, a: &[u8], b: &[u8], mappings: &[(String, String)]) -> String {
    let mut hash = 0xcbf29ce484222325u64;
    for byte in project
        .bytes()
        .chain([0])
        .chain(a.iter().copied())
        .chain([0])
        .chain(b.iter().copied())
        .chain([0])
        .chain(
            mappings
                .iter()
                .flat_map(|(x, y)| x.bytes().chain([0]).chain(y.bytes()).chain([0])),
        )
    {
        hash ^= u64::from(byte);
        hash = hash.wrapping_mul(0x100000001b3);
    }
    format!("{hash:016x}")
}

fn write_html(path: &Path, report: &AnalysisReport) -> Result<(), Box<dyn Error>> {
    let mut rows = String::new();
    for f in &report.findings {
        rows.push_str(&format!("<tr><td>{}</td><td>{}</td><th scope=\"row\">{}</th><td>{}</td><td>{}</td><td><code>{}</code> → <code>{}</code></td></tr>",
            html_escape(&f.id),html_escape(&f.classification),html_escape(&f.target_name),
            html_escape(&f.path),html_escape(&f.explanation),html_escape(&f.before.to_string()),html_escape(&f.after.to_string())));
    }
    let html = format!("<!doctype html><html lang=\"en\"><head><meta charset=\"utf-8\"><meta name=\"viewport\" content=\"width=device-width,initial-scale=1\"><title>Doppelganger static analysis</title><style>body{{font:14px system-ui,sans-serif;color:#172329;margin:0}}header{{background:#172329;color:#fff;padding:24px max(24px,calc((100vw - 1100px)/2))}}main{{max-width:1100px;margin:auto;padding:24px}}h1{{font-size:24px;margin:0}}table{{border-collapse:collapse;width:100%;text-align:left}}td,th{{padding:9px;border-bottom:1px solid #d9e0e3;vertical-align:top;overflow-wrap:anywhere}}code{{font-size:12px;white-space:pre-wrap}}@media(max-width:700px){{table{{display:block;overflow-x:auto}}}}</style></head><body><header><h1>Doppelganger static preflight</h1><p>{} · {} → {}</p></header><main><p>{} findings · {} migration hints · {} suggested runtime tests</p><p>Static analysis only. No migration transaction was executed.</p><table><thead><tr><th>ID</th><th>Class</th><th>Target</th><th>Path</th><th>Reason</th><th>Before → After</th></tr></thead><tbody>{rows}</tbody></table></main></body></html>",
        html_escape(&report.project),html_escape(&report.from.version),html_escape(&report.to.version),
        report.findings.len(),report.migration_hints.len(),report.suggested_runtime_tests.len());
    fs::write(path, html)?;
    Ok(())
}

fn short_value(value: &serde_json::Value) -> String {
    let rendered = value.to_string();
    if rendered.chars().count() > 160 {
        format!("{}...", rendered.chars().take(160).collect::<String>())
    } else {
        rendered
    }
}

fn execute(
    project: &str,
    from: &Path,
    to: &Path,
    mappings: &[(String, String)],
    expected_program_id: Option<&str>,
    format: &str,
) -> Result<(), Box<dyn Error>> {
    if !["text", "json", "html"].contains(&format) {
        return Err("format must be text, json, or html".into());
    }
    let from_path = idl_path(from)?;
    let to_path = idl_path(to)?;
    let old_bytes = dg_analysis::read_bounded(&from_path, 8 * 1024 * 1024)?;
    let new_bytes = dg_analysis::read_bounded(&to_path, 8 * 1024 * 1024)?;
    let old = normalize(&old_bytes)?;
    let new = normalize(&new_bytes)?;
    if mappings
        .iter()
        .map(|(a, _)| a)
        .collect::<std::collections::BTreeSet<_>>()
        .len()
        != mappings.len()
        || mappings
            .iter()
            .map(|(_, b)| b)
            .collect::<std::collections::BTreeSet<_>>()
            .len()
            != mappings.len()
    {
        return Err("duplicate account mapping".into());
    }
    if let Some(expected) = expected_program_id {
        if old.address.as_deref() != Some(expected) || new.address.as_deref() != Some(expected) {
            return Err("registered project IDL address does not match manifest program_id".into());
        }
    }
    for (a, b) in mappings {
        if !old.accounts.contains_key(a) || !new.accounts.contains_key(b) {
            return Err(format!("account mapping {a} -> {b} does not exist in both IDLs").into());
        }
    }
    let report = analyze(project, &old, &new, mappings);
    let output = PathBuf::from(".doppelganger/analysis")
        .join(stable_run_id(project, &old_bytes, &new_bytes, mappings));
    fs::create_dir_all(&output)?;
    let json_path = output.join("analysis.json");
    let data = serde_json::to_vec_pretty(&report)?;
    fs::write(&json_path, &data)?;
    if format == "html" {
        write_html(&output.join("analysis.html"), &report)?;
    }
    if format == "json" {
        println!("{}", String::from_utf8(data)?);
    } else {
        println!("DOPPELGANGER STATIC PREFLIGHT ANALYSIS\nProject: {project}\nUpgrade: {} -> {}\nFindings: {}\nMigration hints: {}\nSuggested runtime tests: {}",
            old.version,new.version,report.findings.len(),report.migration_hints.len(),report.suggested_runtime_tests.len());
        for f in &report.findings {
            println!(
                "\n{} {} {}\n  {}\n  {} -> {}\n  {}",
                f.id,
                f.classification,
                f.target_name,
                f.path,
                short_value(&f.before),
                short_value(&f.after),
                f.explanation
            );
            for hint in &f.migration_hints {
                println!("  Hint: {}", hint.suggested_test);
            }
        }
        println!("\nJSON: {}", json_path.display());
        if format == "html" {
            println!("HTML: {}", output.join("analysis.html").display());
        }
    }
    Ok(())
}

pub fn direct(args: impl Iterator<Item = String>) -> Result<(), Box<dyn Error>> {
    let mut from = None;
    let mut to = None;
    let mut format = "text".to_owned();
    let mut mappings = Vec::new();
    let mut args = args.peekable();
    while let Some(flag) = args.next() {
        match flag.as_str() {
            "--from" => from = Some(args.next().ok_or("--from needs a path")?),
            "--to" => to = Some(args.next().ok_or("--to needs a path")?),
            "--format" => format = args.next().ok_or("--format needs a value")?,
            "--account" => {
                let pair = args.next().ok_or("--account needs OLD:NEW")?;
                let (old, new) = pair.split_once(':').ok_or("--account needs OLD:NEW")?;
                if old.is_empty() || new.is_empty() {
                    return Err("--account needs OLD:NEW".into());
                }
                mappings.push((old.to_owned(), new.to_owned()));
            }
            _ => return Err(format!("unknown analyze option {flag}").into()),
        }
    }
    let from = from.ok_or("missing --from")?;
    let to = to.ok_or("missing --to")?;
    execute(
        "direct",
        Path::new(&from),
        Path::new(&to),
        &mappings,
        None,
        &format,
    )
}

pub fn registered(name: &str, format: &str) -> Result<(), Box<dyn Error>> {
    let (manifest, project) = project::registered(name)?;
    let from = project.artifact(&manifest, &project.v1.idl)?;
    let to = project.artifact(&manifest, &project.v2.idl)?;
    let mappings: Vec<_> = project
        .accounts
        .iter()
        .map(|a| (a.v1.clone(), a.v2.clone()))
        .collect();
    execute(
        name,
        &from,
        &to,
        &mappings,
        Some(&project.program_id),
        format,
    )
}
