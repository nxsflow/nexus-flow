//! The relationship matrix (sp6.6 ②b → sp6.4 ③): the plugin-declared **type system** the write seam
//! enforces over the meaning-free core's `parent` edge. A plugin declares, in its `[types]` table,
//! the full **type list**, which **parent→child type pairs** are allowed, the **cardinality** (how
//! many parents) per child type, and a global **depth limit**. The core stays opinion-free about the
//! number of levels and the parent cardinality; both are the plugin's, enforced only here and at the
//! convergence-time structural backstop in `invariant`.
//!
//! Single-parent is a *matrix* rule (cardinality [`Cardinality::Single`]), not a core hardcode: it
//! makes a `set parent` re-point. [`Cardinality::Many`] lets parents accumulate without bound (the
//! seam personal-todo exercises). [`RelationshipMatrix::default`] reproduces the pre-`[types]`
//! behaviour (the only container is `project`, single-parent, unlimited depth) for any config that
//! omits the table.

use serde::{Deserialize, Deserializer};
use std::collections::{BTreeMap, BTreeSet};

/// How many parents a child type may have. The default, `Single`, makes a `set parent` *re-point*
/// (clear the old parent(s), attach the new) — the bundled issue-tracker's "at most one parent"
/// rule. `Many` lets parents accumulate without bound (personal-todo's "any number of `project`
/// parents"). `Limited(n)` caps the set at `n`. Deserialized from a positive integer (`1` →
/// `Single`, `n` → `Limited(n)`) or the string `"many"`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Cardinality {
    /// Exactly one parent: a `set parent` re-points (cardinality 1).
    #[default]
    Single,
    /// Up to `n` parents; the `n`-th+1 is a loud reject.
    Limited(usize),
    /// Any number of parents — never rejected on cardinality.
    Many,
}

impl<'de> Deserialize<'de> for Cardinality {
    /// Accept a positive integer (`1` ⇒ `Single`, `n` ⇒ `Limited(n)`) or the string `"many"`.
    /// `0` is rejected loudly — a parentable type has at least one parent slot; a type that may
    /// not be parented simply has no rule.
    fn deserialize<D>(d: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        #[derive(Deserialize)]
        #[serde(untagged)]
        enum Raw {
            Int(u64),
            Str(String),
        }
        match Raw::deserialize(d)? {
            Raw::Int(0) => Err(serde::de::Error::custom(
                "cardinality must be ≥ 1 (a parentable type has at least one parent slot); \
                 omit the rule entirely for a root type",
            )),
            Raw::Int(1) => Ok(Cardinality::Single),
            Raw::Int(n) => Ok(Cardinality::Limited(n as usize)),
            Raw::Str(s) if s == "many" => Ok(Cardinality::Many),
            Raw::Str(s) => Err(serde::de::Error::custom(format!(
                "unknown cardinality '{s}'; expected a positive integer or \"many\""
            ))),
        }
    }
}

/// One child type's parenthood rule: the parent types it may attach to and how many parents it may
/// have. A child type with NO rule may not be parented at all (orphan only).
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct ParentRule {
    /// The parent types a child of this type may attach to. Empty ⇒ a root type (no parent).
    #[serde(default)]
    pub parents: BTreeSet<String>,
    /// How many parents a child of this type may have. Defaults to [`Cardinality::Single`].
    #[serde(default)]
    pub max: Cardinality,
}

/// The plugin-declared type system — the `[types]` table. Core-enforced at write time by
/// [`RelationshipMatrix::validate_parent`] (via the facade's `check_parent`).
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct RelationshipMatrix {
    /// Every type the plugin allows — the authoritative valid set `create --type` checks against.
    /// A root container (e.g. `epic`/`project`) appears here but has no `rules` entry.
    #[serde(default)]
    pub list: BTreeSet<String>,
    /// `child_type → rule`. A child type absent here may not have a parent (orphan only).
    #[serde(default)]
    pub rules: BTreeMap<String, ParentRule>,
    /// Max hierarchy depth — the most levels any root→leaf path may have. `None` ⇒ unlimited.
    #[serde(default)]
    pub max_depth: Option<usize>,
}

impl Default for RelationshipMatrix {
    /// Pre-`[types]` behaviour (the require_live_project + single-parent rules ②a preserved): the
    /// only container is `project`; both `task` and `project` may have a single `project` parent;
    /// depth is unlimited. Used for any config that omits the `[types]` table.
    fn default() -> Self {
        let parent_project = || ParentRule {
            parents: BTreeSet::from(["project".to_string()]),
            max: Cardinality::Single,
        };
        RelationshipMatrix {
            list: BTreeSet::from(["project".to_string(), "task".to_string()]),
            rules: BTreeMap::from([
                ("task".to_string(), parent_project()),
                ("project".to_string(), parent_project()),
            ]),
            max_depth: None,
        }
    }
}

