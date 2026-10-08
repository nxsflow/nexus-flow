//! A described change as one conditional write (6j6v.vvw6 point 1, the DynamoDB half).
//!
//! [`plan`] turns a [`Change`] into a [`Write`]: the row's sort key, the attributes to set, and the
//! condition under which the write takes effect. A write whose condition fails changed nothing and
//! is not an error — it is a register that did not beat the stored version, or a row that already
//! existed. Every effect stays commutative and idempotent, as it is in the SQLite applier:
//!
//! | effect     | SQLite applier                                  | write                                                   |
//! |------------|-------------------------------------------------|---------------------------------------------------------|
//! | `Ensure`   | `INSERT OR IGNORE`                              | set key + cells, if the row is absent                   |
//! | `Put`      | upsert, every cell replaced                     | set key + cells, unconditionally                        |
//! | `Register` | upsert `WHERE` unwritten `OR` theirs beats ours | set key + cells + version, if the row is absent, the version unwritten, or theirs beats ours |
//!
//! "Unwritten" is decided as SQLite decides it: a version column a row never wrote holds the
//! column's `DEFAULT`. Where that default is NULL, any version wins; where it is a number (every
//! `_v INTEGER DEFAULT 0` register), the version must beat the default — so an op numbered `(0, 0)`
//! or below loses against a fresh register here exactly as it does locally. A NULL cell is removed
//! where the column defaults to NULL — the same reading — and stored as DynamoDB's NULL only where
//! it does not, because there a removed attribute would read as the default.

use crate::layout::{row_key, Layout, LayoutError, RESERVED, SK};
use nxs_foundation::change::{Cell, Change, Effect, Wins};

/// One write to one row.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Write {
    /// The row's sort key within its stream's partition.
    pub sk: String,
    /// Attributes to set. The sort key is never among them.
    pub set: Vec<(String, Cell)>,
    /// Attributes to remove.
    pub remove: Vec<String>,
    /// When the write takes effect; `None` is always.
    pub condition: Option<Cond>,
}

/// A write's condition, over the row as stored.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Cond {
    /// No such row.
    RowAbsent,
    /// The row exists.
    RowPresent,
    /// The row has no such attribute.
    Absent(String),
    /// The stored version — every column present — loses to `theirs` under `wins`: lower under
    /// [`Wins::Higher`], higher under [`Wins::Lower`], compared column by column.
    Beats {
        columns: Vec<String>,
        theirs: Vec<i64>,
        wins: Wins,
    },
    /// At least one holds.
    Any(Vec<Cond>),
}

impl Write {
    /// An unconditional write of `set`.
    pub fn put(sk: String, set: Vec<(String, Cell)>) -> Write {
        Write {
            sk,
            set,
            remove: Vec::new(),
            condition: None,
        }
    }
}

/// What one attribute weighs toward the item limit, as DynamoDB counts it: its name and its value
/// — a number at its widest, 21 bytes.
pub fn attribute_bytes(name: &str, cell: &Cell) -> usize {
    name.len()
        + match cell {
            Cell::Null => 1,
            Cell::Text(v) => v.len(),
            Cell::Int(_) => 21,
        }
}

impl Write {
    /// What this write alone puts into its row, the sort key included.
    pub fn bytes(&self) -> usize {
        crate::layout::SK.len()
            + self.sk.len()
            + self
                .set
                .iter()
                .map(|(n, c)| attribute_bytes(n, c))
                .sum::<usize>()
    }
}

/// The write that carries `change` out.
pub fn plan(layout: &Layout, change: &Change) -> Result<Write, LayoutError> {
    let table = layout.table(change.table)?;
    let (cells, version, wins) = match &change.effect {
        Effect::Ensure { cells } | Effect::Put { cells } => (cells, &[][..], None),
        Effect::Register {
            cells,
            version,
            wins,
        } => (cells, &version[..], Some(*wins)),
    };
    let mut set: Vec<(String, Cell)> = Vec::new();
    for (column, cell) in change.key.iter().chain(cells) {
        set.push((column.clone(), cell.clone()));
    }
    for (column, v) in version {
        set.push((column.clone(), Cell::Int(*v)));
    }
    for (column, _) in &set {
        if RESERVED.contains(&column.as_str()) {
            return Err(LayoutError::ReservedColumn {
                table: change.table.to_string(),
                column: column.clone(),
            });
        }
        if !table.columns.contains(column) {
            return Err(LayoutError::UnknownColumn {
                table: change.table.to_string(),
                column: column.clone(),
            });
        }
    }
    let condition = match &change.effect {
        Effect::Ensure { .. } => Some(Cond::RowAbsent),
        Effect::Put { .. } => None,
        Effect::Register { .. } => {
            let wins = wins.expect("a register has a rule");
            let theirs: Vec<i64> = version.iter().map(|(_, v)| *v).collect();
            let columns: Vec<String> = version.iter().map(|(c, _)| c.clone()).collect();
            let defaults: Vec<Cell> = columns.iter().map(|c| table.default_of(c)).collect();
            // SQLite: `WHERE <any version column> IS NULL OR (theirs) beats (stored)`, with a column
            // the row never wrote holding its DEFAULT.
            let unwritten_wins = defaults.iter().any(|d| !matches!(d, Cell::Int(_)))
                || beats(
                    &theirs,
                    &defaults
                        .iter()
                        .map(|d| match d {
                            Cell::Int(v) => *v,
                            _ => unreachable!("checked above"),
                        })
                        .collect::<Vec<_>>(),
                    wins,
                );
            let mut any = vec![Cond::RowAbsent];
            if unwritten_wins {
                any.extend(columns.iter().map(|c| Cond::Absent(c.clone())));
            }
            any.push(Cond::Beats {
                columns,
                theirs,
                wins,
            });
            Some(Cond::Any(any))
        }
    };
    let (set, remove) = null_as_absent(table, set);
    Ok(Write {
        sk: row_key(change.table, &change.key)?,
        set,
        remove,
        condition,
    })
}

