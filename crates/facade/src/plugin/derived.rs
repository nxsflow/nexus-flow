//! Derived ranking keys, the plugin side (docs/specs/derived-ranking-keys.md, 6j6v.x0wf).
//!
//! A key in a ranking `order` is a FIELD key (with or without a `precedence` list) or one of three
//! DERIVED keys that read the item's neighbours in the graph:
//!
//! - `related`: a value taken from the neighbours over one outbound edge kind (spec §3.1);
//! - `count`: how many neighbours over one edge kind and direction match (spec §3.2);
//! - `sum`: several counts added with equal weight (spec §3.3).
//!
//! This module holds their parts and the load-time rules that need the whole plugin: the filter
//! values, the fields `of` reads, which order `of = "rank"` reads, and the acyclic type chain
//! (spec §4, §7). The structural rules live with the other key rules in
//! [`RankSpec::validate_in`](super::RankSpec); the computation lives in the read layer.

use super::{Dir, FieldDecl, Nulls, RankKey, RankSpec, Ranking, LIST_SECTION, NEXT_SECTION};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};

/// The edge kinds a `related` key follows outward from the item (spec §3.1).
pub const RELATED_EDGES: &[&str] = &["parent", "contributes_to", "dep", "mentions"];

/// The kinds a `count` (and a `sum` summand) counts (spec §3.2): the four inbound names, then the
/// four outbound kinds of [`RELATED_EDGES`].
pub const COUNT_EDGES: &[&str] = &[
    "children",
    "contributors",
    "dependents",
    "mentioned_by",
    "parent",
    "contributes_to",
    "dep",
    "mentions",
];

/// How several neighbours of a `related` key become one value (spec §3.1).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, Serialize, Default)]
#[serde(rename_all = "lowercase")]
pub enum Pick {
    /// The smallest value — the best rank, the lowest field value. The default.
    #[default]
    Min,
    /// The largest value.
    Max,
}

/// What a `related` key reads on each neighbour (spec §3.1): `of = "rank"`, the neighbour's
/// position in its own type's declared order (spec §4), or `of = { field = ... }`, a field of the
/// neighbour — a canonical field or a custom field the plugin declares (spec §10.2).
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum RankOf {
    /// The neighbour's rank in its own type's declared order.
    Rank,
    /// A field of the neighbour.
    Field(OfField),
}

/// `of = { field = ..., precedence = [...] }`: the neighbour's field value, or — with a
/// `precedence` list — the value's position in that list (an absent or unlisted value after every
/// listed one, as for a precedence key).
///
/// `dir` and `nulls` are accepted at their defaults only (`asc`, `last`), so the spec's own example
/// `of = { field = "order", dir = "asc" }` loads. Direction and null placement belong to the
/// `related` key that holds the `of`; a non-default one here would be a silent no-op, so it is
/// refused at load, as `dir` on a precedence key is.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
#[non_exhaustive]
pub struct OfField {
    pub field: String,
    #[serde(default)]
    pub dir: Dir,
    #[serde(default)]
    pub nulls: Nulls,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub precedence: Vec<String>,
}

impl<'de> Deserialize<'de> for RankOf {
    fn deserialize<D: serde::Deserializer<'de>>(d: D) -> std::result::Result<RankOf, D::Error> {
        use serde::de::Error;
        match toml::Value::deserialize(d)? {
            toml::Value::String(s) if s == "rank" => Ok(RankOf::Rank),
            toml::Value::String(s) => Err(D::Error::custom(format!(
                "of = \"{s}\": the only name `of` takes is \"rank\"; a field is \
                 `of = {{ field = \"...\" }}`"
            ))),
            v @ toml::Value::Table(_) => OfField::deserialize(v)
                .map(RankOf::Field)
                .map_err(|e| D::Error::custom(format!("of: {e}"))),
            other => Err(D::Error::custom(format!(
                "of = {other}: expected \"rank\" or `{{ field = \"...\" }}`"
            ))),
        }
    }
}

impl Serialize for RankOf {
    fn serialize<S: serde::Serializer>(&self, s: S) -> std::result::Result<S::Ok, S::Error> {
        match self {
            RankOf::Rank => s.serialize_str("rank"),
            RankOf::Field(f) => f.serialize(s),
        }
    }
}

