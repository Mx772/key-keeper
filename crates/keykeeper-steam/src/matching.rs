//! Matching a key's game name to Steam search results.

use crate::api::AppMatch;

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum MatchOutcome {
    /// Safe to apply automatically.
    Exact(AppMatch),
    /// Let the user pick (best first, at most 5).
    Ambiguous(Vec<AppMatch>),
    None,
}

/// Lowercase, `&` → `and`, trademark signs and punctuation dropped, a leading "the" removed,
/// whitespace collapsed: "The Witcher® 3: Wild Hunt" → "witcher 3 wild hunt".
pub fn normalize_title(s: &str) -> String {
    let lowered = s.to_lowercase().replace('&', " and ");
    let cleaned: String = lowered
        .chars()
        .map(|c| if c.is_alphanumeric() { c } else { ' ' })
        .collect();
    let words: Vec<&str> = cleaned.split_whitespace().collect();
    let words = match words.split_first() {
        Some((&"the", rest)) if !rest.is_empty() => rest.to_vec(),
        _ => words,
    };
    words.join(" ")
}

pub fn best_match(name: &str, results: &[AppMatch]) -> MatchOutcome {
    let want = normalize_title(name);
    if want.is_empty() || results.is_empty() {
        return MatchOutcome::None;
    }
    let exact: Vec<&AppMatch> = results
        .iter()
        .filter(|r| normalize_title(&r.name) == want)
        .collect();
    match exact.as_slice() {
        [one] => return MatchOutcome::Exact((*one).clone()),
        [_, _, ..] => return MatchOutcome::Ambiguous(exact.into_iter().take(5).cloned().collect()),
        [] => {}
    }
    // A lone result that extends the name ("Spiritfarer" → "Spiritfarer®: Farewell Edition").
    if let [only] = results
        && normalize_title(&only.name).starts_with(&format!("{want} "))
    {
        return MatchOutcome::Exact(only.clone());
    }
    MatchOutcome::Ambiguous(results.iter().take(5).cloned().collect())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn m(id: u32, name: &str) -> AppMatch {
        AppMatch {
            app_id: id,
            name: name.into(),
        }
    }

    #[test]
    fn normalization() {
        assert_eq!(
            normalize_title("The Witcher® 3: Wild Hunt"),
            "witcher 3 wild hunt"
        );
        assert_eq!(normalize_title("Haiku, the Robot"), "haiku the robot");
        assert_eq!(
            normalize_title("Ori & the Blind Forest™"),
            "ori and the blind forest"
        );
        assert_eq!(normalize_title("The"), "the");
        assert_eq!(normalize_title("  "), "");
    }

    #[test]
    fn outcomes() {
        let r = [m(1231880, "Haiku, the Robot"), m(2, "Haiku Soundtrack")];
        assert_eq!(
            best_match("haiku the robot", &r),
            MatchOutcome::Exact(m(1231880, "Haiku, the Robot"))
        );
        assert_eq!(
            best_match(
                "Spiritfarer",
                &[m(972660, "Spiritfarer®: Farewell Edition")]
            ),
            MatchOutcome::Exact(m(972660, "Spiritfarer®: Farewell Edition"))
        );
        assert!(
            matches!(
                best_match("Spirit", &[m(972660, "Spiritfarer®: Farewell Edition")]),
                MatchOutcome::Ambiguous(_)
            ),
            "no partial-word matches"
        );
        assert!(
            matches!(best_match("Doom", &[m(1, "DOOM"), m(2, "Doom")]), MatchOutcome::Ambiguous(v) if v.len() == 2)
        );
        assert_eq!(best_match("Anything", &[]), MatchOutcome::None);
        let many: Vec<AppMatch> = (0..9).map(|i| m(i, &format!("Game {i}"))).collect();
        assert!(matches!(best_match("Gam", &many), MatchOutcome::Ambiguous(v) if v.len() == 5));
    }
}
