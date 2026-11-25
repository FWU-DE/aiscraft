//! „Entdecken“: the tabs, sort orders and filter groups of AIS.chat's overview pages
//! (`entity-overview.tsx`), applied to assistants, characters and learning scenarios together.
//!
//! Matching follows AIS.chat (`custom-chat-filter-utils.ts`): within a group any selected value
//! counts (OR), across groups each group with a selection must match (AND). Sorting follows
//! `entity-overview/utils.ts`: newest first by default, popularity ties go to the newest.

use std::cmp::Ordering;

use serde::{Deserialize, Serialize};

use crate::{PersonaFilters, PersonaInfo, PersonaKind};

/// A filter group of AIS.chat's filter panel.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FilterGroup {
    SchoolTypes,
    GradeRanges,
    Subjects,
    Categories,
    FederalStates,
    Languages,
}

/// AIS.chat's keys and German labels (`apps/chat-bot/messages/de.json`), in the order of its
/// enums (`packages/shared/src/db/schema.ts`).
const SCHOOL_TYPES: &[(&str, &str)] = &[
    ("elementary-school", "Grundschule"),
    ("special-needs-school", "Förderschule"),
    ("middle-school", "Mittel- / Hauptschule"),
    ("secondary-school", "Realschule"),
    ("grammar-school", "Gymnasium"),
    ("comprehensive-school", "Gesamtschule"),
    ("vocational-school", "Berufsschule"),
    ("technical-college", "Fachoberschule"),
    ("other", "Andere"),
];
const GRADE_RANGES: &[(&str, &str)] = &[("range-1", "1-4"), ("range-2", "5-7"), ("range-3", "8-10"), ("range-4", "11-13")];
const SUBJECTS: &[(&str, &str)] = &[
    ("german", "Deutsch"),
    ("german-as-second-language", "Deutsch als Zweitsprache"),
    ("english", "Englisch"),
    ("french", "Französisch"),
    ("greek", "Griechisch"),
    ("italian", "Italienisch"),
    ("latin", "Latein"),
    ("russian", "Russisch"),
    ("spanish", "Spanisch"),
    ("turkish", "Türkisch"),
    ("geography", "Geographie"),
    ("history", "Geschichte"),
    ("politics", "Politische Bildung"),
    ("economics", "Wirtschaftskunde"),
    ("art", "Kunst"),
    ("music", "Musik"),
    ("sports", "Sport"),
    ("business-studies", "Wirtschaft & Beruf"),
    ("health", "Gesundheit"),
    ("intercultural-education", "Interkulturelle Bildung"),
    ("media-education", "Medienpädagogik"),
    ("education", "Pädagogik"),
    ("psychology", "Psychologie"),
    ("addiction-prevention", "Sucht und Prävention"),
    ("comprehensive-subjects", "Übergreifende Themen"),
    ("traffic-education", "Verkehrserziehung"),
    ("biology", "Biologie"),
    ("chemistry", "Chemie"),
    ("informatics", "Informatik/ITB"),
    ("mathematics", "Mathematik"),
    ("physics", "Physik"),
    ("social-studies", "Sachkunde"),
    ("environmental-studies", "Umwelt"),
    ("ethics", "Ethik"),
    ("philosophy", "Philosophie"),
    ("religion", "Religion"),
];
const CATEGORIES: &[(&str, &str)] = &[
    ("artificial-intelligence", "KI & Prompting"),
    ("writing", "Schreiben & Texte"),
    ("projects", "Projekte"),
    ("coaching", "Lernbegleitung & Coaching"),
    ("organisation", "Organisation"),
    ("feedback", "Feedback"),
    ("conversation", "Sprachpraxis & Konversationen"),
    ("historical-figures", "Historische Persönlichkeiten & Zeitzeugen"),
    ("experts", "Fachliche Experten"),
    ("lesson-planning", "Unterrichtsplanung"),
    ("school-development", "Schulentwicklung"),
    ("teaching-material", "Materialerstellung"),
];
const FEDERAL_STATES: &[(&str, &str)] = &[
    ("baden-wuerttemberg", "Baden-Württemberg"),
    ("bavaria", "Bayern"),
    ("berlin", "Berlin"),
    ("brandenburg", "Brandenburg"),
    ("bremen", "Bremen"),
    ("hamburg", "Hamburg"),
    ("hesse", "Hessen"),
    ("mecklenburg-western-pomerania", "Mecklenburg-Vorpommern"),
    ("lower-saxony", "Niedersachsen"),
    ("northrhine-westphalia", "Nordrhein-Westfalen"),
    ("rhineland-palatinate", "Rheinland-Pfalz"),
    ("saarland", "Saarland"),
    ("saxony", "Sachsen"),
    ("saxony-anhalt", "Sachsen-Anhalt"),
    ("schleswig-holstein", "Schleswig-Holstein"),
    ("thuringia", "Thüringen"),
];
const LANGUAGES: &[(&str, &str)] = &[
    ("german", "Deutsch"),
    ("english", "Englisch"),
    ("arabic", "Arabisch"),
    ("turkish", "Türkisch"),
    ("french", "Französisch"),
    ("italian", "Italienisch"),
    ("spanish", "Spanisch"),
    ("greek", "Griechisch"),
    ("latin", "Latein"),
    ("russian", "Russisch"),
];

