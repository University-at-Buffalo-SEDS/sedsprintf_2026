// build.rs
//
// Generates C header + Python .pyi enums directly from telemetry_config.json,
// by *discovering the JSON path* from the Rust source that invokes:
//
//   define_telemetry_schema!(path = "telemetry_config.json");
//
// Also generates a C-friendly SedsResult enum from the TelemetryErrorCode enum
// found in src/lib.rs.
//
// No cbindgen is used. No intermediate temp header is written.
// Output:
//   - C-Headers/sedsprintf.h                     (injects enums into template marker)
//   - python-files/sedsprintf_rs/sedsprintf_rs.pyi (injects enums into template marker)
//
// Rebuild triggers:
//   - src/config.rs (or overridden)
//   - discovered telemetry_config.json
//   - src/lib.rs (or overridden)
//   - the header templates
//
// Optional env vars:
//   - SEDSPRINTF_RS_SKIP_ENUMGEN=1          -> skip all enum generation
//   - SEDSPRINTF_RS_CONFIG_RS=path/to.rs    -> override source file to scan (default: src/config.rs)
//   - SEDSPRINTF_RS_LIB_RS=path/to.rs       -> override lib.rs to scan (default: src/lib.rs)

use regex::Regex;
use serde::Deserialize;
use std::collections::HashSet;
use std::env;
use std::fs;
use std::path::{Path, PathBuf};

// ========================= JSON schema =========================

#[derive(Debug, Deserialize)]
struct TelemetryConfig {
    endpoints: Vec<JsonEndpoint>,
    types: Vec<JsonType>,
}

#[derive(Debug, Deserialize)]
struct JsonEndpoint {
    /// Rust enum variant, e.g. "SdCard"
    rust: String,
    /// Schema string name, e.g. "SD_CARD" (ALL CAPS)
    name: String,
    /// Optional docstring for the enum variant
    #[serde(default)]
    doc: Option<String>,
    /// Optional broadcast mode variant name, e.g. "Default"
    #[serde(default)]
    _broadcast_mode: Option<String>,
}

#[derive(Debug, Deserialize)]
struct JsonType {
    /// Rust enum variant, e.g. "TelemetryError"
    rust: String,
    /// Schema string name, e.g. "TELEMETRY_ERROR" (ALL CAPS)
    name: String,
    #[serde(default)]
    doc: Option<String>,

    element: JsonElement,
    class: String,

    /// list of DataEndpoint rust variants, e.g. ["SdCard","Radio"]
    endpoints: Vec<String>,
}

#[derive(Debug, Deserialize)]
#[serde(tag = "kind")]
enum JsonElement {
    Static { data_type: String },
    Dynamic { data_type: String },
}

// ========================= main =========================

fn main() {
    if env::var_os("SEDSPRINTF_RS_SKIP_ENUMGEN").is_some() {
        println!("cargo:warning=Skipping enum generation (SEDSPRINTF_RS_SKIP_ENUMGEN set)");
        return;
    }

    let crate_dir = PathBuf::from(env::var("CARGO_MANIFEST_DIR").expect("CARGO_MANIFEST_DIR"));

    let config_rs_rel =
        env::var("SEDSPRINTF_RS_CONFIG_RS").unwrap_or_else(|_| "src/config.rs".to_string());
    let config_rs_path = crate_dir.join(&config_rs_rel);

    let lib_rs_rel = env::var("SEDSPRINTF_RS_LIB_RS").unwrap_or_else(|_| "src/lib.rs".to_string());
    let lib_rs_path = crate_dir.join(&lib_rs_rel);

    // Rebuild triggers
    println!("cargo:rerun-if-changed={}", config_rs_path.display());
    println!("cargo:rerun-if-changed={}", lib_rs_path.display());

    // Discover schema json path by scanning config.rs for define_telemetry_schema!(path="...")
    let schema_path = find_schema_path_from_config_rs(&config_rs_path, &crate_dir);
    println!("cargo:rerun-if-changed={}", schema_path.display());
    // Rebuild if the schema json changes
    println!("cargo:rerun-if-changed={}", schema_path.display());

    // Re-run if the override env vars change (so switching paths rebuilds)
    println!("cargo:rerun-if-env-changed=SEDSPRINTF_RS_CONFIG_RS");
    println!("cargo:rerun-if-env-changed=SEDSPRINTF_RS_LIB_RS");
    println!("cargo:rerun-if-env-changed=SEDSPRINTF_RS_SKIP_ENUMGEN");

    // Make the discovered JSON path available to the crate at compile time.
    // Use in Rust as: env!("SEDSPRINTF_RS_SCHEMA_JSON")
    println!(
        "cargo:rustc-env=SEDSPRINTF_RS_SCHEMA_JSON={}",
        schema_path.display()
    );

    // Templates: rebuild if they change
    let header_tpl_path = crate_dir.join("header_templates/sedsprintf.h.txt");
    let pyi_tpl_path = crate_dir.join("header_templates/sedsprintf_rs.pyi.txt");
    println!("cargo:rerun-if-changed={}", header_tpl_path.display());
    println!("cargo:rerun-if-changed={}", pyi_tpl_path.display());

    // Load + validate JSON schema
    let cfg = load_schema_absolute(&schema_path);
    validate_schema(&cfg);

    // Parse TelemetryErrorCode from lib.rs into SedsResult members
    let seds_result = parse_seds_result_from_lib_rs(&lib_rs_path);

    // Generate C enums text
    let (c_dt, c_ep) = render_c_enums(&cfg);
    let c_sr = render_c_enum_seds_result(&seds_result);

    let c_enums_joined = format!("{c_dt}\n\n{c_ep}\n\n{c_sr}\n");

    // Generate PYI enums text (DataType, DataEndpoint, SedsResult)
    let pyi = render_pyi_enums(&cfg, &seds_result);

    // Inject into templates
    write_injected(
        &header_tpl_path,
        "/* {{AUTOGEN:ENUMS}} */",
        &crate_dir.join("C-Headers/sedsprintf.h"),
        &c_enums_joined,
    );

    write_injected(
        &pyi_tpl_path,
        "/* {{AUTOGEN:PY_ENUMS}} */",
        &crate_dir
            .join("python-files")
            .join("sedsprintf_rs")
            .join("sedsprintf_rs.pyi"),
        &pyi,
    );
}

