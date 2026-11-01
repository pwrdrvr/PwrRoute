//! Explanations of Finicky's host options shared by Config Files and import
//! review. These are notices only; a config never changes app preferences.

use pwrroute_engine::RulesOptions;

pub fn notices(options: &RulesOptions) -> Vec<String> {
    [
        (
            "keepRunning",
            options.keep_running,
            "Ignored: PwrRoute always stays running to route links, even with its windows closed.",
        ),
        (
            "hideIcon",
            options.hide_icon,
            "Ignored: PwrRoute keeps its tray/menu-bar icon visible and has no setting to hide it.",
        ),
        (
            "checkForUpdates",
            options.check_for_updates,
            "Ignored: PwrRoute manages its own updates. Use Settings › Updates; this option does not change update checks.",
        ),
        (
            "logRequests",
            options.log_requests,
            "Ignored: Activity records in memory. Choose disk logging in Settings › Privacy › Journal (off by default); this option does not change it.",
        ),
    ]
    .into_iter()
    .filter_map(|(name, value, explanation)| {
        value.map(|value| format!("{name}: {value} — {explanation}"))
    })
    .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn absent_options_have_no_notices() {
        assert!(notices(&RulesOptions::default()).is_empty());
    }

    #[test]
    fn both_boolean_values_explain_each_decision_and_where_to_change_it() {
        for value in [true, false] {
            let lines = notices(&RulesOptions {
                keep_running: Some(value),
                hide_icon: Some(value),
                check_for_updates: Some(value),
                log_requests: Some(value),
            });
            assert_eq!(lines.len(), 4);
            for (line, option) in
                lines
                    .iter()
                    .zip(["keepRunning", "hideIcon", "checkForUpdates", "logRequests"])
            {
                assert!(line.starts_with(&format!("{option}: {value} — Ignored:")));
            }
            assert!(lines[0].contains("always stays running"));
            assert!(lines[1].contains("no setting to hide it"));
            assert!(lines[2].contains("Settings › Updates"));
            assert!(lines[3].contains("Settings › Privacy › Journal (off by default)"));
        }
    }
}
