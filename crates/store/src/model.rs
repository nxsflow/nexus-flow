//! nexus-flow's NoCentralDB model (RFC 0001, "The model"). The source is `model/model.ts` at the
//! repository root; `ncdb model build` writes `model/model.lock.json` from it, and this crate
//! reads only that committed lock, compiled in, so building `nxs` needs neither Node nor `ncdb`.

/// The text of the committed `model/model.lock.json`, which every member database opens with.
pub const LOCK: &str = include_str!("../../../model/model.lock.json");

/// The module that owns a collection (RFC 0001, "The model", **Each module owns its
/// collections.**). The frame's own collections belong to [`Module::Space`].
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Module {
    /// The frame itself: members and their role records.
    Space,
    /// RFC 0002.
    Tickets,
    /// RFC 0003.
    Decisions,
    /// RFC 0005.
    Goals,
    /// RFC 0009.
    Requests,
    /// RFC 0007.
    Status,
    /// RFC 0008: the move from v1.
    Migration,
}

impl Module {
    /// Every module, in the order of the prefix table.
    pub const ALL: [Module; 7] = [
        Module::Space,
        Module::Tickets,
        Module::Decisions,
        Module::Goals,
        Module::Requests,
        Module::Status,
        Module::Migration,
    ];

    /// The prefix of the module's collection names: the module's name in lower case.
    pub fn prefix(self) -> &'static str {
        match self {
            Module::Space => "space",
            Module::Tickets => "tickets",
            Module::Decisions => "decisions",
            Module::Goals => "goals",
            Module::Requests => "requests",
            Module::Status => "status",
            Module::Migration => "migration",
        }
    }
}

/// Every collection the model declares, with the module that owns it. A test keeps it equal to
/// the collections of [`LOCK`].
pub const COLLECTIONS: &[(&str, Module)] = &[
    ("spaceMember", Module::Space),
    ("ticketsTicket", Module::Tickets),
    ("ticketsNote", Module::Tickets),
    ("ticketsWork", Module::Tickets),
    ("ticketsReference", Module::Tickets),
    ("decisionsDecision", Module::Decisions),
    ("decisionsTopic", Module::Decisions),
    ("decisionsSubject", Module::Decisions),
    ("decisionsSupersession", Module::Decisions),
    ("goalsGoal", Module::Goals),
    ("migrationRun", Module::Migration),
    ("migrationRecord", Module::Migration),
];

/// Whether `name` is a collection name of `module`: it matches `^[a-z][A-Za-z0-9]{0,62}$`
/// (NoCentralDB RFC 0005), and it is the module's prefix followed by the collection's own name in
/// upper camel case.
pub fn is_collection_name(name: &str, module: Module) -> bool {
    let Some(own) = name.strip_prefix(module.prefix()) else {
        return false;
    };
    name.len() <= 63
        && name.bytes().all(|b| b.is_ascii_alphanumeric())
        && own.bytes().next().is_some_and(|b| b.is_ascii_uppercase())
}
