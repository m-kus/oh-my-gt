//! The in-memory stack graph: every local branch, its tracked parent/children,
//! and a validation verdict. Built from git refs in a fixed handful of spawns.

use std::collections::{HashMap, HashSet, VecDeque};

use crate::error::{GtError, Result};
use crate::git;
use crate::meta::{self, BranchMetadata};
use crate::trunk;

/// Whether a branch's metadata places it soundly in the stack.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Validation {
    /// The repository trunk; has no metadata and no parent.
    Trunk,
    /// Tracked, with a resolvable parent and fork point.
    Valid,
    /// Local branch with no metadata ref.
    Untracked,
    /// `parentBranchName` is missing or names a non-existent branch.
    BadParentName,
    /// `parentBranchRevision` is missing or absent from the object database.
    BadParentRevision,
    /// The branch itself is fine, but an ancestor is not `Valid`/`Trunk`.
    InvalidParent,
}

impl Validation {
    pub fn is_usable(self) -> bool {
        matches!(self, Validation::Trunk | Validation::Valid)
    }
}

#[derive(Debug, Clone)]
pub struct BranchNode {
    pub name: String,
    pub tip: String,
    pub meta: Option<BranchMetadata>,
    pub validation: Validation,
    pub parent: Option<String>,
    pub children: Vec<String>,
}

pub struct StackGraph {
    pub nodes: HashMap<String, BranchNode>,
    pub trunk: String,
    pub current: Option<String>,
}

impl StackGraph {
    /// Build the graph for the current repository.
    pub fn load() -> Result<StackGraph> {
        let heads = list_heads()?;
        let meta_refs = list_meta_refs()?;
        let metas = load_metas(&meta_refs)?;
        let current = git::current_branch()?;
        let trunk = trunk::resolve(&heads)?;

        // Pre-resolve which recorded fork points still exist in the object db.
        let revs: Vec<String> = metas
            .values()
            .filter_map(|m| m.parent_branch_revision.clone())
            .collect();
        let live_revs = batch_existing(&revs)?;

        let mut nodes: HashMap<String, BranchNode> = HashMap::new();
        for (name, tip) in &heads {
            let meta = metas.get(name).cloned();
            nodes.insert(
                name.clone(),
                BranchNode {
                    name: name.clone(),
                    tip: tip.clone(),
                    parent: meta.as_ref().and_then(|m| m.parent_branch_name.clone()),
                    meta,
                    validation: Validation::Untracked,
                    children: Vec::new(),
                },
            );
        }

        let mut graph = StackGraph { nodes, trunk, current };
        graph.validate(&heads, &live_revs);
        graph.link_children();
        Ok(graph)
    }

    /// Load the graph and resolve the checked-out branch, erroring if detached.
    pub fn load_current() -> Result<(StackGraph, String)> {
        let graph = StackGraph::load()?;
        let current = graph.current.clone().ok_or_else(|| {
            GtError::Precondition("HEAD is detached; check out a branch first".into())
        })?;
        Ok((graph, current))
    }

    /// The node for a `Valid` tracked branch, or a "not tracked" error.
    pub fn require_tracked(&self, name: &str) -> Result<&BranchNode> {
        self.get(name)
            .filter(|n| n.validation == Validation::Valid)
            .ok_or_else(|| GtError::Usage(format!("`{name}` is not tracked; run `gt track` first")))
    }

