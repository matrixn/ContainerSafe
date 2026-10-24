use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::OnceLock;
static ROMANIAN: AtomicBool = AtomicBool::new(false);
static CATALOG: OnceLock<std::collections::BTreeMap<String, String>> = OnceLock::new();
pub(crate) fn set_language(index: usize) {
    ROMANIAN.store(index == 1, Ordering::Relaxed);
}
fn catalog() -> &'static std::collections::BTreeMap<String, String> {
    CATALOG.get_or_init(|| {
        serde_json::from_str(include_str!("../translations/ro.json"))
            .expect("validated translation catalog")
    })
}
pub(crate) fn retranslate(value: &str) -> String {
    let source = catalog()
        .iter()
        .find(|(_, translation)| translation.as_str() == value)
        .map(|(source, _)| source.as_str())
        .unwrap_or(value);
    text(source)
}
pub(crate) fn text(value: &str) -> String {
    if !ROMANIAN.load(Ordering::Relaxed) {
        return value.to_owned();
    }
    catalog()
        .get(value)
        .cloned()
        .unwrap_or_else(|| value.to_owned())
}
pub(crate) fn paragraphs(value: &str) -> String {
    value
        .split("\n\n")
        .map(text)
        .collect::<Vec<_>>()
        .join("\n\n")
}
pub(crate) fn format(template: &str, values: &[&str]) -> String {
    // Substitute in one pass: names and paths containing braces remain untouched.
    let translated = text(template);
    let mut output = String::new();
    let mut rest = translated.as_str();
    while let Some(start) = rest.find('{') {
        output.push_str(&rest[..start]);
        rest = &rest[start..];
        if let Some(end) = rest.find('}') {
            if let Ok(index) = rest[1..end].parse::<usize>() {
                if let Some(value) = values.get(index) {
                    output.push_str(value);
                    rest = &rest[end + 1..];
                    continue;
                }
            }
        }
        output.push('{');
        rest = &rest[1..];
    }
    output.push_str(rest);
    output
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn catalog_is_valid_and_names_are_not_reformatted() {
        assert_eq!(catalog().get("Settings").unwrap(), "Setări");
        assert_eq!(
            format("{0} / {1}", &["server-{1}", "path-{0}"]),
            "server-{1} / path-{0}"
        );
        assert_eq!(text("a-server-name"), "a-server-name");
    }
}