// ========================= discovery =========================

fn find_schema_path_from_config_rs(config_rs_path: &Path, crate_dir: &Path) -> PathBuf {
    let text = fs::read_to_string(config_rs_path)
        .unwrap_or_else(|e| panic!("failed to read {}: {e}", config_rs_path.display()));

    // Matches across newlines; captures the string literal contents
    // define_telemetry_schema!(path = "telemetry_config.json");
    let re = Regex::new(r#"(?s)define_telemetry_schema!\s*\(\s*[^)]*?\bpath\s*=\s*"([^"]+)""#)
        .expect("regex compile failed");

    let mut it = re.captures_iter(&text);

    let first = it.next().unwrap_or_else(|| {
        panic!(
            "could not find define_telemetry_schema!(path = \"...\") in {}",
            config_rs_path.display()
        )
    });

    // If there are multiple, error (keeps build deterministic)
    if it.next().is_some() {
        panic!(
            "multiple define_telemetry_schema!(path = \"...\") invocations found in {} (expected exactly one)",
            config_rs_path.display()
        );
    }

    let rel = first.get(1).unwrap().as_str();
    crate_dir.join(rel)
}

// ========================= load / validate =========================

fn load_schema_absolute(path: &Path) -> TelemetryConfig {
    let bytes = fs::read(path)
        .unwrap_or_else(|e| panic!("failed to read telemetry schema {}: {e}", path.display()));

    serde_json::from_slice::<TelemetryConfig>(&bytes).unwrap_or_else(|e| {
        panic!(
            "failed to parse telemetry schema {} as JSON: {e}",
            path.display()
        )
    })
}

fn validate_schema(cfg: &TelemetryConfig) {
    if cfg.endpoints.is_empty() {
        panic!("telemetry_config.json: endpoints is empty");
    }
    if cfg.types.is_empty() {
        panic!("telemetry_config.json: types is empty");
    }

    // Ensure schema names are ALL CAPS (or underscore/digit).
    for ep in &cfg.endpoints {
        ensure_rust_ident(&ep.rust, "endpoints[].rust");
        ensure_all_caps(&ep.name, "endpoints[].name");
    }

    let endpoint_set: HashSet<&str> = cfg.endpoints.iter().map(|e| e.rust.as_str()).collect();

    for ty in &cfg.types {
        ensure_rust_ident(&ty.rust, "types[].rust");
        ensure_all_caps(&ty.name, "types[].name");

        // Ensure referenced endpoints exist
        for eprust in &ty.endpoints {
            if !endpoint_set.contains(eprust.as_str()) {
                panic!(
                    "telemetry_config.json: type {} ({}) references unknown endpoint {:?}. Valid endpoints: {:?}",
                    ty.rust,
                    ty.name,
                    eprust,
                    cfg.endpoints
                        .iter()
                        .map(|e| e.rust.as_str())
                        .collect::<Vec<_>>()
                );
            }
        }

        // Sanity check element datatype + class
        match &ty.element {
            JsonElement::Static { data_type } | JsonElement::Dynamic { data_type } => {
                if data_type.trim().is_empty() {
                    panic!(
                        "telemetry_config.json: types[{}].element.data_type is empty",
                        ty.rust
                    );
                }
            }
        }
        if ty.class.trim().is_empty() {
            panic!("telemetry_config.json: types[{}].class is empty", ty.rust);
        }
    }
}

fn ensure_rust_ident(s: &str, field: &str) {
    let re = Regex::new(r"^[A-Za-z_][A-Za-z0-9_]*$").unwrap();
    if !re.is_match(s) {
        panic!("telemetry_config.json: {field} must be a valid Rust identifier, got {s:?}");
    }
}

fn ensure_all_caps(s: &str, field: &str) {
    let ok = s
        .chars()
        .all(|c| c.is_ascii_uppercase() || c == '_' || c.is_ascii_digit());
    if !ok {
        panic!("telemetry_config.json: {field} must be ALL CAPS (A-Z, 0-9, _), got {s:?}");
    }
}

// ========================= lib.rs -> SedsResult =========================

#[derive(Debug, Clone)]
struct SedsResultEnum {
    // name -> value (i64)
    members: Vec<(String, i64)>, // (NAME, VALUE)
}

/// Parse TelemetryErrorCode from lib.rs and build a SedsResult enum mapping.
/// Rules:
/// - Includes all TelemetryErrorCode variants as SEDS_<NAME> with same numeric value.
/// - Ensures SEDS_OK = 0 exists (injected if missing).
/// - Ensures SEDS_ERR = -1 exists (injected if missing).
fn parse_seds_result_from_lib_rs(lib_rs_path: &Path) -> SedsResultEnum {
    let text = fs::read_to_string(lib_rs_path)
        .unwrap_or_else(|e| panic!("failed to read {}: {e}", lib_rs_path.display()));

    let re_enum = Regex::new(r#"(?s)\bpub\s+enum\s+TelemetryErrorCode\s*\{(.*?)}"#)
        .expect("regex compile failed");

    let caps = re_enum.captures(&text).unwrap_or_else(|| {
        panic!(
            "could not find `pub enum TelemetryErrorCode {{ ... }}` in {}",
            lib_rs_path.display()
        )
    });

    let body = caps.get(1).unwrap().as_str();

    let re_member = Regex::new(r#"(?m)^\s*([A-Za-z_][A-Za-z0-9_]*)\s*=\s*([-]?\d+)\s*,?\s*$"#)
        .expect("regex compile failed");

    let mut members: Vec<(String, i64)> = Vec::new();

    for m in re_member.captures_iter(body) {
        let rust_name = m.get(1).unwrap().as_str();
        let val: i64 = m.get(2).unwrap().as_str().parse().unwrap();
        let c_name = format!("SEDS_{}", to_screaming_snake(rust_name));
        members.push((c_name, val));
    }

    if members.is_empty() {
        panic!(
            "TelemetryErrorCode enum found, but no explicit `Variant = <int>` members were parsed in {}",
            lib_rs_path.display()
        );
    }

    // Inject SEDS_OK=0 and SEDS_ERR=-1 if missing
    let has_ok = members.iter().any(|(n, _)| n == "SEDS_OK");
    let has_err = members.iter().any(|(n, _)| n == "SEDS_ERR");
    if !has_ok {
        members.push(("SEDS_OK".to_string(), 0));
    }
    if !has_err {
        members.push(("SEDS_ERR".to_string(), -1));
    }

    // Dedup by name (keep first), then sort by value descending (0, -1, -2, ...)
    {
        use std::collections::HashSet;
        let mut seen = HashSet::new();
        members.retain(|(n, _)| seen.insert(n.clone()));
    }

    members.sort_by(|(na, va), (nb, vb)| {
        // primary: value descending
        vb.cmp(va)
            // secondary: name ascending (stable-ish)
            .then_with(|| na.cmp(nb))
    });
    SedsResultEnum { members }

}

fn to_screaming_snake(s: &str) -> String {
    // Simple PascalCase/camelCase -> SCREAMING_SNAKE.
    // Good enough for your variant names like GenericError, InvalidType, SizeMismatchError...
    let mut out = String::new();
    let mut prev_lower_or_digit = false;

    for ch in s.chars() {
        if ch.is_ascii_uppercase() {
            if prev_lower_or_digit {
                out.push('_');
            }
            out.push(ch);
            prev_lower_or_digit = false;
        } else if ch.is_ascii_lowercase() {
            out.push(ch.to_ascii_uppercase());
            prev_lower_or_digit = true;
        } else if ch.is_ascii_digit() {
            if !out.ends_with('_') && !out.is_empty() && !prev_lower_or_digit {
                // usually not needed; keep conservative
            }
            out.push(ch);
            prev_lower_or_digit = true;
        } else {
            // skip unexpected chars
        }
    }

    if out.is_empty() {
        "UNKNOWN".to_string()
    } else {
        out
    }
}

// ========================= render C enums =========================

fn render_c_enums(cfg: &TelemetryConfig) -> (String, String) {
    let dt = render_c_enum_datatype(cfg);
    let ep = render_c_enum_endpoint(cfg);
    (dt, ep)
}

fn render_c_enum_datatype(cfg: &TelemetryConfig) -> String {
    let mut lines = Vec::new();
    lines.push("typedef enum SedsDataType {".to_string());

    // Sequential discriminants from 0, in the JSON order
    for (i, ty) in cfg.types.iter().enumerate() {
        // TELEMETRY_ERROR -> SEDS_DT_TELEMETRY_ERROR
        let name = format!("SEDS_DT_{}", ty.name);
        let doc = ty.doc.as_deref().unwrap_or("").trim();

        if !doc.is_empty() {
            lines.push(format!("  /* {} */", sanitize_c_comment(doc)));
        }
        lines.push(format!("  {name} = {i},"));
    }

    lines.push("} SedsDataType;".to_string());
    lines.join("\n")
}

fn render_c_enum_endpoint(cfg: &TelemetryConfig) -> String {
    let mut lines = Vec::new();
    lines.push("typedef enum SedsDataEndpoint {".to_string());

    for (i, ep) in cfg.endpoints.iter().enumerate() {
        // SD_CARD -> SEDS_EP_SD_CARD
        let name = format!("SEDS_EP_{}", ep.name);
        let doc = ep.doc.as_deref().unwrap_or("").trim();

        if !doc.is_empty() {
            lines.push(format!("  /* {} */", sanitize_c_comment(doc)));
        }
        lines.push(format!("  {name} = {i},"));
    }

    lines.push("} SedsDataEndpoint;".to_string());
    lines.join("\n")
}

fn render_c_enum_seds_result(sr: &SedsResultEnum) -> String {
    let mut lines = Vec::new();
    lines.push("typedef enum SedsResult {".to_string());

    for (name, val) in &sr.members {
        lines.push(format!("  {name} = {val},"));
    }

    lines.push("} SedsResult;".to_string());
    lines.join("\n")
}

fn sanitize_c_comment(s: &str) -> String {
    s.replace("*/", "* /")
}

// ========================= render PYI enums =========================

fn render_pyi_enums(cfg: &TelemetryConfig, sr: &SedsResultEnum) -> String {
    // This assumes your template already imports IntEnum, etc.
    let dt = render_python_intenum(
        "DataType",
        "Wire-level type tags (generated from telemetry_config.json).",
        cfg.types
            .iter()
            .enumerate()
            .map(|(i, t)| (t.name.as_str(), i as i64, t.doc.as_deref().unwrap_or("")))
            .collect::<Vec<_>>(),
    );

    let ep = render_python_intenum(
        "DataEndpoint",
        "Routing endpoints for packets (generated from telemetry_config.json).",
        cfg.endpoints
            .iter()
            .enumerate()
            .map(|(i, e)| (e.name.as_str(), i as i64, e.doc.as_deref().unwrap_or("")))
            .collect::<Vec<_>>(),
    );

    let sr_members = sr
        .members
        .iter()
        .map(|(name, val)| (name.as_str(), *val, ""))
        .collect::<Vec<_>>();

    let sr_txt = render_python_intenum(
        "SedsResult",
        "Result/error codes (generated from TelemetryErrorCode).",
        sr_members,
    );

    format!("{dt}\n\n{ep}\n\n{sr_txt}\n")
}

fn render_python_intenum(name: &str, doc: &str, members: Vec<(&str, i64, &str)>) -> String {
    let mut lines = Vec::new();
    lines.push(format!("class {name}(IntEnum):"));
    lines.push(format!("    \"\"\"{doc}\"\"\""));

    for (ident, value, member_doc) in members {
        if member_doc.trim().is_empty() {
            lines.push(format!("    {ident}: int = {value}"));
        } else {
            lines.push(format!(
                "    {ident}: int = {value}  #: {}",
                member_doc.trim()
            ));
        }
    }

    lines.join("\n")
}

// ========================= template injection =========================

fn write_injected(template_path: &Path, marker: &str, out_path: &Path, injected: &str) {
    let tpl = fs::read_to_string(template_path)
        .unwrap_or_else(|e| panic!("read template {} failed: {e}", template_path.display()));

    if !tpl.contains(marker) {
        panic!(
            "template {} is missing marker: {}",
            template_path.display(),
            marker
        );
    }

    let final_text = tpl.replace(marker, injected);

    if let Some(parent) = out_path.parent() {
        fs::create_dir_all(parent)
            .unwrap_or_else(|e| panic!("create dir {} failed: {e}", parent.display()));
    }

    fs::write(out_path, final_text)
        .unwrap_or_else(|e| panic!("write {} failed: {e}", out_path.display()));
}