/// How a valid parent write should land given the child type's cardinality (sp6.6). The store has
/// two primitives; the matrix decides which one a `set parent` maps to.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ParentWriteMode {
    /// Cardinality `Single`: re-point — clear the child's current parent(s), then add this one.
    Replace,
    /// Cardinality `Many`/`Limited` (still under the limit): add this parent to the existing set.
    Add,
}

/// Why a parent write was rejected — a closed set surfaced as a `validation` envelope by the caller.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MatrixViolation {
    /// The `parent_type → child_type` pair is not declared allowed.
    Pair,
    /// The child is already at its parent-cardinality limit.
    Cardinality,
    /// The edge would push a root→leaf path past the depth limit.
    Depth,
}

impl RelationshipMatrix {
    /// Validate internal consistency at load time: every rule's child type and every parent it names
    /// must be a declared type in [`list`](Self::list). A dangling reference is a loud error (the
    /// loader surfaces it as a `validation` envelope) instead of a silently unreachable rule.
    pub fn validate(&self) -> Result<(), String> {
        for (child, rule) in &self.rules {
            if !self.list.contains(child) {
                return Err(format!(
                    "type rule for undeclared type '{child}'; add it to types.list"
                ));
            }
            for parent in &rule.parents {
                if !self.list.contains(parent) {
                    return Err(format!(
                        "rule for '{child}' allows undeclared parent type '{parent}'; \
                         add it to types.list"
                    ));
                }
            }
        }
        Ok(())
    }

    /// Is `t` a declared type of this plugin?
    pub fn is_type(&self, t: &str) -> bool {
        self.list.contains(t)
    }

    /// The plugin's representative "first item" type for onboarding CTAs (nexus-flow-92zt): the
    /// ROOT container — the first declared type with no parent rule, so it can only be a root
    /// (e.g. `epic` / `project`) — which is always a valid `--type` and the natural first thing to
    /// create. Falls back to the first declared type if every type is parentable, and `None` only
    /// for an (invalid) empty type set.
    pub fn example_type(&self) -> Option<&str> {
        self.list
            .iter()
            .find(|t| !self.rules.contains_key(*t))
            .or_else(|| self.list.iter().next())
            .map(String::as_str)
    }

    /// The pure write-time decision: given the candidate edge's `parent_type`/`child_type`, the
    /// child's CURRENT parent count (excluding the candidate), and the resulting hierarchy `depth`
    /// through the edge, either approve it with a [`ParentWriteMode`] or reject it with a
    /// [`MatrixViolation`]. `depth` is only consulted when `max_depth` is set, so the caller may pass
    /// `0` when it is `None`. Single-parent falls out as `Cardinality::Single` ⇒ `Replace` — no core
    /// hardcode.
    pub fn validate_parent(
        &self,
        parent_type: &str,
        child_type: &str,
        existing_parents: usize,
        depth: usize,
    ) -> Result<ParentWriteMode, MatrixViolation> {
        if !self.pair_allowed(parent_type, child_type) {
            return Err(MatrixViolation::Pair);
        }
        if let Some(limit) = self.max_depth {
            if depth > limit {
                return Err(MatrixViolation::Depth);
            }
        }
        // pair_allowed ⇒ a rule for child_type exists, so the cardinality is always `Some`.
        match self.cardinality(child_type).unwrap_or_default() {
            Cardinality::Single => Ok(ParentWriteMode::Replace),
            Cardinality::Many => Ok(ParentWriteMode::Add),
            Cardinality::Limited(n) => {
                if existing_parents < n {
                    Ok(ParentWriteMode::Add)
                } else {
                    Err(MatrixViolation::Cardinality)
                }
            }
        }
    }

    /// Is a `parent_type → child_type` parent edge allowed by the matrix?
    pub fn pair_allowed(&self, parent_type: &str, child_type: &str) -> bool {
        self.rules
            .get(child_type)
            .is_some_and(|r| r.parents.contains(parent_type))
    }

    /// The child type's cardinality, or `None` if the type may not be parented at all (no rule).
    pub fn cardinality(&self, child_type: &str) -> Option<Cardinality> {
        self.rules.get(child_type).map(|r| r.max)
    }

    /// The finite parent cap for `child_type`, if any — `Some(n)` for `Limited(n)`, `None` for
    /// `Single`/`Many`/an unparentable type. Only meaningful for the cardinality-exceeded message.
    pub fn parent_limit(&self, child_type: &str) -> Option<usize> {
        match self.cardinality(child_type) {
            Some(Cardinality::Limited(n)) => Some(n),
            _ => None,
        }
    }

