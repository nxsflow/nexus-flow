//! Every collection of the model follows RFC 0001's naming rule ("The model", **Each module owns
//! its collections.**): it matches `^[a-z][A-Za-z0-9]{0,62}$`, and it is its module's prefix
//! followed by the collection's own name in upper camel case.

use nexus_flow_store::model::{self, Module};

/// The names of the collections the committed lock declares: the keys of its top-level
/// `collections` object.
fn lock_collections() -> Vec<String> {
    let mut json = Json::new(model::LOCK);
    let mut found = None;
    json.object(|json, key| {
        if key == "collections" {
            let mut names = Vec::new();
            json.object(|json, name| {
                names.push(name.to_owned());
                json.skip();
            });
            found = Some(names);
        } else {
            json.skip();
        }
    });
    found.expect("the lock has a collections object")
}

#[test]
fn the_table_names_every_collection_of_the_lock_once() {
    let mut table: Vec<&str> = model::COLLECTIONS.iter().map(|c| c.0).collect();
    table.sort_unstable();
    let before = table.len();
    table.dedup();
    assert_eq!(table.len(), before, "a collection is declared twice");
    let mut lock = lock_collections();
    lock.sort_unstable();
    assert_eq!(table, lock, "model::COLLECTIONS and model.lock.json differ");
}

#[test]
fn every_collection_follows_the_naming_rule_of_its_module() {
    assert!(!model::COLLECTIONS.is_empty());
    for &(name, module) in model::COLLECTIONS {
        assert!(
            model::is_collection_name(name, module),
            "{name} breaks the rule for module {module:?}"
        );
    }
}

#[test]
fn the_prefix_table_follows_rfc_0001() {
    let table: Vec<(Module, &str)> = Module::ALL.iter().map(|&m| (m, m.prefix())).collect();
    assert_eq!(
        table,
        [
            (Module::Space, "space"),
            (Module::Tickets, "tickets"),
            (Module::Decisions, "decisions"),
            (Module::Goals, "goals"),
            (Module::Requests, "requests"),
            (Module::Status, "status"),
        ]
    );
}

#[test]
fn a_name_is_its_module_prefix_and_an_upper_camel_case_name() {
    for (name, module) in [
        ("spaceMember", Module::Space),
        ("ticketsTicket", Module::Tickets),
        ("decisionsDecision", Module::Decisions),
        ("goalsGoal", Module::Goals),
        ("requestsRequest", Module::Requests),
        ("statusNote", Module::Status),
        ("ticketsTicketLink2", Module::Tickets),
    ] {
        assert!(model::is_collection_name(name, module), "{name}");
    }
    for (name, module, why) in [
        ("space", Module::Space, "the prefix alone"),
        ("spacemember", Module::Space, "the own name not upper camel case"),
        ("ticketsTicket", Module::Space, "another module's prefix"),
        ("space_Member", Module::Space, "a separator"),
        ("spaceMember-2", Module::Space, "a character outside [A-Za-z0-9]"),
        ("SpaceMember", Module::Space, "an upper-case first letter"),
        ("spaceMÜmber", Module::Space, "a non-ASCII letter"),
    ] {
        assert!(!model::is_collection_name(name, module), "{name}: {why}");
    }
    let longest = format!("space{}", "M".repeat(58));
    assert_eq!(longest.len(), 63);
    assert!(model::is_collection_name(&longest, Module::Space));
    let too_long = format!("space{}", "M".repeat(59));
    assert!(!model::is_collection_name(&too_long, Module::Space), "64 characters");
}

/// Just enough of a JSON reader to list an object's keys and skip their values: the lock is
/// JSON that `ncdb` writes, and the store has no JSON dependency.
struct Json<'a> {
    text: &'a [u8],
    at: usize,
}

impl<'a> Json<'a> {
    fn new(text: &'a str) -> Json<'a> {
        Json {
            text: text.as_bytes(),
            at: 0,
        }
    }

    fn space(&mut self) {
        while self.text[self.at].is_ascii_whitespace() {
            self.at += 1;
        }
    }

    fn expect(&mut self, byte: u8) {
        self.space();
        assert_eq!(self.text[self.at] as char, byte as char, "at byte {}", self.at);
        self.at += 1;
    }

    fn peek(&mut self) -> u8 {
        self.space();
        self.text[self.at]
    }

    /// Reads a string and returns its raw text; the lock's keys hold no escapes.
    fn string(&mut self) -> &'a str {
        self.expect(b'"');
        let start = self.at;
        while self.text[self.at] != b'"' {
            self.at += if self.text[self.at] == b'\\' { 2 } else { 1 };
        }
        self.at += 1;
        std::str::from_utf8(&self.text[start..self.at - 1]).unwrap()
    }

    /// Reads an object, calling `each` with every key; `each` must read or skip the value.
    fn object(&mut self, mut each: impl FnMut(&mut Json<'a>, &'a str)) {
        self.expect(b'{');
        if self.peek() == b'}' {
            self.at += 1;
            return;
        }
        loop {
            let key = self.string();
            self.expect(b':');
            each(self, key);
            match self.peek() {
                b',' => self.at += 1,
                _ => return self.expect(b'}'),
            }
        }
    }

    fn skip(&mut self) {
        match self.peek() {
            b'"' => {
                self.string();
            }
            b'{' => self.object(|json, _| json.skip()),
            b'[' => {
                self.at += 1;
                if self.peek() == b']' {
                    self.at += 1;
                    return;
                }
                loop {
                    self.skip();
                    match self.peek() {
                        b',' => self.at += 1,
                        _ => return self.expect(b']'),
                    }
                }
            }
            _ => {
                while !matches!(self.text[self.at], b',' | b'}' | b']') {
                    self.at += 1;
                }
            }
        }
    }
}