/// One summand of a `sum` key (spec §3.3): exactly the parts of a `count` — the kind and its
/// optional `type`, `status` and `label` filters. `dir` belongs to the `sum`, so a summand refuses
/// it at parse, as it refuses any other part. A per-summand `weight` is deliberately left out
/// (spec §10.3); `#[non_exhaustive]` keeps room to add it.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
#[non_exhaustive]
pub struct CountPart {
    pub count: String,
    #[serde(default, rename = "type", skip_serializing_if = "Option::is_none")]
    pub item_type: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub status: Option<Vec<String>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub label: Option<String>,
}

/// Which ranking policy a key stands in, and so which section it is spelled under.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub(crate) enum Verb {
    /// `[ranking.next]`.
    Next,
    /// `[ranking.list]`.
    List,
}

impl Verb {
    /// The TOML section of this verb's policy.
    pub(crate) fn section(self) -> &'static str {
        match self {
            Verb::Next => NEXT_SECTION,
            Verb::List => LIST_SECTION,
        }
    }

    /// The other verb.
    fn other(self) -> Verb {
        match self {
            Verb::Next => Verb::List,
            Verb::List => Verb::Next,
        }
    }
}

impl Ranking {
    /// The policy of `verb`.
    pub(crate) fn spec(&self, verb: Verb) -> &RankSpec {
        match verb {
            Verb::Next => &self.next,
            Verb::List => &self.list,
        }
    }

    /// The declared order whose rank `of = "rank"` reads for a neighbour of type `ty`, asked from a
    /// key under `verb` (spec §4: the type's rank under "the verb that ranks it"): `verb`'s own
    /// `[ranking.<verb>.<ty>]` when it declares one, else the other verb's. A person ranked by
    /// `list` can so rank by the rank of the projects `next` ranks (spec §8). `None` when neither
    /// declares an order for `ty`: such a neighbour has no rank and reads as null.
    pub(crate) fn rank_source(&self, verb: Verb, ty: &str) -> Option<Verb> {
        [verb, verb.other()]
            .into_iter()
            .find(|v| self.spec(*v).by_type.contains_key(ty))
    }

    /// Whether the policy of `verb` holds a derived key anywhere.
    pub(crate) fn has_derived(&self, verb: Verb) -> bool {
        self.spec(verb).has_derived()
    }

    /// The load-time rules of the derived keys that need the whole plugin (spec §7), checked after
    /// the structural rules of each section ([`RankSpec::validate_in`]/`validate_types_in`):
    ///
    /// - a `status` filter names only statuses of `[vocabulary.status]`;
    /// - `of = { field = ... }` names a field the ranking can read
    ///   ([`RANKABLE_FIELDS`](crate::record::RANKABLE_FIELDS)) or a custom field of `[fields]`,
    ///   and a custom field scoped `on` some types applies to the key's `type`;
    /// - `of = "rank"` with a `type` names a type with a declared order
    ///   (`[ranking.next.<type>]` or `[ranking.list.<type>]`);
    /// - the type graph is acyclic: an edge from each per-type order to every order its keys read
    ///   the rank of; a cycle is refused with the cycle named (spec §4).
    pub(crate) fn validate_derived(
        &self,
        statuses: &BTreeMap<String, String>,
        fields: &BTreeMap<String, FieldDecl>,
    ) -> std::result::Result<(), String> {
        for verb in [Verb::Next, Verb::List] {
            let section = verb.section();
            for (scope, order) in self.spec(verb).orders(section) {
                for key in order.iter().filter(|k| k.is_derived()) {
                    self.validate_derived_key(verb, &scope, key, statuses, fields)?;
                }
            }
        }
        self.check_type_chain()
    }

