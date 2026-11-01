//! The `/rewind` query: words that sessions and days must match.
//!
//! Every word must match (AND), all within the same session. A plain word
//! matches a session's activity, project or task (fuzzy, like the start
//! form's suggestions) or its notes (as a substring). A prefix limits a
//! word to one field: `a:` activity, `p:` project, `t:` task (fuzzy), `n:`
//! notes (substring). Quotes make a phrase (`n:"burst budget"`), matched
//! as a substring in any field. Dates limit the days instead:
//! `2026-10-23`, `2026-10` and `2026` on their own or after `d:`, and
//! month names (`d:oct`, any year) only after `d:`, since "may" is also a
//! word. Case never matters. Pure: no db, no UI.

use std::sync::LazyLock;

use chrono::{Datelike, NaiveDate};
use fuzzy_matcher::FuzzyMatcher;
use fuzzy_matcher::skim::SkimMatcherV2;

use crate::session::Session;

/// The fuzzy matcher, built once: the pickers, the palette and searches
/// match on every keystroke (or frame).
pub fn matcher() -> &'static SkimMatcherV2 {
    static MATCHER: LazyLock<SkimMatcherV2> = LazyLock::new(SkimMatcherV2::default);
    &MATCHER
}

/// Where a word is looked for.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Field {
    Any,
    Activity,
    Project,
    Task,
    Notes,
}

/// The days a date word allows.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Dates {
    /// From the first to the last day, both included.
    Between(NaiveDate, NaiveDate),
    /// A month of any year (1–12).
    Month(u32),
}

#[derive(Clone, Debug, PartialEq, Eq)]
enum Term {
    Text {
        field: Field,
        /// Lowercased.
        text: String,
        phrase: bool,
    },
    Date(Dates),
}

/// A parsed query; empty matches everything.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Query {
    terms: Vec<Term>,
}

/// How a session matched: the first note line a word was found in, to
/// show under it.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct SessionMatch {
    pub note: Option<String>,
}

impl Query {
    pub fn parse(input: &str) -> Self {
        Self {
            terms: words(input).into_iter().filter_map(term).collect(),
        }
    }

    pub fn is_empty(&self) -> bool {
        self.terms.is_empty()
    }

    /// Whether the query has words for sessions (not only dates).
    pub fn has_text(&self) -> bool {
        self.terms.iter().any(|t| matches!(t, Term::Text { .. }))
    }

    /// Whether `date` is within every date word.
    pub fn matches_date(&self, date: NaiveDate) -> bool {
        self.terms.iter().all(|term| match term {
            Term::Date(Dates::Between(first, last)) => (*first..=*last).contains(&date),
            Term::Date(Dates::Month(month)) => date.month() == *month,
            Term::Text { .. } => true,
        })
    }

    /// How `session` matches every word that isn't a date, if it does.
    pub fn match_session(&self, session: &Session) -> Option<SessionMatch> {
        let mut found = SessionMatch::default();
        for term in &self.terms {
            let Term::Text {
                field,
                text,
                phrase,
            } = term
            else {
                continue;
            };
            let label = |value: Option<&str>| {
                value.is_some_and(|value| match phrase {
                    true => value.to_lowercase().contains(text.as_str()),
                    false => matcher().fuzzy_match(value, text).is_some(),
                })
            };
            let labels = match field {
                Field::Any => {
                    label(Some(&session.activity))
                        || label(session.project.as_deref())
                        || label(session.task.as_deref())
                }
                Field::Activity => label(Some(&session.activity)),
                Field::Project => label(session.project.as_deref()),
                Field::Task => label(session.task.as_deref()),
                Field::Notes => false,
            };
            if labels {
                continue;
            }
            if !matches!(field, Field::Any | Field::Notes) {
                return None;
            }
            let note = session
                .notes
                .as_deref()
                .unwrap_or_default()
                .lines()
                .find(|line| line.to_lowercase().contains(text.as_str()))?;
            found.note.get_or_insert_with(|| note.to_string());
        }
        Some(found)
    }
}

/// The query's words: split on spaces, except inside quotes, which stay
/// on the word (`n:"a b"`).
fn words(input: &str) -> Vec<String> {
    let mut words = Vec::new();
    let mut word = String::new();
    let mut quoted = false;
    for c in input.chars() {
        match c {
            '"' => {
                quoted = !quoted;
                word.push(c);
            }
            c if c.is_whitespace() && !quoted => {
                if !word.is_empty() {
                    words.push(std::mem::take(&mut word));
                }
            }
            c => word.push(c),
        }
    }
    if !word.is_empty() {
        words.push(word);
    }
    words
}

/// A word as a term; `None` for one with nothing left to match (`p:`, a
/// bad date after `d:`).
fn term(word: String) -> Option<Term> {
    let word = word.to_lowercase();
    let (prefix, rest) = match word.split_once(':') {
        Some((prefix @ ("a" | "p" | "t" | "n" | "d"), rest)) => (Some(prefix), rest),
        _ => (None, word.as_str()),
    };
    let field = match prefix {
        Some("d") => return dates(rest, true).map(Term::Date),
        Some("a") => Field::Activity,
        Some("p") => Field::Project,
        Some("t") => Field::Task,
        Some("n") => Field::Notes,
        _ => match dates(rest, false) {
            Some(dates) => return Some(Term::Date(dates)),
            None => Field::Any,
        },
    };
    let phrase = rest.len() >= 2 && rest.starts_with('"') && rest.ends_with('"');
    let text = rest.trim_matches('"').to_string();
    (!text.is_empty()).then_some(Term::Text {
        field,
        text,
        phrase,
    })
}