/// At most this many values per group, on an entity or in a selection.
pub const MAX_FILTER_VALUES: usize = 40;

impl FilterGroup {
    pub const ALL: [FilterGroup; 6] = [
        FilterGroup::SchoolTypes,
        FilterGroup::GradeRanges,
        FilterGroup::Subjects,
        FilterGroup::Categories,
        FilterGroup::FederalStates,
        FilterGroup::Languages,
    ];

    pub fn key(self) -> &'static str {
        match self {
            FilterGroup::SchoolTypes => "school_types",
            FilterGroup::GradeRanges => "grade_ranges",
            FilterGroup::Subjects => "subjects",
            FilterGroup::Categories => "categories",
            FilterGroup::FederalStates => "federal_states",
            FilterGroup::Languages => "languages",
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|g| g.key() == s || g.key().replace('_', "-") == s)
    }

    /// The German name AIS.chat gives the group.
    pub fn label(self) -> &'static str {
        match self {
            FilterGroup::SchoolTypes => "Schulform",
            FilterGroup::GradeRanges => "Klassenstufe",
            FilterGroup::Subjects => "Fach",
            FilterGroup::Categories => "Kategorie",
            FilterGroup::FederalStates => "Bundesland",
            FilterGroup::Languages => "Sprache",
        }
    }

    /// The values AIS.chat knows, as (key, German label).
    pub fn values(self) -> &'static [(&'static str, &'static str)] {
        match self {
            FilterGroup::SchoolTypes => SCHOOL_TYPES,
            FilterGroup::GradeRanges => GRADE_RANGES,
            FilterGroup::Subjects => SUBJECTS,
            FilterGroup::Categories => CATEGORIES,
            FilterGroup::FederalStates => FEDERAL_STATES,
            FilterGroup::Languages => LANGUAGES,
        }
    }

    /// The label of a value; a key AIS.chat added after us shows as it is. Grade ranges read
    /// „Klasse 5-7“.
    pub fn value_label(self, key: &str) -> String {
        match self.values().iter().find(|(k, _)| *k == key) {
            Some((_, l)) if self == FilterGroup::GradeRanges => format!("Klasse {l}"),
            Some((_, l)) => (*l).to_string(),
            None => key.to_string(),
        }
    }

    /// Position of a value in AIS.chat's order; unknown keys come last.
    pub fn rank(self, key: &str) -> usize {
        self.values().iter().position(|(k, _)| *k == key).unwrap_or(usize::MAX)
    }
}

/// A plausible filter key: lower-case ASCII letters, digits and dashes, at most 60 characters.
pub fn filter_key_ok(key: &str) -> bool {
    !key.is_empty() && key.len() <= 60 && key.chars().all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-')
}

/// AIS.chat's overview tabs (`OverviewFilter`), in its order.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ExploreSource {
    Mine,
    Official,
    School,
    Community,
    #[default]
    All,
    Favorites,
}