/// Split `cells` into what to set and what to remove: a NULL cell whose column defaults to NULL is
/// removed, which reads the same. So DynamoDB's NULL is stored only where the default is not NULL —
/// and never on a register's version, whose "unwritten" test is the attribute's absence.
pub(crate) fn null_as_absent(
    table: &crate::layout::TableLayout,
    cells: Vec<(String, Cell)>,
) -> (Vec<(String, Cell)>, Vec<String>) {
    let mut set = Vec::new();
    let mut remove = Vec::new();
    for (column, cell) in cells {
        if cell == Cell::Null && table.default_of(&column) == Cell::Null {
            remove.push(column);
        } else {
            set.push((column, cell));
        }
    }
    (set, remove)
}

/// Whether `theirs` beats `stored` under `wins`.
pub(crate) fn beats(theirs: &[i64], stored: &[i64], wins: Wins) -> bool {
    match wins {
        Wins::Higher => theirs > stored,
        Wins::Lower => theirs < stored,
    }
}

/// The DynamoDB spelling of a write: an update expression, a condition expression, and the names
/// and values they refer to by placeholder.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Expression {
    pub update: String,
    pub condition: Option<String>,
    pub names: Vec<(String, String)>,
    pub values: Vec<(String, Cell)>,
}

impl Write {
    /// The update and condition expressions of this write. Every attribute goes by a `#n<i>` name
    /// placeholder, so a column named like a DynamoDB reserved word (`status`, `type`, `name`) is
    /// written as any other.
    pub fn expression(&self) -> Expression {
        let mut names: Vec<(String, String)> = Vec::new();
        let mut values: Vec<(String, Cell)> = Vec::new();
        let mut update = String::new();
        if !self.set.is_empty() {
            let parts: Vec<String> = self
                .set
                .iter()
                .map(|(a, c)| {
                    let n = name_of(a, &mut names);
                    format!("{n} = {}", value_of(c.clone(), &mut values))
                })
                .collect();
            update = format!("SET {}", parts.join(", "));
        }
        if !self.remove.is_empty() {
            let parts: Vec<String> = self.remove.iter().map(|a| name_of(a, &mut names)).collect();
            if !update.is_empty() {
                update.push(' ');
            }
            update.push_str(&format!("REMOVE {}", parts.join(", ")));
        }
        let condition = self.condition.as_ref().map(|c| {
            render(c, &mut |a| name_of(a, &mut names), &mut |v| {
                value_of(v, &mut values)
            })
        });
        Expression {
            update,
            condition,
            names,
            values,
        }
    }
}

/// The placeholder of attribute `attr`, the same one each time it is named.
fn name_of(attr: &str, names: &mut Vec<(String, String)>) -> String {
    if let Some((p, _)) = names.iter().find(|(_, a)| a == attr) {
        return p.clone();
    }
    let p = format!("#n{}", names.len());
    names.push((p.clone(), attr.to_string()));
    p
}

/// A fresh placeholder for `cell`.
fn value_of(cell: Cell, values: &mut Vec<(String, Cell)>) -> String {
    let p = format!(":v{}", values.len());
    values.push((p.clone(), cell));
    p
}