    /// The accepted parent types for `child_type`, sorted, for a self-correcting error message.
    pub fn allowed_parents(&self, child_type: &str) -> Vec<String> {
        self.rules
            .get(child_type)
            .map(|r| r.parents.iter().cloned().collect())
            .unwrap_or_default()
    }

    /// Is `t` a container — a type some other type may be parented under? Derived: `t` appears in
    /// some rule's `parents` set. A leaf type (never a parent) is not a container even if it is a
    /// declared type. This is the containment role `nxf schema` / `flow_schema` expose (y5j8).
    pub fn is_container(&self, t: &str) -> bool {
        self.rules.values().any(|r| r.parents.contains(t))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_matrix_reproduces_pre_matrix_behaviour() {
        let m = RelationshipMatrix::default();
        // parent must be a project; task/project are the only child types.
        assert!(m.pair_allowed("project", "task"));
        assert!(m.pair_allowed("project", "project"));
        assert!(
            !m.pair_allowed("task", "task"),
            "a task may not be a parent"
        );
        assert!(
            !m.pair_allowed("project", "milestone"),
            "unknown child type"
        );
        // single-parent for everything; unlimited depth.
        assert_eq!(m.cardinality("task"), Some(Cardinality::Single));
        assert_eq!(m.cardinality("project"), Some(Cardinality::Single));
        assert_eq!(
            m.cardinality("milestone"),
            None,
            "untyped child cannot be parented"
        );
        assert_eq!(m.max_depth, None);
        // the valid type set is the two roles.
        assert!(m.is_type("project") && m.is_type("task"));
        assert!(!m.is_type("epic"));
    }

    #[test]
    fn example_type_is_the_root_container_of_each_plugin_shape() {
        // The onboarding "create your first item" example must name a type the active plugin
        // actually declares (nexus-flow-92zt): the old hardcoded `task` is in NEITHER bundled
        // plugin. example_type() picks the ROOT container — the declared type with no parent rule
        // (epic / project) — which is always a valid `--type` and the natural first item.
        let issue_tracker: RelationshipMatrix = toml::from_str(
            r#"
            list = ["epic", "bug", "feature", "chore", "decision"]
            max_depth = 2
            [rules.bug]
            parents = ["epic"]
            [rules.feature]
            parents = ["epic"]
            [rules.chore]
            parents = ["epic"]
            [rules.decision]
            parents = ["epic"]
        "#,
        )
        .unwrap();
        assert_eq!(issue_tracker.example_type(), Some("epic"));

        let personal_todo: RelationshipMatrix = toml::from_str(
            r#"
            list = ["project", "todo", "termin"]
            max_depth = 2
            [rules.todo]
            parents = ["project"]
            max = "many"
            [rules.termin]
            parents = ["project"]
            max = "many"
        "#,
        )
        .unwrap();
        assert_eq!(personal_todo.example_type(), Some("project"));

        // Degenerate: no root type (every type parentable) → fall back to the first declared type,
        // still a valid `--type`.
        let no_root: RelationshipMatrix = toml::from_str(
            r#"
            list = ["a", "b"]
            [rules.a]
            parents = ["b"]
            [rules.b]
            parents = ["a"]
        "#,
        )
        .unwrap();
        assert_eq!(no_root.example_type(), Some("a"));

        // Empty list → None (an invalid plugin never loads this far; guard anyway).
        let empty = RelationshipMatrix {
            list: BTreeSet::new(),
            rules: BTreeMap::new(),
            max_depth: None,
        };
        assert_eq!(empty.example_type(), None);
    }

    #[test]
    fn cardinality_deserializes_from_int_or_many() {
        #[derive(Deserialize)]
        struct Holder {
            a: Cardinality,
            b: Cardinality,
            c: Cardinality,
        }
        let h: Holder = toml::from_str(
            r#"a = 1
b = 3
c = "many""#,
        )
        .unwrap();
        assert_eq!(h.a, Cardinality::Single);
        assert_eq!(h.b, Cardinality::Limited(3));
        assert_eq!(h.c, Cardinality::Many);
        // 0 and unknown strings are loud rejects.
        assert!(toml::from_str::<Holder>("a = 0\nb = 1\nc = 1").is_err());
        assert!(toml::from_str::<Holder>("a = \"lots\"\nb = 1\nc = 1").is_err());
    }

    #[test]
    fn validate_parent_enforces_pair_cardinality_and_depth() {
        // The pure write-time decision — a test matrix exercising all axes. `Single` ⇒ Replace
        // (re-point); `Limited(n)` ⇒ Add until the limit, then a Cardinality reject; `Many` ⇒ Add.
        let m: RelationshipMatrix = toml::from_str(
            r#"
            list = ["epic", "bug", "project", "todo"]
            max_depth = 2
            [rules.todo]
            parents = ["project"]
            max = 2
            [rules.bug]
            parents = ["epic"]
            max = 1
        "#,
        )
        .unwrap();
        use MatrixViolation::*;
        use ParentWriteMode::*;
        // pair + cardinality mode
        assert_eq!(
            m.validate_parent("epic", "bug", 0, 2),
            Ok(Replace),
            "Single → re-point"
        );
        assert_eq!(
            m.validate_parent("project", "todo", 0, 2),
            Ok(Add),
            "Limited(2) under the cap → add"
        );
        assert_eq!(
            m.validate_parent("epic", "todo", 0, 2),
            Err(Pair),
            "epic is not an allowed parent of todo"
        );
        // cardinality limit (todo: max 2)
        assert_eq!(
            m.validate_parent("project", "todo", 1, 2),
            Ok(Add),
            "1 existing < 2"
        );
        assert_eq!(
            m.validate_parent("project", "todo", 2, 2),
            Err(Cardinality),
            "already at the 2-parent limit"
        );
        // depth limit (2): exactly at the cap is fine, beyond is rejected
        assert_eq!(m.validate_parent("project", "todo", 0, 2), Ok(Add));
        assert_eq!(m.validate_parent("project", "todo", 0, 3), Err(Depth));
    }

    #[test]
    fn many_cardinality_never_rejects_on_count() {
        // personal-todo's "beliebig viele": a `Many` child accumulates parents without an upper
        // bound — even a high existing count still adds.
        let m: RelationshipMatrix = toml::from_str(
            r#"
            list = ["project", "todo"]
            [rules.todo]
            parents = ["project"]
            max = "many"
        "#,
        )
        .unwrap();
        assert_eq!(m.cardinality("todo"), Some(Cardinality::Many));
        assert_eq!(m.parent_limit("todo"), None, "no finite cap");
        assert_eq!(
            m.validate_parent("project", "todo", 0, 0),
            Ok(ParentWriteMode::Add)
        );
        assert_eq!(
            m.validate_parent("project", "todo", 1_000, 0),
            Ok(ParentWriteMode::Add),
            "Many never hits a cardinality wall"
        );
    }

    #[test]
    fn matrix_deserializes_from_the_sp6_4_types_shape() {
        // The shape sp6.4's `[types]` produces: an explicit type list + per-child parent sets +
        // cardinality + a depth cap. A root type is in `list` but has no rule.
        let m: RelationshipMatrix = toml::from_str(
            r#"
            list = ["project", "todo"]
            max_depth = 2
            [rules.todo]
            parents = ["project"]
            max = "many"
        "#,
        )
        .unwrap();
        assert_eq!(m.max_depth, Some(2));
        assert!(m.pair_allowed("project", "todo"));
        assert_eq!(m.cardinality("todo"), Some(Cardinality::Many));
        assert!(m.is_type("project") && m.is_type("todo"));
        assert!(
            !m.pair_allowed("project", "project"),
            "project is a root (no rule)"
        );
        assert_eq!(
            m.cardinality("project"),
            None,
            "a root type has no parent rule"
        );
        m.validate().expect("rules reference only declared types");
    }

    #[test]
    fn is_container_is_true_for_a_declared_parent_type() {
        // A container is exactly a type some other type may be filed under (appears in a rule's
        // `parents`). Derived — no new TOML field. `context` is a container; `note` (a leaf) is not.
        let m: RelationshipMatrix = toml::from_str(
            r#"
            list = ["context", "project", "note"]
            max_depth = 2
            [rules.note]
            parents = ["context"]
            max = 1
        "#,
        )
        .unwrap();
        assert!(m.is_container("context"), "context is a parent of note");
        assert!(!m.is_container("note"), "note is a leaf, never a parent");
        assert!(
            !m.is_container("project"),
            "project is declared but no type parents under it"
        );
        // The default matrix: project is the only container.
        let d = RelationshipMatrix::default();
        assert!(d.is_container("project"));
        assert!(!d.is_container("task"));
    }

    #[test]
    fn validate_rejects_rules_referencing_undeclared_types() {
        // A rule whose child type is not in `list`.
        let m: RelationshipMatrix = toml::from_str(
            r#"
            list = ["epic"]
            [rules.bug]
            parents = ["epic"]
        "#,
        )
        .unwrap();
        assert!(m.validate().is_err(), "bug is not in types.list");

        // A rule that names a parent type not in `list`.
        let m: RelationshipMatrix = toml::from_str(
            r#"
            list = ["bug"]
            [rules.bug]
            parents = ["epic"]
        "#,
        )
        .unwrap();
        assert!(m.validate().is_err(), "epic (parent) is not in types.list");
    }
}