impl ExploreSource {
    pub const ALL: [ExploreSource; 6] =
        [ExploreSource::Mine, ExploreSource::Official, ExploreSource::School, ExploreSource::Community, ExploreSource::All, ExploreSource::Favorites];

    pub fn key(self) -> &'static str {
        match self {
            ExploreSource::Mine => "mine",
            ExploreSource::Official => "official",
            ExploreSource::School => "school",
            ExploreSource::Community => "community",
            ExploreSource::All => "all",
            ExploreSource::Favorites => "favorites",
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|x| x.key() == s)
    }

    pub fn label(self) -> &'static str {
        match self {
            ExploreSource::Mine => "Meine",
            ExploreSource::Official => "Offiziell",
            ExploreSource::School => "Schulintern",
            ExploreSource::Community => "Community",
            ExploreSource::All => "Alle",
            ExploreSource::Favorites => "Favoriten",
        }
    }

    pub fn hint(self) -> &'static str {
        match self {
            ExploreSource::Mine => "Die du selbst angelegt hast",
            ExploreSource::Official => "Vom Land bereitgestellt",
            ExploreSource::School => "An deiner Schule geteilt",
            ExploreSource::Community => "Von Lehrkräften für alle geteilt",
            ExploreSource::All => "Alles, was du sehen darfst",
            ExploreSource::Favorites => "Deine Favoriten (der Stern auf einer Karte)",
        }
    }

    pub fn matches(self, p: &PersonaInfo) -> bool {
        match self {
            ExploreSource::Mine => p.own,
            ExploreSource::Official => p.sharing.official,
            ExploreSource::School => p.sharing.school,
            ExploreSource::Community => p.sharing.community,
            ExploreSource::All => true,
            ExploreSource::Favorites => p.favorite,
        }
    }
}

/// AIS.chat's sort orders (`VALID_SORT_OPTIONS`).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum ExploreSort {
    #[default]
    #[serde(rename = "date-desc")]
    DateDesc,
    #[serde(rename = "date-asc")]
    DateAsc,
    #[serde(rename = "name-asc")]
    NameAsc,
    #[serde(rename = "name-desc")]
    NameDesc,
    #[serde(rename = "popularity")]
    Popularity,
}

impl ExploreSort {
    pub const ALL: [ExploreSort; 5] = [ExploreSort::DateDesc, ExploreSort::DateAsc, ExploreSort::NameAsc, ExploreSort::NameDesc, ExploreSort::Popularity];

    pub fn key(self) -> &'static str {
        match self {
            ExploreSort::DateDesc => "date-desc",
            ExploreSort::DateAsc => "date-asc",
            ExploreSort::NameAsc => "name-asc",
            ExploreSort::NameDesc => "name-desc",
            ExploreSort::Popularity => "popularity",
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|x| x.key() == s || x.key().replace('-', "_") == s)
    }

    pub fn label(self) -> &'static str {
        match self {
            ExploreSort::DateDesc => "Neueste zuerst",
            ExploreSort::DateAsc => "Älteste zuerst",
            ExploreSort::NameAsc => "Alphabetisch A-Z",
            ExploreSort::NameDesc => "Alphabetisch Z-A",
            ExploreSort::Popularity => "Beliebteste zuerst",
        }
    }

    /// The order of two entries. Without a date an entry goes after those with one; equal ones
    /// go by name, so the order never depends on the server's.
    pub fn compare(self, a: &PersonaInfo, b: &PersonaInfo) -> Ordering {
        let by_name = || name_key(&a.name).cmp(&name_key(&b.name)).then_with(|| a.name.cmp(&b.name)).then_with(|| a.id.cmp(&b.id));
        let newest = || match (&a.updated_at, &b.updated_at) {
            (Some(x), Some(y)) => y.cmp(x),
            (Some(_), None) => Ordering::Less,
            (None, Some(_)) => Ordering::Greater,
            (None, None) => Ordering::Equal,
        };
        match self {
            ExploreSort::DateDesc => newest().then_with(by_name),
            ExploreSort::DateAsc => match (&a.updated_at, &b.updated_at) {
                (Some(x), Some(y)) => x.cmp(y),
                (Some(_), None) => Ordering::Less,
                (None, Some(_)) => Ordering::Greater,
                (None, None) => Ordering::Equal,
            }
            .then_with(by_name),
            ExploreSort::NameAsc => by_name(),
            ExploreSort::NameDesc => by_name().reverse(),
            ExploreSort::Popularity => b.popularity.cmp(&a.popularity).then_with(newest).then_with(by_name),
        }
    }
}

