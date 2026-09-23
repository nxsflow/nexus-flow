//! The compile-time plugin self-registration registry (r16h.1). Each plugin — the OSS bundled
//! `issue-tracker`/`personal-todo` here, and any consumer's proprietary plugin from ITS OWN crate —
//! `inventory::submit!`s ONE [`PluginRegistration`] carrying its name + embedded TOML. `plugin::load`
//! / `plugin::plugin_names` / `plugin::available` read this roster exclusively, so a consumer plugin
//! appears purely by registering + linking — no OSS source change, and its TOML never lands in this
//! repo or on disk. Mirrors the module roster (`nxs_init::module`), one level down.

/// One plugin's self-registered descriptor.
pub struct PluginRegistration {
    /// The plugin name — matches `[flow] plugin = "..."` and the TOML's own `name`.
    pub name: &'static str,
    /// The embedded TOML source (the consumer's `include_str!`, so it rides the consumer binary).
    pub toml: &'static str,
    /// Deterministic listing ordinal (declared — never link-order or alphabetical accident).
    pub order: u16,
}

inventory::collect!(PluginRegistration);

// The two plugins this OSS repo ships. A consumer's proprietary plugin submits the same descriptor
// from its own crate (its TOML via its own `include_str!`), never here.
inventory::submit!(PluginRegistration {
    name: "issue-tracker",
    toml: include_str!("issue-tracker.toml"),
    order: 10,
});
inventory::submit!(PluginRegistration {
    name: "personal-todo",
    toml: include_str!("personal-todo.toml"),
    order: 20,
});

/// Every registered plugin, sorted into canonical `(order, name)` order (link order is unspecified).
pub fn registered() -> Vec<&'static PluginRegistration> {
    sort_registry(inventory::iter::<PluginRegistration>.into_iter().collect())
}

/// Sort by declared `order`, then `name` — pure, so determinism is testable without `inventory`.
pub fn sort_registry(mut regs: Vec<&PluginRegistration>) -> Vec<&PluginRegistration> {
    regs.sort_by(|a, b| a.order.cmp(&b.order).then_with(|| a.name.cmp(b.name)));
    regs
}

/// The registration for `name`: `Ok(Some)` for exactly one match, `Ok(None)` for none, and `Err`
/// for a DUPLICATE — a name registered more than once is ambiguous (which plugin's TOML wins would
/// be link-order-dependent), directly violating the "byte-stable regardless of link order" goal, so
/// it is a loud error rather than a silent first-wins pick.
pub fn find(name: &str) -> Result<Option<&'static PluginRegistration>, String> {
    resolve(&registered(), name)
}

/// Resolve `name` within a registry slice (the pure core of [`find`], so the duplicate case is
/// testable without `inventory`): exactly one match ⇒ `Ok(Some)`, none ⇒ `Ok(None)`, two or more ⇒
/// `Err` naming the collision. A duplicate can only arise when a consumer plugin registers a name
/// that collides with another registered plugin — a build/packaging bug, surfaced loudly.
pub fn resolve<'a>(
    regs: &[&'a PluginRegistration],
    name: &str,
) -> Result<Option<&'a PluginRegistration>, String> {
    let mut hits = regs.iter().copied().filter(|r| r.name == name);
    let first = hits.next();
    if hits.next().is_some() {
        return Err(format!(
            "plugin name '{name}' is registered more than once — ambiguous: a consumer plugin \
             collides with another registered plugin, and which one wins would depend on link order"
        ));
    }
    Ok(first)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn reg(name: &'static str, order: u16) -> PluginRegistration {
        PluginRegistration {
            name,
            toml: "",
            order,
        }
    }

    #[test]
    fn sort_registry_orders_by_declared_ordinal_then_name() {
        let a = reg("issue-tracker", 10);
        let b = reg("personal-todo", 20);
        let c = reg("aaa", 20); // ties on order 20 → name breaks it (aaa before personal-todo)
        let sorted = sort_registry(vec![&b, &c, &a]);
        assert_eq!(
            sorted.iter().map(|r| r.name).collect::<Vec<_>>(),
            vec!["issue-tracker", "aaa", "personal-todo"]
        );
    }

    #[test]
    fn registered_collects_the_two_oss_plugins_in_order() {
        // This runs in the facade lib test binary, which links ONLY the OSS submissions — exactly two.
        let names: Vec<&str> = registered().iter().map(|r| r.name).collect();
        assert_eq!(names, vec!["issue-tracker", "personal-todo"]);
    }

    #[test]
    fn resolve_is_ok_for_a_unique_name_and_none_for_an_absent_one() {
        let a = reg("issue-tracker", 10);
        let b = reg("personal-todo", 20);
        let regs = vec![&a, &b];
        assert_eq!(
            resolve(&regs, "issue-tracker").unwrap().unwrap().name,
            "issue-tracker"
        );
        assert!(resolve(&regs, "absent").unwrap().is_none());
    }

    #[test]
    fn resolve_rejects_a_duplicate_name_regardless_of_order() {
        // The Critical (review): a name registered twice is ambiguous — first-wins would be
        // link-order-dependent. resolve() rejects it loudly, whether the ordinals differ or tie.
        let a = reg("dup", 10);
        let b = reg("dup", 20); // same name, different order
        assert!(resolve(&[&a, &b], "dup").is_err());
        let c = reg("dup", 10); // same name, same order (the pure link-order case)
        assert!(resolve(&[&a, &c], "dup").is_err());
        // A collision does not poison an unrelated, unique name.
        let d = reg("unique", 30);
        assert_eq!(
            resolve(&[&a, &b, &d], "unique").unwrap().unwrap().name,
            "unique"
        );
    }
}