    fn validate_derived_key(
        &self,
        verb: Verb,
        scope: &str,
        key: &RankKey,
        statuses: &BTreeMap<String, String>,
        fields: &BTreeMap<String, FieldDecl>,
    ) -> std::result::Result<(), String> {
        let desc = key.describe();
        let check_status = |status: &Option<Vec<String>>| {
            for s in status.iter().flatten() {
                if !statuses.contains_key(s) {
                    let valid: Vec<&str> = statuses.keys().map(String::as_str).collect();
                    return Err(format!(
                        "{scope}ranking key {desc} filters on status '{s}', which \
                         [vocabulary.status] does not declare; valid: {}",
                        valid.join(", ")
                    ));
                }
            }
            Ok(())
        };
        check_status(&key.status)?;
        for part in key.sum.iter().flatten() {
            check_status(&part.status)?;
        }
        match &key.of {
            Some(RankOf::Field(of)) => {
                let canonical = crate::record::RANKABLE_FIELDS.contains(&of.field.as_str());
                match fields.get(&of.field) {
                    _ if canonical => {}
                    None => {
                        return Err(format!(
                            "{scope}ranking key {desc} reads `of` field '{}', which is neither a \
                             field the ranking can read ({}) nor a custom field of [fields]",
                            of.field,
                            crate::record::RANKABLE_FIELDS.join(", ")
                        ));
                    }
                    Some(decl) => {
                        if let Some(ty) = &key.item_type {
                            if !decl.on.is_empty() && !decl.on.contains(ty) {
                                return Err(format!(
                                    "{scope}ranking key {desc} reads custom field '{}' on '{ty}' \
                                     neighbours, but the field applies only to {} — it would \
                                     read null on every neighbour",
                                    of.field,
                                    decl.on.join(", ")
                                ));
                            }
                        }
                    }
                }
            }
            Some(RankOf::Rank) => {
                if let Some(ty) = &key.item_type {
                    if self.rank_source(verb, ty).is_none() {
                        return Err(format!(
                            "{scope}ranking key {desc} reads the rank of '{ty}', which has no \
                             declared order — declare [{}.{ty}] or [{}.{ty}], or read a field \
                             with `of = {{ field = \"...\" }}`",
                            verb.section(),
                            verb.other().section(),
                        ));
                    }
                }
            }
            None => {}
        }
        Ok(())
    }

    /// The orders a per-type order `(verb, ty)` reads the rank of: for each `of = "rank"` key, the
    /// [`rank_source`](Self::rank_source) of its `type`, or of EVERY type with a declared order when
    /// the key names none (any neighbour may then be of any type, the order's own type included).
    pub(crate) fn rank_deps(&self, keys: &[RankKey], verb: Verb) -> BTreeSet<(Verb, String)> {
        let ranked: BTreeSet<&String> = [Verb::Next, Verb::List]
            .into_iter()
            .flat_map(|v| self.spec(v).by_type.keys())
            .collect();
        let mut deps = BTreeSet::new();
        for key in keys.iter().filter(|k| k.of == Some(RankOf::Rank)) {
            let targets: Vec<&String> = match &key.item_type {
                Some(ty) => vec![ty],
                None => ranked.iter().copied().collect(),
            };
            for ty in targets {
                if let Some(src) = self.rank_source(verb, ty) {
                    deps.insert((src, ty.clone()));
                }
            }
        }
        deps
    }

    /// Refuse a cycle in the type graph (spec §4) — depth-first over the per-type orders, naming
    /// the first cycle found as its chain of sections.
    fn check_type_chain(&self) -> std::result::Result<(), String> {
        let nodes: Vec<(Verb, String)> = [Verb::Next, Verb::List]
            .into_iter()
            .flat_map(|v| self.spec(v).by_type.keys().map(move |t| (v, t.clone())))
            .collect();
        let mut done: BTreeSet<(Verb, String)> = BTreeSet::new();
        for start in &nodes {
            let mut path: Vec<(Verb, String)> = Vec::new();
            self.visit(start.clone(), &mut path, &mut done)?;
        }
        Ok(())
    }

