//! How CPS groups parameters on its screens (categories → pages → groups → parameters), exported
//! from `XMLToolCompactData.sdf` by `tools/export-rcdb-options.ps1` to `ui.json` next to the option
//! catalogue. Parameters are layout field ids ([`crate::layout::FieldDef::id`]). Hytera data: it is
//! loaded from the user's data folder, never shipped.

use std::collections::{BTreeMap, HashMap};
use std::path::{Path, PathBuf};

use serde::Deserialize;

use crate::{Error, Result};

#[derive(Debug, Clone, PartialEq)]
pub struct Group {
    pub name: String,
    /// Layout field ids, in CPS's order.
    pub params: Vec<u32>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Page {
    pub name: String,
    /// CPS's page type: 0 basic, 1 one-dimensional list, 2 two-level list.
    pub kind: i64,
    /// Groups with parameters, nested groups flattened in order.
    pub groups: Vec<Group>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Category {
    pub name: String,
    pub pages: Vec<Page>,
    pub children: Vec<Category>,
}

/// The screen structure for one RCDB version, from the model's root categories down.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct UiTree {
    pub rcdb_version: String,
    pub roots: Vec<Category>,
}

#[derive(Deserialize)]
struct Raw {
    rcdb_version: String,
    roots: Vec<(i64, i64)>,
    categories: Vec<(i64, String)>,
    category_children: Vec<(i64, i64, i64)>,
    category_pages: Vec<(i64, i64, i64)>,
    pages: Vec<(i64, String, i64)>,
    page_groups: Vec<(i64, i64, i64)>,
    groups: Vec<(i64, String)>,
    group_children: Vec<(i64, i64, i64)>,
    group_params: Vec<(i64, i64, i64)>,
}

/// `parent → [child]` ordered by index.
fn links(rows: &[(i64, i64, i64)]) -> HashMap<i64, Vec<i64>> {
    let mut map: HashMap<i64, Vec<(i64, i64)>> = HashMap::new();
    for &(parent, child, index) in rows {
        map.entry(parent).or_default().push((index, child));
    }
    map.into_iter()
        .map(|(k, mut v)| {
            v.sort();
            (k, v.into_iter().map(|(_, c)| c).collect())
        })
        .collect()
}

impl UiTree {
    pub fn parse(json: &str) -> Result<Self> {
        let raw: Raw = serde_json::from_str(json.trim_start_matches('\u{FEFF}'))
            .map_err(|e| Error::Invalid(format!("ui JSON: {e}")))?;
        let category_names: HashMap<i64, String> = raw.categories.into_iter().collect();
        let page_info: HashMap<i64, (String, i64)> = raw.pages.into_iter().map(|(id, n, k)| (id, (n, k))).collect();
        let group_names: HashMap<i64, String> = raw.groups.into_iter().collect();
        let (category_children, category_pages) = (links(&raw.category_children), links(&raw.category_pages));
        let (page_groups, group_children) = (links(&raw.page_groups), links(&raw.group_children));
        let group_params = links(&raw.group_params);

        fn flatten(
            id: i64,
            names: &HashMap<i64, String>,
            children: &HashMap<i64, Vec<i64>>,
            params: &HashMap<i64, Vec<i64>>,
            depth: usize,
            out: &mut Vec<Group>,
        ) {
            if depth > 16 {
                return; // guard against cycles
            }
            let own: Vec<u32> = params.get(&id).into_iter().flatten().filter_map(|&p| u32::try_from(p).ok()).collect();
            if !own.is_empty() {
                out.push(Group { name: names.get(&id).cloned().unwrap_or_default(), params: own });
            }
            for &c in children.get(&id).into_iter().flatten() {
                flatten(c, names, children, params, depth + 1, out);
            }
        }

        let build_page = |id: i64| -> Option<Page> {
            let (name, kind) = page_info.get(&id)?.clone();
            let mut groups = Vec::new();
            for &g in page_groups.get(&id).into_iter().flatten() {
                flatten(g, &group_names, &group_children, &group_params, 0, &mut groups);
            }
            Some(Page { name, kind, groups })
        };

        fn build_category(
            id: i64,
            depth: usize,
            names: &HashMap<i64, String>,
            children: &HashMap<i64, Vec<i64>>,
            pages: &HashMap<i64, Vec<i64>>,
            build_page: &dyn Fn(i64) -> Option<Page>,
        ) -> Category {
            Category {
                name: names.get(&id).cloned().unwrap_or_default(),
                pages: pages.get(&id).into_iter().flatten().filter_map(|&p| build_page(p)).collect(),
                children: if depth > 16 {
                    Vec::new()
                } else {
                    children
                        .get(&id)
                        .into_iter()
                        .flatten()
                        .map(|&c| build_category(c, depth + 1, names, children, pages, build_page))
                        .collect()
                },
            }
        }

        let mut roots = raw.roots;
        roots.sort_by_key(|&(_, index)| index);
        let roots = roots
            .into_iter()
            .map(|(id, _)| build_category(id, 0, &category_names, &category_children, &category_pages, &build_page))
            .collect();
        Ok(UiTree { rcdb_version: raw.rcdb_version, roots })
    }

