//! „Entdecken“: tabs, sort order, search and filters over the assistants, characters and
//! learning scenarios of AIS.chat, as AIS.chat's overview pages offer them
//! (`aiscraft_core::explore`). The choice lives in [`crate::State::explore`] for this run, like
//! AIS.chat keeps tab and sort order in the browser tab's session storage: closing and opening
//! the page keeps it, a new start begins with „Alle“, „Neueste zuerst“ and no filters, so no
//! forgotten filter hides entries after a restart.

use aiscraft_core::explore::{MAX_FILTER_VALUES, filter_key_ok, matches_groups, matches_query, query_terms};
use aiscraft_core::{BackendKind, Error, ExploreFilter, ExploreSort, ExploreSource, FilterGroup, PersonaInfo, PersonaKind, Result};
use serde_json::{Value, json};

use crate::commands::{CommandSpec, opt_str};
use crate::{Ctx, State};

/// The search keeps at most this many characters.
const MAX_QUERY: usize = 200;
/// `explore.list` returns at most this many entries.
const MAX_LIST: usize = 500;

pub(crate) static COMMANDS: &[CommandSpec] = &[
    CommandSpec {
        id: "explore.filter",
        label: "Entdecken filtern",
        group: "Entdecken",
        shortcut: None,
        params: "source?: mine|official|school|community|all|favorites (Reiter wie in AIS.chat), kind?: all|assistant|character|learning_scenario, \
                 query?: string (Name oder Beschreibung), group?: school_types|grade_ranges|subjects|categories|federal_states|languages, \
                 values?: [string] (setzt die Auswahl der Gruppe), value?: string + on?: bool (einen Wert an- oder abwählen; ohne on umschalten). \
                 Innerhalb einer Gruppe reicht ein Wert, über Gruppen hinweg müssen alle passen",
        palette: false,
        run: |c, p| Box::pin(async move { filter(&c, &p) }),
    },
    CommandSpec {
        id: "explore.sort",
        label: "Entdecken sortieren",
        group: "Entdecken",
        shortcut: None,
        params: "by: date-desc (Neueste zuerst)|date-asc (Älteste zuerst)|name-asc (Alphabetisch A-Z)|name-desc (Alphabetisch Z-A)|popularity (Beliebteste zuerst)",
        palette: false,
        run: |c, p| Box::pin(async move { sort(&c, &p) }),
    },
    CommandSpec {
        id: "explore.reset",
        label: "Filter zurücksetzen",
        group: "Entdecken",
        shortcut: None,
        params: "all?: bool (auch Reiter, Art und Suche; die Sortierung bleibt). Ohne all: nur die Filterwerte",
        palette: false,
        run: |c, p| Box::pin(async move { reset(&c, &p) }),
    },
    CommandSpec {
        id: "explore.list",
        label: "Entdecken: gefilterte Liste",
        group: "Entdecken",
        shortcut: None,
        params: "backend?: string. Die Einträge mit den gewählten Filtern in der gewählten Reihenfolge, dazu die Reiter und Filterwerte mit Anzahl",
        palette: false,
        run: |c, p| Box::pin(async move { list(&c, &p) }),
    },
];

/// The entries Entdecken lists: every AIS.chat backend's list, with the backend's id.
pub fn entries(s: &State, backend: Option<&str>) -> Vec<(String, PersonaInfo)> {
    s.backends
        .iter()
        .filter(|b| b.config.kind == BackendKind::AisChat && backend.is_none_or(|id| b.config.id == id))
        .flat_map(|b| b.personas.iter().flatten().map(|p| (b.config.id.clone(), p.clone())))
        .collect()
}

/// „Schulintern“ is offered only when something is shared within a school (AIS.chat shows the
/// tab only while the state allows it, and the backend marks nothing otherwise).
pub fn school_tab(items: &[(String, PersonaInfo)]) -> bool {
    items.iter().any(|(_, p)| p.sharing.school)
}

fn summary(s: &State) -> Value {
    let items = entries(s, None);
    json!({"filter": s.explore, "hits": s.explore.apply(&items, |x| &x.1).len(), "active": s.explore.active_count()})
}

fn filter(c: &Ctx, p: &Value) -> Result<Value> {
    let mut f = c.state().explore.clone();
    if let Some(v) = p.get("source") {
        let key = v.as_str().unwrap_or("");
        f.source = ExploreSource::parse(key).ok_or_else(|| Error::invalid("source muss mine, official, school, community, all oder favorites sein"))?;
    }
    if let Some(v) = p.get("kind") {
        let bad = || Error::invalid("kind muss all, assistant, character oder learning_scenario sein");
        f.kind = match v.as_str() {
            None if v.is_null() => None,
            Some("all") => None,
            Some(k) => Some(PersonaKind::parse(k).ok_or_else(bad)?),
            None => return Err(bad()),
        };
    }
    if let Some(v) = p.get("query") {
        let q = v.as_str().ok_or_else(|| Error::invalid("query muss Text sein"))?;
        f.query = q.chars().filter(|c| !c.is_control()).take(MAX_QUERY).collect();
    }
    let group = match opt_str(p, "group") {
        Some(g) => Some(FilterGroup::parse(&g).ok_or_else(|| Error::invalid(format!("Unbekannte Filtergruppe „{}“", g.chars().take(40).collect::<String>())))?),
        None if p.get("values").is_some() || p.get("value").is_some() => return Err(Error::invalid("group fehlt")),
        None => None,
    };
    if let Some(g) = group {
        if let Some(values) = p.get("values") {
            let list = values.as_array().ok_or_else(|| Error::invalid("values muss eine Liste sein"))?;
            if list.len() > MAX_FILTER_VALUES {
                return Err(Error::invalid(format!("Höchstens {MAX_FILTER_VALUES} Werte je Gruppe")));
            }
            let mut keys = Vec::new();
            for v in list {
                let k = v.as_str().filter(|k| filter_key_ok(k)).ok_or_else(|| Error::invalid("Ungültiger Filterwert"))?;
                keys.push(k.to_string());
            }
            f.selected.get_mut(g).clear();
            for k in keys {
                f.set_value(g, &k, true);
            }
        }
        if let Some(v) = p.get("value") {
            let k = v.as_str().filter(|k| filter_key_ok(k)).ok_or_else(|| Error::invalid("Ungültiger Filterwert"))?;
            let on = p.get("on").and_then(Value::as_bool).unwrap_or_else(|| !f.selected.get(g).iter().any(|x| x == k));
            f.set_value(g, k, on);
        }
    }
    Ok(c.update(|s| {
        s.explore = f;
        summary(s)
    }))
}

