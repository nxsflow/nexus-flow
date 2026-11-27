//! A decision's handle (RFC 0003, "Handles"): `YYYY-MM-DD-<ticket>-<slug>`, the name a file in
//! `factory/decisions/` has today without `.md`. `<ticket>` is the triggering ticket's label, or
//! `no-ticket`; `<slug>` is the title in lowercase words joined by `-`, cut at 60 characters on a
//! word boundary, unless the writer gives a slug of their own.
//!
//! A word is a run of ASCII letters and digits; every other character separates words, so
//! `write-once` is two words and a letter outside ASCII is left out, as a file name today has it.

/// The longest slug made from a title, in characters.
pub const SLUG_MAX: usize = 60;

/// The slug of `title`: its words in lowercase, joined by `-`, as many as fit in 60 characters.
/// A first word longer than that is cut inside it. `None` when the title has no word.
pub fn slug(title: &str) -> Option<String> {
    let mut slug = String::new();
    for word in title
        .split(|c: char| !c.is_ascii_alphanumeric())
        .filter(|word| !word.is_empty())
    {
        let word = word.to_ascii_lowercase();
        if slug.is_empty() {
            slug = word.chars().take(SLUG_MAX).collect();
        } else if slug.len() + 1 + word.len() <= SLUG_MAX {
            slug.push('-');
            slug.push_str(&word);
        } else {
            break;
        }
    }
    (!slug.is_empty()).then_some(slug)
}

/// Whether `text` is a slug: lowercase letters and digits in words joined by single `-`.
pub fn is_slug(text: &str) -> bool {
    !text.is_empty()
        && text.split('-').all(|word| {
            !word.is_empty()
                && word
                    .bytes()
                    .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit())
        })
}

/// The handle of a decision made on `date` with the triggering ticket `ticket` and the slug
/// `slug`.
pub fn handle(date: &str, ticket: Option<&str>, slug: &str) -> String {
    format!("{date}-{}-{slug}", ticket.unwrap_or("no-ticket"))
}