    pub fn load(path: &Path) -> Result<Self> {
        let text = std::fs::read_to_string(path).map_err(|source| Error::Io { path: path.to_path_buf(), source })?;
        Self::parse(&text).map_err(|e| Error::Invalid(format!("{}: {e}", path.display())))
    }

    /// Where the exporter writes `ui.json` for `version` (next to the option catalogue).
    pub fn default_path(version: &str) -> Option<PathBuf> {
        Some(crate::options::Options::default_path(version)?.with_file_name("ui.json"))
    }

    /// Every (category path, page) pair, depth first, in CPS's order.
    pub fn pages(&self) -> Vec<(Vec<&str>, &Page)> {
        fn walk<'a>(c: &'a Category, path: &mut Vec<&'a str>, out: &mut Vec<(Vec<&'a str>, &'a Page)>) {
            path.push(&c.name);
            for p in &c.pages {
                out.push((path.clone(), p));
            }
            for child in &c.children {
                walk(child, path, out);
            }
            path.pop();
        }
        let mut out = Vec::new();
        for r in &self.roots {
            walk(r, &mut Vec::new(), &mut out);
        }
        out
    }
}

/// Count of parameters per page name, for diagnostics.
pub fn page_sizes(tree: &UiTree) -> BTreeMap<String, usize> {
    tree.pages().into_iter().map(|(_, p)| (p.name.clone(), p.groups.iter().map(|g| g.params.len()).sum())).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE: &str = r#"{"rcdb_version":"V1","roots":[[2,1],[1,0]],
        "categories":[[1,"Radio Information"],[2,"Common"],[3,"Basic Setting"]],
        "category_children":[[2,3,0]],
        "category_pages":[[1,10,0],[3,12,1],[3,11,0]],
        "pages":[[10,"Radio Information",0],[11,"Radio Alias",0],[12,"Power",0]],
        "page_groups":[[11,100,0],[12,101,0]],
        "groups":[[100,"Radio Alias"],[101,"Power"],[102,"Thresholds"]],
        "group_children":[[101,102,0]],
        "group_params":[[100,1103,0],[101,7,1],[101,6,0],[102,8,0]]}"#;

    #[test]
    fn builds_the_tree_in_cps_order() {
        let t = UiTree::parse(SAMPLE).unwrap();
        let names: Vec<(Vec<&str>, &str)> = t.pages().into_iter().map(|(path, p)| (path, p.name.as_str())).collect();
        assert_eq!(
            names,
            vec![
                (vec!["Radio Information"], "Radio Information"),
                (vec!["Common", "Basic Setting"], "Radio Alias"),
                (vec!["Common", "Basic Setting"], "Power"),
            ]
        );
        let power = t.pages()[2].1;
        assert_eq!(
            power.groups,
            vec![Group { name: "Power".into(), params: vec![6, 7] }, Group { name: "Thresholds".into(), params: vec![8] }]
        );
        assert_eq!(page_sizes(&t)["Power"], 3);
    }
}