fn sort(c: &Ctx, p: &Value) -> Result<Value> {
    let by = opt_str(p, "by").unwrap_or_default();
    let sort = ExploreSort::parse(&by).ok_or_else(|| Error::invalid("by muss date-desc, date-asc, name-asc, name-desc oder popularity sein"))?;
    Ok(c.update(|s| {
        s.explore.sort = sort;
        summary(s)
    }))
}

fn reset(c: &Ctx, p: &Value) -> Result<Value> {
    let all = p.get("all").and_then(Value::as_bool).unwrap_or(false);
    Ok(c.update(|s| {
        if all {
            s.explore = ExploreFilter { sort: s.explore.sort, ..Default::default() };
        } else {
            s.explore.selected = Default::default();
        }
        summary(s)
    }))
}

fn list(c: &Ctx, p: &Value) -> Result<Value> {
    let s = c.state();
    let backend = opt_str(p, "backend");
    if let Some(id) = &backend
        && s.backend(id).is_none()
    {
        return Err(Error::not_found(format!("Dienst „{}“ nicht gefunden", id.chars().take(60).collect::<String>())));
    }
    let items = entries(&s, backend.as_deref());
    let f = &s.explore;
    let shown = f.apply(&items, |x| &x.1);
    let school = school_tab(&items);
    let sources: Vec<Value> = source_counts(&items, f)
        .into_iter()
        .filter(|(x, _)| *x != ExploreSource::School || school)
        .map(|(x, n)| json!({"key": x.key(), "label": x.label(), "count": n, "selected": x == f.source}))
        .collect();
    let groups: Vec<Value> = FilterGroup::ALL
        .iter()
        .filter_map(|g| {
            let values = group_values(&items, f, *g);
            (!values.is_empty()).then(|| json!({"key": g.key(), "label": g.label(), "values": values}))
        })
        .collect();
    let list: Vec<Value> = shown
        .iter()
        .take(MAX_LIST)
        .map(|(b, p)| {
            let mut v = serde_json::to_value(p).unwrap_or(Value::Null);
            if let Some(o) = v.as_object_mut() {
                o.insert("backend".into(), json!(b));
            }
            v
        })
        .collect();
    Ok(json!({
        "filter": f,
        "sort_label": f.sort.label(),
        "hits": shown.len(),
        "active": f.active_count(),
        "sources": sources,
        "groups": groups,
        "items": list,
    }))
}

/// The values of one group that some entry carries (and those selected), in AIS.chat's order,
/// each with the number of entries it would show with the other filters as they are.
pub fn group_values(items: &[(String, PersonaInfo)], f: &ExploreFilter, g: FilterGroup) -> Vec<GroupValue> {
    let mut keys: Vec<String> = items.iter().flat_map(|(_, p)| p.filters.get(g).iter().cloned()).collect();
    keys.extend(f.selected.get(g).iter().cloned());
    keys.sort_by_key(|k| (g.rank(k), k.clone()));
    keys.dedup();
    // Counts as if only this value were chosen in the group, the rest of the filter as it is;
    // the search folds each entry once, not once per value.
    let mut without = f.selected.clone();
    without.get_mut(g).clear();
    let terms = query_terms(&f.query);
    let base: Vec<&PersonaInfo> = items
        .iter()
        .map(|(_, p)| p)
        .filter(|p| f.source.matches(p) && f.matches_kind(p) && matches_groups(&p.filters, &without) && matches_query(p, &terms))
        .collect();
    keys.into_iter()
        .map(|k| {
            let count = base.iter().filter(|p| p.filters.get(g).contains(&k)).count();
            GroupValue { label: g.value_label(&k), selected: f.selected.get(g).contains(&k), key: k, count }
        })
        .collect()
}

/// How many entries each tab would show with the rest of the filter as it is.
pub fn source_counts(items: &[(String, PersonaInfo)], f: &ExploreFilter) -> Vec<(ExploreSource, usize)> {
    let terms = query_terms(&f.query);
    let base: Vec<&PersonaInfo> =
        items.iter().map(|(_, p)| p).filter(|p| f.matches_kind(p) && matches_groups(&p.filters, &f.selected) && matches_query(p, &terms)).collect();
    ExploreSource::ALL.iter().map(|s| (*s, base.iter().filter(|p| s.matches(p)).count())).collect()
}

/// One value of a filter group as Entdecken offers it.
#[derive(Debug, Clone, PartialEq, serde::Serialize)]
pub struct GroupValue {
    pub key: String,
    pub label: String,
    /// Entries it would show (with the other filters as they are).
    pub count: usize,
    pub selected: bool,
}