/// What „Entdecken“ shows: tab, kind, search, sort order and the selected filter values.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct ExploreFilter {
    /// The tab (AIS.chat's `OverviewFilter`); „Alle“ by default.
    pub source: ExploreSource,
    /// Only one kind; `None` = assistants, characters and learning scenarios together.
    pub kind: Option<PersonaKind>,
    /// Words that must all occur in name or description.
    pub query: String,
    pub sort: ExploreSort,
    /// The selected values per group.
    pub selected: PersonaFilters,
}

impl ExploreFilter {
    /// Tab, filter groups and search, not the kind (the kind chips count with this).
    pub fn matches(&self, p: &PersonaInfo) -> bool {
        self.matcher()(p)
    }

    /// [`Self::matches`] for many entries: the search words are folded once.
    pub fn matcher(&self) -> impl Fn(&PersonaInfo) -> bool + '_ {
        let terms = query_terms(&self.query);
        move |p| self.source.matches(p) && matches_groups(&p.filters, &self.selected) && matches_query(p, &terms)
    }

    pub fn matches_kind(&self, p: &PersonaInfo) -> bool {
        self.kind.is_none_or(|k| p.kind == k)
    }

    /// How many filter values are selected (the badge on „Filter“).
    pub fn active_count(&self) -> usize {
        FilterGroup::ALL.iter().map(|g| self.selected.get(*g).len()).sum()
    }

    /// The entries to show, in order: `items` filtered by everything and sorted.
    pub fn apply<'a, T>(&self, items: &'a [T], info: impl Fn(&T) -> &PersonaInfo) -> Vec<&'a T> {
        let matches = self.matcher();
        let mut out: Vec<&T> = items.iter().filter(|x| matches(info(x)) && self.matches_kind(info(x))).collect();
        out.sort_by(|a, b| self.sort.compare(info(a), info(b)));
        out
    }

    /// Select or deselect one value; unknown-looking keys are refused, the count is capped.
    pub fn set_value(&mut self, group: FilterGroup, key: &str, on: bool) -> bool {
        if !filter_key_ok(key) {
            return false;
        }
        let list = self.selected.get_mut(group);
        let present = list.iter().any(|v| v == key);
        if on && !present && list.len() < MAX_FILTER_VALUES {
            list.push(key.to_string());
            list.sort_by_key(|v| (group.rank(v), v.clone()));
        } else if !on && present {
            list.retain(|v| v != key);
        }
        true
    }
}

/// AIS.chat's `matchesFilterValues`: OR within a group, AND across groups; a group without a
/// selection matches everything.
pub fn matches_groups(values: &PersonaFilters, selected: &PersonaFilters) -> bool {
    FilterGroup::ALL.iter().all(|g| {
        let want = selected.get(*g);
        want.is_empty() || values.get(*g).iter().any(|v| want.contains(v))
    })
}

/// Lower case without umlauts and accents, so "ubung", "uebung" and "Übung" find each other.
pub fn fold(s: &str) -> String {
    fold_chars(s).replace("ae", "a").replace("oe", "o").replace("ue", "u")
}

fn fold_chars(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.chars().flat_map(char::to_lowercase) {
        match c {
            'ä' | 'à' | 'á' | 'â' | 'ã' | 'å' => out.push('a'),
            'ö' | 'ò' | 'ó' | 'ô' | 'õ' | 'ø' => out.push('o'),
            'ü' | 'ù' | 'ú' | 'û' => out.push('u'),
            'é' | 'è' | 'ê' | 'ë' => out.push('e'),
            'í' | 'ì' | 'î' | 'ï' => out.push('i'),
            'ç' => out.push('c'),
            'ñ' => out.push('n'),
            'ß' => out.push_str("ss"),
            _ => out.push(c),
        }
    }
    out
}