    fn validate(&mut self, heads: &HashMap<String, String>, live_revs: &HashSet<String>) {
        let names: Vec<String> = self.nodes.keys().cloned().collect();
        for name in &names {
            let verdict = {
                let node = &self.nodes[name];
                if *name == self.trunk {
                    Validation::Trunk
                } else if node.meta.is_none() {
                    Validation::Untracked
                } else {
                    let m = node.meta.as_ref().unwrap();
                    match (&m.parent_branch_name, &m.parent_branch_revision) {
                        (None, _) => Validation::BadParentName,
                        (Some(p), _) if !heads.contains_key(p) => Validation::BadParentName,
                        (Some(_), None) => Validation::BadParentRevision,
                        (Some(_), Some(rev)) if !live_revs.contains(rev) => {
                            Validation::BadParentRevision
                        }
                        (Some(_), Some(_)) => Validation::Valid,
                    }
                }
            };
            self.nodes.get_mut(name).unwrap().validation = verdict;
        }

        // Propagate: a branch with a non-usable ancestor is `InvalidParent`.
        // Iterate to a fixpoint (stacks are shallow, so this is cheap).
        loop {
            let mut changed = false;
            for name in &names {
                let node = &self.nodes[name];
                if node.validation != Validation::Valid {
                    continue;
                }
                let parent = node.parent.clone().unwrap();
                let parent_ok = self.nodes.get(&parent).map(|p| p.validation.is_usable());
                if parent_ok != Some(true) {
                    self.nodes.get_mut(name).unwrap().validation = Validation::InvalidParent;
                    changed = true;
                }
            }
            if !changed {
                break;
            }
        }
    }

    fn link_children(&mut self) {
        let pairs: Vec<(String, String)> = self
            .nodes
            .values()
            .filter(|n| n.validation == Validation::Valid)
            .map(|n| (n.parent.clone().unwrap(), n.name.clone()))
            .collect();
        for (parent, child) in pairs {
            if let Some(p) = self.nodes.get_mut(&parent) {
                p.children.push(child);
            }
        }
        for n in self.nodes.values_mut() {
            n.children.sort();
        }
    }

    pub fn get(&self, name: &str) -> Option<&BranchNode> {
        self.nodes.get(name)
    }

    pub fn is_trunk(&self, name: &str) -> bool {
        name == self.trunk
    }

    /// Branches with `Valid` metadata, in no particular order.
    pub fn tracked(&self) -> Vec<&str> {
        self.nodes
            .values()
            .filter(|n| n.validation == Validation::Valid)
            .map(|n| n.name.as_str())
            .collect()
    }

    /// Does this branch's recorded fork point lag behind its parent's tip?
    pub fn needs_restack(&self, name: &str) -> bool {
        let Some(node) = self.nodes.get(name) else { return false };
        if node.validation != Validation::Valid {
            return false;
        }
        let parent = node.parent.as_deref().unwrap();
        let recorded = node.meta.as_ref().and_then(|m| m.parent_branch_revision.as_deref());
        let parent_tip = self.nodes.get(parent).map(|p| p.tip.as_str());
        match (recorded, parent_tip) {
            (Some(r), Some(t)) => r != t,
            _ => true,
        }
    }

    /// Self + all descendants, parents before children (topological).
    pub fn subtree(&self, root: &str) -> Vec<String> {
        let mut out = Vec::new();
        let mut queue = VecDeque::from([root.to_string()]);
        while let Some(name) = queue.pop_front() {
            out.push(name.clone());
            if let Some(node) = self.nodes.get(&name) {
                for c in &node.children {
                    queue.push_back(c.clone());
                }
            }
        }
        out
    }

    /// Descendants of a branch (excludes the branch itself), topological.
    pub fn descendants(&self, root: &str) -> Vec<String> {
        let mut s = self.subtree(root);
        s.remove(0);
        s
    }

    /// Ancestor chain from trunk down to (and including) `name`.
    pub fn path_from_trunk(&self, name: &str) -> Vec<String> {
        let mut chain = Vec::new();
        let mut cur = Some(name.to_string());
        while let Some(c) = cur {
            chain.push(c.clone());
            cur = self.nodes.get(&c).and_then(|n| n.parent.clone());
        }
        chain.reverse();
        chain
    }

    /// Would parenting `branch` onto `new_parent` create a cycle?
    pub fn would_cycle(&self, branch: &str, new_parent: &str) -> bool {
        new_parent == branch || self.descendants(branch).iter().any(|d| d == new_parent)
    }