fn render(
    cond: &Cond,
    name: &mut dyn FnMut(&str) -> String,
    value: &mut dyn FnMut(Cell) -> String,
) -> String {
    match cond {
        Cond::RowAbsent => format!("attribute_not_exists({})", name(SK)),
        Cond::RowPresent => format!("attribute_exists({})", name(SK)),
        Cond::Absent(a) => format!("attribute_not_exists({})", name(a)),
        Cond::Beats {
            columns,
            theirs,
            wins,
        } => {
            // Stored loses to theirs: stored < theirs under Higher, stored > theirs under Lower,
            // lexicographically — built from the last column outwards.
            let op = match wins {
                Wins::Higher => "<",
                Wins::Lower => ">",
            };
            let mut expr: Option<String> = None;
            for (c, v) in columns.iter().zip(theirs).rev() {
                let n = name(c);
                let lose = format!("{n} {op} {}", value(Cell::Int(*v)));
                expr = Some(match expr {
                    None => lose,
                    Some(rest) => {
                        format!("({lose} OR ({n} = {} AND {rest}))", value(Cell::Int(*v)))
                    }
                });
            }
            // A register always has a version; one without could beat nothing.
            expr.unwrap_or_else(|| {
                let sk = name(SK);
                format!("(attribute_exists({sk}) AND attribute_not_exists({sk}))")
            })
        }
        Cond::Any(parts) => {
            let parts: Vec<String> = parts.iter().map(|p| render(p, name, value)).collect();
            format!("({})", parts.join(" OR "))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use nxs_foundation::change::{cells, Version};

    fn layout() -> Layout {
        let conn = rusqlite::Connection::open_in_memory().unwrap();
        conn.execute_batch(
            "CREATE TABLE reg(id TEXT PRIMARY KEY, v TEXT, v_l INTEGER DEFAULT 0, v_s INTEGER DEFAULT 0,
                              first TEXT, first_l INTEGER, first_s INTEGER);",
        )
        .unwrap();
        Layout::from_sqlite(&conn, &["reg"]).unwrap()
    }

    fn register(version: Version, wins: Wins) -> Change {
        let (cell, columns) = match wins {
            Wins::Higher => ("v", ("v_l", "v_s")),
            Wins::Lower => ("first", ("first_l", "first_s")),
        };
        Change::register(
            "reg",
            cells([("id", "a".into())]),
            cells([(cell, "x".into())]),
            columns,
            version,
            wins,
        )
    }

    #[test]
    fn an_unwritten_register_with_a_numeric_default_must_beat_it() {
        let at = |lamport, site| Version { lamport, site };
        // DEFAULT 0: (1, 0) beats the default, so a row without the version may take it…
        let w = plan(&layout(), &register(at(1, 0), Wins::Higher)).unwrap();
        let Some(Cond::Any(any)) = &w.condition else {
            panic!()
        };
        assert!(any.contains(&Cond::Absent("v_l".into())));
        // …but (0, 0) does not, exactly as SQLite's `(0, 0) > (0, 0)` is false.
        let w = plan(&layout(), &register(at(0, 0), Wins::Higher)).unwrap();
        let Some(Cond::Any(any)) = &w.condition else {
            panic!()
        };
        assert!(!any.iter().any(|c| matches!(c, Cond::Absent(_))));
        assert!(any.contains(&Cond::RowAbsent));
        // A NULL default is unwritten: any version takes it, under either rule.
        let w = plan(&layout(), &register(at(-5, 0), Wins::Lower)).unwrap();
        let Some(Cond::Any(any)) = &w.condition else {
            panic!()
        };
        assert!(any.contains(&Cond::Absent("first_l".into())));
    }

    #[test]
    fn a_column_the_table_does_not_have_or_reserves_is_refused() {
        let bad = Change::put(
            "reg",
            cells([("id", "a".into())]),
            cells([("nope", "x".into())]),
        );
        assert!(matches!(
            plan(&layout(), &bad),
            Err(LayoutError::UnknownColumn { .. })
        ));
        let bad = Change::put("missing", cells([("id", "a".into())]), Vec::new());
        assert!(matches!(
            plan(&layout(), &bad),
            Err(LayoutError::UnknownTable(_))
        ));
    }

    #[test]
    fn the_expression_names_every_attribute_by_placeholder_and_compares_lexicographically() {
        let w = plan(
            &layout(),
            &register(
                Version {
                    lamport: 7,
                    site: 3,
                },
                Wins::Higher,
            ),
        )
        .unwrap();
        let e = w.expression();
        assert_eq!(e.update, "SET #n0 = :v0, #n1 = :v1, #n2 = :v2, #n3 = :v3");
        assert_eq!(
            e.condition.as_deref(),
            Some(
                "(attribute_not_exists(#n4) OR attribute_not_exists(#n2) OR attribute_not_exists(#n3) \
                 OR (#n2 < :v5 OR (#n2 = :v6 AND #n3 < :v4)))"
            )
        );
        assert_eq!(
            e.names,
            ["id", "v", "v_l", "v_s", "sk"]
                .iter()
                .enumerate()
                .map(|(i, a)| (format!("#n{i}"), a.to_string()))
                .collect::<Vec<_>>()
        );
        assert_eq!(e.values[4], (":v4".into(), Cell::Int(3)));
        assert_eq!(e.values[5], (":v5".into(), Cell::Int(7)));
    }
}