/// The key names sort by: case and accents do not count ("Äpfel" next to "Apfel").
pub fn name_key(name: &str) -> String {
    fold_chars(name.trim())
}

/// Does every word of the query occur in name or description?
pub fn matches_query(p: &PersonaInfo, terms: &[String]) -> bool {
    if terms.is_empty() {
        return true;
    }
    let hay = fold(&format!("{} {}", p.name, p.description.as_deref().unwrap_or("")));
    terms.iter().all(|t| hay.contains(t.as_str()))
}

pub fn query_terms(q: &str) -> Vec<String> {
    fold(q).split_whitespace().map(str::to_string).collect()
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::indexing_slicing)]
    use super::*;

    fn p(id: &str, name: &str) -> PersonaInfo {
        PersonaInfo {
            kind: PersonaKind::Assistant,
            id: id.into(),
            name: name.into(),
            description: None,
            own: false,
            initial_message: None,
            exercise: None,
            prompt_suggestions: Vec::new(),
            model_id: None,
            picture_url: None,
            favorite: false,
            web_search: None,
            updated_at: None,
            popularity: 0,
            sharing: Default::default(),
            filters: Default::default(),
        }
    }

    #[test]
    fn groups_or_within_and_across() {
        let mut a = p("a", "A");
        a.filters.school_types = vec!["grammar-school".into()];
        a.filters.subjects = vec!["mathematics".into(), "physics".into()];
        let mut sel = PersonaFilters::default();
        assert!(matches_groups(&a.filters, &sel));
        sel.school_types = vec!["elementary-school".into(), "grammar-school".into()];
        assert!(matches_groups(&a.filters, &sel), "one of the school types is enough");
        sel.subjects = vec!["history".into()];
        assert!(!matches_groups(&a.filters, &sel), "every group with a selection must match");
        sel.subjects.push("physics".into());
        assert!(matches_groups(&a.filters, &sel));
    }

    #[test]
    fn sort_orders_and_ties() {
        let mut x = p("x", "Zebra");
        x.updated_at = Some("2026-01-01T00:00:00.000Z".into());
        x.popularity = 5;
        let mut y = p("y", "ähre");
        y.updated_at = Some("2026-06-01T00:00:00.000Z".into());
        y.popularity = 5;
        let mut z = p("z", "Birne");
        z.popularity = 9;
        let names = |s: ExploreSort| {
            let mut v = [x.clone(), y.clone(), z.clone()];
            v.sort_by(|a, b| s.compare(a, b));
            v.iter().map(|p| p.id.clone()).collect::<Vec<_>>().join("")
        };
        assert_eq!(names(ExploreSort::DateDesc), "yxz", "without a date last");
        assert_eq!(names(ExploreSort::DateAsc), "xyz");
        assert_eq!(names(ExploreSort::NameAsc), "yzx", "umlauts sort with their letter");
        assert_eq!(names(ExploreSort::NameDesc), "xzy");
        assert_eq!(names(ExploreSort::Popularity), "zyx", "a tie goes to the newest");
        assert_eq!(ExploreSort::parse("name_asc"), Some(ExploreSort::NameAsc));
        assert_eq!(serde_json::to_value(ExploreSort::DateDesc).unwrap(), "date-desc");
    }

    #[test]
    fn values_are_checked_and_ordered() {
        let mut f = ExploreFilter::default();
        assert!(f.set_value(FilterGroup::GradeRanges, "range-3", true));
        assert!(f.set_value(FilterGroup::GradeRanges, "range-1", true));
        assert!(f.set_value(FilterGroup::GradeRanges, "range-1", true));
        assert_eq!(f.selected.grade_ranges, ["range-1", "range-3"]);
        assert!(!f.set_value(FilterGroup::Subjects, "<script>", true));
        assert_eq!(f.active_count(), 2);
        assert_eq!(FilterGroup::GradeRanges.value_label("range-2"), "Klasse 5-7");
        assert_eq!(FilterGroup::Subjects.value_label("astronomy"), "astronomy");
        assert_eq!(FilterGroup::parse("federal-states"), Some(FilterGroup::FederalStates));
    }
}