    fn visit(
        &self,
        node: (Verb, String),
        path: &mut Vec<(Verb, String)>,
        done: &mut BTreeSet<(Verb, String)>,
    ) -> std::result::Result<(), String> {
        if done.contains(&node) {
            return Ok(());
        }
        if let Some(at) = path.iter().position(|n| *n == node) {
            let name = |(v, t): &(Verb, String)| format!("[{}.{t}]", v.section());
            let mut chain: Vec<String> = path[at..].iter().map(name).collect();
            chain.push(name(&node));
            return Err(format!(
                "the ranking types rank by each other in a cycle: {} — a type may rank by the \
                 rank of another only along a chain. A `related ... of = \"rank\"` key without \
                 `type` reads the rank of every type with a declared order, its own included; \
                 name the neighbour's `type`",
                chain.join(" → ")
            ));
        }
        let keys = &self.spec(node.0).by_type[&node.1].order;
        let deps = self.rank_deps(keys, node.0);
        path.push(node.clone());
        for dep in deps {
            self.visit(dep, path, done)?;
        }
        path.pop();
        done.insert(node);
        Ok(())
    }
}

impl RankSpec {
    /// Every order of this policy with the scope its errors name: the default order (an empty
    /// scope), then each per-type order (`[section.<type>] `).
    pub(crate) fn orders(&self, section: &str) -> Vec<(String, &Vec<RankKey>)> {
        std::iter::once((String::new(), &self.order))
            .chain(
                self.by_type
                    .iter()
                    .map(|(t, o)| (format!("[{section}.{t}] "), &o.order)),
            )
            .collect()
    }

    /// Whether any order of this policy holds a derived key.
    pub fn has_derived(&self) -> bool {
        self.order
            .iter()
            .chain(self.by_type.values().flat_map(|o| o.order.iter()))
            .any(RankKey::is_derived)
    }
}

impl RankKey {
    /// Whether this is a derived key (`related`, `count` or `sum`) rather than a field key.
    pub fn is_derived(&self) -> bool {
        self.related.is_some() || self.count.is_some() || self.sum.is_some()
    }

    /// The key as its errors name it: `'<field>'` for a field key, its kind and edge for a derived
    /// one (`related = "parent"`, `count = "children"`, `sum`).
    pub(crate) fn describe(&self) -> String {
        if let Some(r) = &self.related {
            format!("`related = \"{r}\"`")
        } else if let Some(c) = &self.count {
            format!("`count = \"{c}\"`")
        } else if self.sum.is_some() {
            "`sum`".to_string()
        } else {
            format!("'{}'", self.field)
        }
    }

