use factory_core::ProjectUsage;
pub fn render(report: &ProjectUsage) -> String {
    let mut text = String::from("Beobachteter Verbrauch aus archivierten OpenCode-Nachrichten\nKeine Rechnung; fehlende Angaben werden nicht als kostenlos gewertet.\nReasoning/Cache sind separate Zaehler, keine zusaetzlichen Gesamttokens.\n\n");
    if report.groups.is_empty() {
        text.push_str("Noch keine archivierten Assistant-Nachrichten.\n");
    }
    for group in &report.groups {
        let safe = |s: &str| s.chars().filter(|c| !c.is_control()).collect::<String>();
        text.push_str(&format!("{} · {}/{}\nNachrichten: {} ({} ohne Abschluss)\nInput: {} · Output: {} · Reasoning: {}\nCache gelesen: {} · geschrieben: {}\nGemeldete Kosten: {:.6} USD\nOhne Tokenangaben: {} · ohne Kostenangaben: {}\n\n",
            safe(&group.role),safe(group.provider_id.as_deref().unwrap_or("unbekannt")),safe(group.model_id.as_deref().unwrap_or("unbekannt")),
            group.assistant_messages,group.unfinished_messages,group.tokens.input,group.tokens.output,group.tokens.reasoning,
            group.tokens.cache.read,group.tokens.cache.write,group.reported_cost_usd,group.messages_missing_tokens,group.messages_missing_cost));
    }
    text
}
