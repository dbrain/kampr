/// Claude's own plan dialog, shared by the hooked card and the headless ask: each answer's key,
/// its label, and the mode an approval leaves plan mode for (#635). The last answer sends the
/// plan back with words.
pub const ANSWERS: [(&str, &str, Option<&str>); 3] = [
    ("1", "Yes, auto-accept edits", Some("acceptEdits")),
    ("2", "Yes, manually approve edits", Some("default")),
    ("3", "No, tell Claude what to change", None),
];

pub fn mode(key: &str) -> Option<&'static str> {
    ANSWERS
        .iter()
        .find(|(k, ..)| *k == key)
        .and_then(|(.., mode)| *mode)
}