    /// The structural rules of one key (spec §7), checked by
    /// [`RankSpec::validate_in`](super::RankSpec) for every key of every order. `scope` prefixes
    /// the error (`[ranking.next.<type>] ` or empty for the default order); `default_next` says the
    /// key stands in the `[ranking.next]` default order.
    pub(crate) fn validate_kind(
        &self,
        scope: &str,
        default_next: bool,
    ) -> std::result::Result<(), String> {
        let kinds = [
            (!self.field.is_empty(), "field"),
            (self.related.is_some(), "related"),
            (self.count.is_some(), "count"),
            (self.sum.is_some(), "sum"),
        ];
        let set: Vec<&str> = kinds.iter().filter(|(on, _)| *on).map(|k| k.1).collect();
        let desc = self.describe();
        match set.len() {
            0 => {
                return Err(format!(
                    "{scope}a ranking key names no kind — give it `field`, `related`, `count` or \
                     `sum`"
                ))
            }
            1 => {}
            _ => {
                return Err(format!(
                    "{scope}ranking key {desc} mixes kinds ({}) — a key is exactly one of field, \
                     related, count or sum",
                    set.join(", ")
                ))
            }
        }
        let parts = [
            (self.item_type.is_some(), "type"),
            (self.of.is_some(), "of"),
            (self.pick.is_some(), "pick"),
            (self.status.is_some(), "status"),
            (self.label.is_some(), "label"),
        ];
        let stray = |allowed: &[&str]| {
            parts
                .iter()
                .find(|(on, name)| *on && !allowed.contains(name))
                .map(|p| p.1)
        };
        if !self.is_derived() {
            if let Some(part) = stray(&[]) {
                return Err(format!(
                    "{scope}ranking key {desc} is a field key, and `{part}` belongs to a derived \
                     key (related, count, sum)"
                ));
            }
            return Ok(());
        }
        if default_next {
            return Err(format!(
                "{scope}ranking key {desc} is a derived key in the [ranking.next] default \
                 `order`, which also orders `blocked` and the lanes' `--sort rank`; derived keys \
                 rank in a per-type order [ranking.next.<type>] (or under [ranking.list])"
            ));
        }
        if !self.precedence.is_empty() {
            return Err(format!(
                "{scope}ranking key {desc} sets a precedence list; on a derived key it belongs in \
                 `of = {{ field = \"...\", precedence = [...] }}`"
            ));
        }
        let never_null = |what: &str| {
            if self.nulls != Nulls::Last {
                Err(format!(
                    "{scope}ranking key {desc} sets `nulls`, but {what} is never null — drop it"
                ))
            } else {
                Ok(())
            }
        };
        if let Some(edge) = &self.related {
            if !RELATED_EDGES.contains(&edge.as_str()) {
                return Err(format!(
                    "{scope}ranking key {desc} follows no edge kind the ranking knows; valid: {}",
                    RELATED_EDGES.join(", ")
                ));
            }
            if let Some(part) = stray(&["type", "of", "pick"]) {
                return Err(format!(
                    "{scope}ranking key {desc} sets `{part}`, which a `related` key does not \
                     take (its parts: type, of, pick, dir, nulls)"
                ));
            }
            match &self.of {
                None => {
                    return Err(format!(
                        "{scope}ranking key {desc} does not say what to read on the neighbours — \
                         add `of = \"rank\"` or `of = {{ field = \"...\" }}`"
                    ))
                }
                Some(RankOf::Field(of)) if of.dir != Dir::Asc || of.nulls != Nulls::Last => {
                    return Err(format!(
                        "{scope}ranking key {desc} sets dir/nulls inside `of`; they belong to the \
                         `related` key itself — move them out of `of`"
                    ))
                }
                Some(_) => {}
            }
        } else if let Some(edge) = &self.count {
            Self::check_count_edge(scope, &desc, edge)?;
            if let Some(part) = stray(&["type", "status", "label"]) {
                return Err(format!(
                    "{scope}ranking key {desc} sets `{part}`, which a `count` key does not take \
                     (its parts: type, status, label, dir)"
                ));
            }
            Self::check_status_list(scope, &desc, &self.status)?;
            never_null("a count")?;
        } else if let Some(parts) = &self.sum {
            if parts.is_empty() {
                return Err(format!(
                    "{scope}ranking key {desc} adds no counts — list at least one `{{ count = ... }}`"
                ));
            }
            if let Some(part) = stray(&[]) {
                return Err(format!(
                    "{scope}ranking key {desc} sets `{part}`; a `sum` takes only its summands and \
                     `dir` — filters belong in each summand"
                ));
            }
            for p in parts {
                Self::check_count_edge(scope, &desc, &p.count)?;
                Self::check_status_list(scope, &desc, &p.status)?;
            }
            never_null("a sum")?;
        }
        Ok(())
    }

    fn check_count_edge(scope: &str, desc: &str, edge: &str) -> std::result::Result<(), String> {
        if COUNT_EDGES.contains(&edge) {
            Ok(())
        } else {
            Err(format!(
                "{scope}ranking key {desc} counts '{edge}', which is no edge kind or direction \
                 the ranking knows; valid: {}",
                COUNT_EDGES.join(", ")
            ))
        }
    }

    fn check_status_list(
        scope: &str,
        desc: &str,
        status: &Option<Vec<String>>,
    ) -> std::result::Result<(), String> {
        if status.as_ref().is_some_and(Vec::is_empty) {
            return Err(format!(
                "{scope}ranking key {desc} filters on an empty status list, which counts nothing \
                 — list the statuses, or drop `status`"
            ));
        }
        Ok(())
    }

    /// The item types this key names (its own `type` and each summand's), for the check against
    /// `[types].list`.
    pub(crate) fn named_types(&self) -> Vec<&str> {
        self.item_type
            .iter()
            .map(String::as_str)
            .chain(
                self.sum
                    .iter()
                    .flatten()
                    .filter_map(|p| p.item_type.as_deref()),
            )
            .collect()
    }
}
