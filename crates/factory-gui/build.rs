//! Compile the bilingual catalog into type-checked Rust format expressions.
use std::{collections::BTreeMap, env, fs, path::PathBuf};

fn main() {
    println!("cargo:rerun-if-changed=locales/messages.json");
    let messages: BTreeMap<String, [String; 2]> =
        serde_json::from_str(&fs::read_to_string("locales/messages.json").unwrap()).unwrap();
    let mut source = String::from(
        "// Generated from locales/messages.json; do not edit.\n\
         macro_rules! tr {\n",
    );
    for (key, [de, en]) in &messages {
        assert!(!de.is_empty() && !en.is_empty(), "Missing translation for {key}");
        assert_eq!(fields(de), fields(en), "Format placeholders differ for {key}");
        // Arguments are forwarded verbatim so Rust checks formatting in both languages.
        // Templates cannot accidentally be rendered as literal "{placeholder}" text.
        if !de.contains('{') {
            source.push_str(&format!(
                "({key:?}) => {{ if $crate::i18n::is_german() {{ {de:?} }} else {{ {en:?} }} }};\n"
            ));
        }
        source.push_str(&format!(
            "({key:?}, $($args:tt)*) => {{ if $crate::i18n::is_german() {{ \
             format!({de:?}, $($args)*) }} else {{ format!({en:?}, $($args)*) }} }};\n"
        ));
    }
    source.push_str("}\npub(crate) use tr;\n");
    source.push_str("pub const CATALOG: &[(&str, &str, &str)] = &[\n");
    for (key, [de, en]) in &messages {
        source.push_str(&format!("({key:?}, {de:?}, {en:?}),\n"));
    }
    source.push_str("];\n");
    fs::write(
        PathBuf::from(env::var_os("OUT_DIR").unwrap()).join("messages.rs"),
        source,
    )
    .unwrap();
}

fn fields(text: &str) -> Vec<&str> {
    let mut fields = Vec::new();
    let mut remaining = text;
    while let Some(start) = remaining.find('{') {
        remaining = &remaining[start + 1..];
        if remaining.starts_with('{') {
            remaining = &remaining[1..];
            continue;
        }
        let end = remaining.find('}').expect("Unclosed format placeholder");
        fields.push(&remaining[..end]);
        remaining = &remaining[end + 1..];
    }
    fields.sort_unstable();
    fields
}
