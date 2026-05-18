use std::collections::{BTreeSet, HashMap};

pub struct TreeNode {
    pub title: String,
    pub full_path: String,
    pub depth: usize,
    pub expanded: bool,
    pub has_children: bool,
    pub fact_count: usize,
}

pub fn build(section_paths: &[Vec<String>]) -> Vec<TreeNode> {
    let mut all_prefixes: BTreeSet<Vec<String>> = BTreeSet::new();
    let mut counts: HashMap<String, usize> = HashMap::new();

    for path in section_paths {
        if path.is_empty() {
            continue;
        }
        let key = path.join("/");
        *counts.entry(key).or_default() += 1;
        for i in 1..=path.len() {
            all_prefixes.insert(path[..i].to_vec());
        }
    }

    all_prefixes
        .iter()
        .map(|prefix| {
            let full_path = prefix.join("/");
            let depth = prefix.len() - 1;
            let has_children = all_prefixes
                .iter()
                .any(|p| p.len() == prefix.len() + 1 && p[..prefix.len()] == prefix[..]);
            TreeNode {
                title: prefix.last().unwrap().clone(),
                full_path,
                depth,
                expanded: true,
                has_children,
                fact_count: counts.get(&prefix.join("/")).copied().unwrap_or(0),
            }
        })
        .collect()
}

pub fn visible_indices(nodes: &[TreeNode]) -> Vec<usize> {
    let mut visible = Vec::new();
    let mut skip_below: Option<usize> = None;

    for (i, node) in nodes.iter().enumerate() {
        if let Some(d) = skip_below {
            if node.depth > d {
                continue;
            } else {
                skip_below = None;
            }
        }
        visible.push(i);
        if node.has_children && !node.expanded {
            skip_below = Some(node.depth);
        }
    }
    visible
}

#[cfg(test)]
mod tests {
    use super::*;

    fn paths(strs: &[&str]) -> Vec<Vec<String>> {
        strs.iter()
            .map(|s| {
                if s.is_empty() {
                    vec![]
                } else {
                    s.split('/').map(String::from).collect()
                }
            })
            .collect()
    }

    #[test]
    fn test_build_basic() {
        let p = paths(&["cli/add", "cli/check", "format"]);
        let nodes = build(&p);
        assert_eq!(nodes.len(), 4); // cli, cli/add, cli/check, format
        assert_eq!(nodes[0].title, "cli");
        assert_eq!(nodes[0].depth, 0);
        assert!(nodes[0].has_children);
        assert_eq!(nodes[0].fact_count, 0); // no facts directly in "cli"
        assert_eq!(nodes[1].title, "add");
        assert_eq!(nodes[1].depth, 1);
        assert_eq!(nodes[1].fact_count, 1);
        assert_eq!(nodes[3].title, "format");
        assert_eq!(nodes[3].depth, 0);
        assert_eq!(nodes[3].fact_count, 1);
    }

    #[test]
    fn test_build_counts_duplicates() {
        let p = paths(&["cli/add", "cli/add", "cli/add"]);
        let nodes = build(&p);
        let add_node = nodes.iter().find(|n| n.title == "add").unwrap();
        assert_eq!(add_node.fact_count, 3);
    }

    #[test]
    fn test_build_skips_empty_paths() {
        let p = paths(&["", "cli"]);
        let nodes = build(&p);
        assert_eq!(nodes.len(), 1);
        assert_eq!(nodes[0].title, "cli");
    }

    #[test]
    fn test_visible_all_expanded() {
        let p = paths(&["cli/add", "cli/check", "format"]);
        let nodes = build(&p);
        let vis = visible_indices(&nodes);
        assert_eq!(vis.len(), 4);
    }

    #[test]
    fn test_visible_collapsed() {
        let p = paths(&["cli/add", "cli/check", "format"]);
        let mut nodes = build(&p);
        nodes[0].expanded = false; // collapse "cli"
        let vis = visible_indices(&nodes);
        assert_eq!(vis.len(), 2); // cli (collapsed) + format
        assert_eq!(nodes[vis[0]].title, "cli");
        assert_eq!(nodes[vis[1]].title, "format");
    }

    #[test]
    fn test_deep_nesting() {
        let p = paths(&["a/b/c/d"]);
        let nodes = build(&p);
        assert_eq!(nodes.len(), 4); // a, a/b, a/b/c, a/b/c/d
        assert_eq!(nodes[0].depth, 0);
        assert_eq!(nodes[3].depth, 3);
        assert_eq!(nodes[3].fact_count, 1);
    }
}