/// A date word: `YYYY-MM-DD`, `YYYY-MM` or `YYYY`; with `names`, also a
/// month name (`oct`, `october`).
fn dates(word: &str, names: bool) -> Option<Dates> {
    if let Ok(day) = NaiveDate::parse_from_str(word, "%Y-%m-%d") {
        return Some(Dates::Between(day, day));
    }
    let number = |s: &str| s.parse::<i32>().ok();
    let year = |s: &str| number(s).filter(|_| s.len() == 4);
    match word.split('-').collect::<Vec<_>>()[..] {
        [y] if let Some(y) = year(y) => Some(Dates::Between(
            NaiveDate::from_ymd_opt(y, 1, 1)?,
            NaiveDate::from_ymd_opt(y, 12, 31)?,
        )),
        [y, m] if let (Some(y), Some(m)) = (year(y), number(m)) => {
            let first = NaiveDate::from_ymd_opt(y, m as u32, 1)?;
            let next = first.checked_add_months(chrono::Months::new(1))?;
            Some(Dates::Between(first, next.pred_opt()?))
        }
        _ if names => {
            const MONTHS: [&str; 12] = [
                "january",
                "february",
                "march",
                "april",
                "may",
                "june",
                "july",
                "august",
                "september",
                "october",
                "november",
                "december",
            ];
            MONTHS
                .iter()
                .position(|name| word.len() >= 3 && name.starts_with(word))
                .map(|i| Dates::Month(i as u32 + 1))
        }
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Utc;

    fn session(activity: &str, project: Option<&str>, task: Option<&str>, notes: &str) -> Session {
        Session {
            id: 1,
            started_at: Utc::now(),
            ended_at: None,
            project: project.map(String::from),
            task: task.map(String::from),
            activity: activity.to_string(),
            notes: (!notes.is_empty()).then(|| notes.to_string()),
            focus: None,
            interruptions: None,
        }
    }

    fn day(y: i32, m: u32, d: u32) -> NaiveDate {
        NaiveDate::from_ymd_opt(y, m, d).unwrap()
    }

    #[test]
    fn plain_words_match_labels_fuzzily_and_notes_as_text() {
        let s = session(
            "Programming",
            Some("api"),
            Some("rate limits"),
            "first note\nthe burst budget is too small\n",
        );
        let matches = |q: &str| Query::parse(q).match_session(&s);
        assert!(matches("prg").is_some(), "fuzzy on the activity");
        assert!(matches("API").is_some(), "case doesn't matter");
        assert_eq!(
            matches("budget").unwrap().note.as_deref(),
            Some("the burst budget is too small")
        );
        assert!(matches("zebra").is_none());
        // Every word, in the same session.
        assert!(matches("api budget").is_some());
        assert!(matches("api zebra").is_none());
    }

    #[test]
    fn prefixes_keep_a_word_to_one_field() {
        let s = session(
            "Writing",
            Some("blog"),
            Some("sqlite tips"),
            "about the api\n",
        );
        let matches = |q: &str| Query::parse(q).match_session(&s).is_some();
        assert!(matches("p:blog"));
        assert!(!matches("p:api"), "only in the notes");
        assert!(matches("n:api"));
        assert!(!matches("n:blog"), "only the project");
        assert!(matches("a:wri t:sqlite"));
        assert!(!matches("a:blog"));
        // A prefix with nothing after it is dropped.
        assert!(Query::parse("p:").is_empty());
    }

    #[test]
    fn quotes_make_a_phrase() {
        let s = session("Programming", Some("api"), None, "the burst budget\n");
        let matches = |q: &str| Query::parse(q).match_session(&s).is_some();
        assert!(matches("n:\"burst budget\""));
        assert!(matches("\"burst budget\""));
        assert!(!matches("n:\"budget burst\""));
        // A phrase is exact on labels too, not fuzzy.
        assert!(!matches("\"prg\""));
        assert!(matches("\"program\""));
    }

    #[test]
    fn dates_limit_the_days() {
        let q = |s: &str| Query::parse(s);
        assert!(q("2026-10-23").matches_date(day(2026, 10, 23)));
        assert!(!q("2026-10-23").matches_date(day(2026, 10, 24)));
        assert!(q("2026-10").matches_date(day(2026, 10, 31)));
        assert!(!q("2026-10").matches_date(day(2026, 11, 1)));
        assert!(q("2025").matches_date(day(2025, 12, 31)));
        assert!(q("d:oct").matches_date(day(2024, 10, 2)));
        assert!(q("d:october").matches_date(day(2026, 10, 2)));
        assert!(!q("d:oct").matches_date(day(2026, 11, 2)));
        // A month name without d: is a word, and a date word isn't text.
        assert!(q("may").has_text());
        assert!(!q("2026-10 d:may").has_text());
        // A date word leaves sessions alone.
        let s = session("Reading", None, None, "");
        assert!(q("2026-10").match_session(&s).is_some());
        // Nonsense after d: is dropped.
        assert!(q("d:someday").is_empty());
    }

    #[test]
    fn words_split_on_spaces_outside_quotes() {
        assert_eq!(words("  a  n:\"b c\" d "), ["a", "n:\"b c\"", "d"]);
    }
}