    /// All branches in the stack containing `name` (its root subtree), topological.
    pub fn stack_of(&self, name: &str) -> Vec<String> {
        let root = self
            .path_from_trunk(name)
            .into_iter()
            .find(|b| self.nodes.get(b).map(|n| n.parent.is_some()).unwrap_or(false))
            .unwrap_or_else(|| name.to_string());
        self.subtree(&root)
    }
}

// ── git-backed loaders ──────────────────────────────────────────────────────

fn list_heads() -> Result<HashMap<String, String>> {
    let out = git::run(&[
        "for-each-ref",
        "--format=%(refname:short) %(objectname)",
        "refs/heads/",
    ])?;
    let mut map = HashMap::new();
    for line in out.lines() {
        if let Some((name, sha)) = line.split_once(' ') {
            map.insert(name.to_string(), sha.to_string());
        }
    }
    Ok(map)
}

fn list_meta_refs() -> Result<HashMap<String, String>> {
    let out = git::run(&[
        "for-each-ref",
        "--format=%(refname) %(objectname)",
        meta::META_REF_PREFIX,
    ])?;
    let mut map = HashMap::new();
    for line in out.lines() {
        if let Some((refname, sha)) = line.split_once(' ') {
            if let Some(branch) = refname.strip_prefix(meta::META_REF_PREFIX) {
                map.insert(branch.to_string(), sha.to_string());
            }
        }
    }
    Ok(map)
}

/// Read every metadata blob in a single `cat-file --batch` invocation.
fn load_metas(meta_refs: &HashMap<String, String>) -> Result<HashMap<String, BranchMetadata>> {
    if meta_refs.is_empty() {
        return Ok(HashMap::new());
    }
    let shas: Vec<String> = meta_refs.values().cloned().collect();
    let blobs = batch_read(&shas)?;
    let mut out = HashMap::new();
    for (branch, sha) in meta_refs {
        if let Some(content) = blobs.get(sha) {
            if let Ok(m) = meta::parse(content) {
                out.insert(branch.clone(), m);
            }
        }
    }
    Ok(out)
}

/// `cat-file --batch`: returns a map of object SHA -> blob contents.
fn batch_read(shas: &[String]) -> Result<HashMap<String, String>> {
    let mut input = shas.join("\n");
    input.push('\n');
    let out = git::run_with_stdin_raw(&["cat-file", "--batch"], input.as_bytes())?;
    let mut map = HashMap::new();
    let mut pos = 0;
    while pos < out.len() {
        let nl = out[pos..]
            .iter()
            .position(|&b| b == b'\n')
            .ok_or_else(|| GtError::Git("malformed cat-file --batch output".into()))?;
        let header = String::from_utf8_lossy(&out[pos..pos + nl]).to_string();
        pos += nl + 1;
        let mut parts = header.split(' ');
        let sha = parts.next().unwrap_or_default().to_string();
        let typ = parts.next().unwrap_or_default();
        if typ != "blob" {
            continue; // `missing` headers carry no body
        }
        let size: usize = parts
            .next()
            .and_then(|s| s.parse().ok())
            .ok_or_else(|| GtError::Git("malformed cat-file size".into()))?;
        let content = String::from_utf8_lossy(&out[pos..pos + size]).to_string();
        pos += size + 1; // skip the trailing newline git appends
        map.insert(sha, content);
    }
    Ok(map)
}

/// `cat-file --batch-check`: returns the subset of SHAs that exist.
fn batch_existing(shas: &[String]) -> Result<HashSet<String>> {
    if shas.is_empty() {
        return Ok(HashSet::new());
    }
    let mut input = shas.join("\n");
    input.push('\n');
    let out = git::run_with_stdin(&["cat-file", "--batch-check"], input.as_bytes())?;
    let mut set = HashSet::new();
    for line in out.lines() {
        let mut parts = line.split(' ');
        let token = parts.next().unwrap_or_default();
        let typ = parts.next().unwrap_or_default();
        if typ != "missing" && !token.is_empty() {
            set.insert(token.to_string());
        }
    }
    Ok(set)
}
