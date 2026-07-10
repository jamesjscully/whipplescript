//! The provenance-native selection algebra (vw note §7.3; untie
//! readiness tracker Phase 2): a revset-shaped composable expression
//! language over recorded change-units, feeding the three selective
//! verbs (`undo <selection>`, `transport <selection>`, `adopt --only`)
//! and the archaeology queries.
//!
//! Grammar (union `|` loosest, then difference `~`, then intersection
//! `&`, parens group):
//!
//! ```text
//! expr   := diff ( '|' diff )*
//! diff   := inter ( '~' inter )*
//! inter  := prim ( '&' prim )*
//! prim   := atom | '(' expr ')'
//! atom   := path(<glob>) | by-effect(<prefix>) | by-origin(<prefix>)
//!         | in-branch(<id>) | change(<id>) | cut(<id>)
//!         | since(<stamp>) | until(<stamp>) | dependents-of(expr)
//! ```
//!
//! The unit of selection is one recorded write: (cut, path,
//! before → after), derived from cut lineage. `dependents-of` is the
//! slicer seam's conservative floor: path-level dependence (a later
//! unit on the same path consumed the earlier one's output). The
//! declaration/slice-granularity atoms (`decl(...)`, `slice-of(...)`)
//! arrive when the slicer joins as this algebra's client — the grammar
//! is closed under adding atoms.

use std::collections::BTreeSet;

use serde::{Deserialize, Serialize};

/// One recorded change-unit: what one cut did to one path.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct ChangeUnit {
    /// Position in the branch's cut order, oldest first — the
    /// dependence direction.
    pub seq: usize,
    pub cut_id: String,
    pub change_id: String,
    pub branch_id: String,
    pub path: String,
    pub before: Option<String>,
    pub after: Option<String>,
    pub origin: Option<String>,
    pub recorded_at: String,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum SelExpr {
    Union(Box<SelExpr>, Box<SelExpr>),
    Intersect(Box<SelExpr>, Box<SelExpr>),
    Difference(Box<SelExpr>, Box<SelExpr>),
    Atom(SelAtom),
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum SelAtom {
    Path(String),
    ByEffect(String),
    ByOrigin(String),
    InBranch(String),
    Change(String),
    Cut(String),
    Since(String),
    Until(String),
    DependentsOf(Box<SelExpr>),
}

/// Parse a selection expression. Errors carry the offending position's
/// remainder — enough for a CLI message.
pub fn parse(input: &str) -> Result<SelExpr, String> {
    let mut parser = Parser { rest: input.trim() };
    let expr = parser.expr()?;
    if !parser.rest.is_empty() {
        return Err(format!("unexpected trailing input: `{}`", parser.rest));
    }
    Ok(expr)
}

struct Parser<'a> {
    rest: &'a str,
}

impl<'a> Parser<'a> {
    fn skip_ws(&mut self) {
        self.rest = self.rest.trim_start();
    }

    fn eat(&mut self, token: char) -> bool {
        self.skip_ws();
        if let Some(stripped) = self.rest.strip_prefix(token) {
            self.rest = stripped;
            true
        } else {
            false
        }
    }

    fn expr(&mut self) -> Result<SelExpr, String> {
        let mut left = self.diff()?;
        while self.eat('|') {
            let right = self.diff()?;
            left = SelExpr::Union(Box::new(left), Box::new(right));
        }
        Ok(left)
    }

    fn diff(&mut self) -> Result<SelExpr, String> {
        let mut left = self.inter()?;
        while self.eat('~') {
            let right = self.inter()?;
            left = SelExpr::Difference(Box::new(left), Box::new(right));
        }
        Ok(left)
    }

    fn inter(&mut self) -> Result<SelExpr, String> {
        let mut left = self.prim()?;
        while self.eat('&') {
            let right = self.prim()?;
            left = SelExpr::Intersect(Box::new(left), Box::new(right));
        }
        Ok(left)
    }

    fn prim(&mut self) -> Result<SelExpr, String> {
        self.skip_ws();
        if self.eat('(') {
            let inner = self.expr()?;
            if !self.eat(')') {
                return Err(format!("expected `)` at `{}`", self.rest));
            }
            return Ok(inner);
        }
        let name_len = self
            .rest
            .find(|c: char| !(c.is_ascii_alphanumeric() || c == '-' || c == '_'))
            .unwrap_or(self.rest.len());
        let name = &self.rest[..name_len];
        if name.is_empty() {
            return Err(format!("expected a selection atom at `{}`", self.rest));
        }
        self.rest = &self.rest[name_len..];
        if !self.eat('(') {
            return Err(format!("expected `(` after `{name}`"));
        }
        if name == "dependents-of" {
            let inner = self.expr()?;
            if !self.eat(')') {
                return Err(format!("expected `)` at `{}`", self.rest));
            }
            return Ok(SelExpr::Atom(SelAtom::DependentsOf(Box::new(inner))));
        }
        // A plain-argument atom: the argument runs to the matching `)`.
        let close = self
            .rest
            .find(')')
            .ok_or_else(|| format!("unterminated `{name}(`"))?;
        let arg = self.rest[..close].trim().to_owned();
        self.rest = &self.rest[close + 1..];
        let atom = match name {
            "path" => SelAtom::Path(arg),
            "by-effect" => SelAtom::ByEffect(arg),
            "by-origin" => SelAtom::ByOrigin(arg),
            "in-branch" => SelAtom::InBranch(arg),
            "change" => SelAtom::Change(arg),
            "cut" => SelAtom::Cut(arg),
            "since" => SelAtom::Since(arg),
            "until" => SelAtom::Until(arg),
            other => return Err(format!("unknown selection atom `{other}`")),
        };
        Ok(SelExpr::Atom(atom))
    }
}

/// A `*`/`?` glob match (segments are not special: `*` crosses `/`,
/// matching the whole-path selection intent).
pub fn glob_matches(pattern: &str, value: &str) -> bool {
    fn inner(pattern: &[u8], value: &[u8]) -> bool {
        match (pattern.first(), value.first()) {
            (None, None) => true,
            (Some(b'*'), _) => {
                inner(&pattern[1..], value) || (!value.is_empty() && inner(pattern, &value[1..]))
            }
            (Some(b'?'), Some(_)) => inner(&pattern[1..], &value[1..]),
            (Some(p), Some(v)) if p == v => inner(&pattern[1..], &value[1..]),
            _ => false,
        }
    }
    inner(pattern.as_bytes(), value.as_bytes())
}

/// Evaluate an expression over a change-unit universe, returning the
/// selected indices.
pub fn eval(expr: &SelExpr, universe: &[ChangeUnit]) -> BTreeSet<usize> {
    match expr {
        SelExpr::Union(a, b) => eval(a, universe)
            .union(&eval(b, universe))
            .copied()
            .collect(),
        SelExpr::Intersect(a, b) => eval(a, universe)
            .intersection(&eval(b, universe))
            .copied()
            .collect(),
        SelExpr::Difference(a, b) => eval(a, universe)
            .difference(&eval(b, universe))
            .copied()
            .collect(),
        SelExpr::Atom(atom) => eval_atom(atom, universe),
    }
}

fn eval_atom(atom: &SelAtom, universe: &[ChangeUnit]) -> BTreeSet<usize> {
    let pick = |predicate: &dyn Fn(&ChangeUnit) -> bool| -> BTreeSet<usize> {
        universe
            .iter()
            .enumerate()
            .filter(|(_, unit)| predicate(unit))
            .map(|(index, _)| index)
            .collect()
    };
    match atom {
        SelAtom::Path(glob) => pick(&|unit| glob_matches(glob, &unit.path)),
        SelAtom::ByEffect(prefix) => pick(&|unit| unit.cut_id.starts_with(prefix.as_str())),
        SelAtom::ByOrigin(prefix) => pick(&|unit| {
            unit.origin
                .as_deref()
                .is_some_and(|origin| origin.starts_with(prefix.as_str()))
        }),
        SelAtom::InBranch(branch) => pick(&|unit| unit.branch_id == *branch),
        SelAtom::Change(change) => pick(&|unit| unit.change_id == *change),
        SelAtom::Cut(cut) => pick(&|unit| unit.cut_id == *cut),
        SelAtom::Since(stamp) => pick(&|unit| unit.recorded_at.as_str() >= stamp.as_str()),
        SelAtom::Until(stamp) => pick(&|unit| unit.recorded_at.as_str() <= stamp.as_str()),
        SelAtom::DependentsOf(inner) => {
            // The conservative dependence floor: a later unit on the
            // same path consumed the earlier one's output. Closure over
            // the universe; includes the seeds.
            let seeds = eval(inner, universe);
            let mut selected = seeds.clone();
            for &seed in &seeds {
                let seed_unit = &universe[seed];
                for (index, unit) in universe.iter().enumerate() {
                    if unit.path == seed_unit.path && unit.seq > seed_unit.seq {
                        selected.insert(index);
                    }
                }
            }
            selected
        }
    }
}

/// The stranding check (selective-undo.maude, the slicer's 7th client at
/// its path-level floor): undoing `selected` strands every RETAINED
/// later unit whose path input came from an undone write. Returns the
/// stranded indices — empty means the exclusion's dependency closure is
/// clean and the proposal is safe.
pub fn stranded_by_undo(selected: &BTreeSet<usize>, universe: &[ChangeUnit]) -> BTreeSet<usize> {
    let mut stranded = BTreeSet::new();
    for &chosen in selected {
        let undone = &universe[chosen];
        for (index, unit) in universe.iter().enumerate() {
            if unit.path == undone.path && unit.seq > undone.seq && !selected.contains(&index) {
                stranded.insert(index);
            }
        }
    }
    stranded
}

#[cfg(test)]
mod tests {
    use super::*;

    fn unit(seq: usize, cut: &str, path: &str, at: &str) -> ChangeUnit {
        ChangeUnit {
            seq,
            cut_id: cut.to_owned(),
            change_id: cut.to_owned(),
            branch_id: "b1".to_owned(),
            path: path.to_owned(),
            before: None,
            after: Some(format!("h{seq}")),
            origin: Some(format!("write:{path}")),
            recorded_at: at.to_owned(),
        }
    }

    #[test]
    fn grammar_parses_composition_with_precedence() {
        let expr = parse("path(src/*.md) & since(t3) | by-effect(eff_) ~ cut(c9)").expect("parse");
        // `|` binds loosest: (path & since) | (by-effect ~ cut).
        let SelExpr::Union(left, right) = expr else {
            panic!("expected a union at the top");
        };
        assert!(matches!(*left, SelExpr::Intersect(..)));
        assert!(matches!(*right, SelExpr::Difference(..)));
        assert!(parse("path(unclosed").is_err());
        assert!(parse("nonsense(x)").is_err());
        assert!(parse("path(a) extra").is_err());
    }

    #[test]
    fn atoms_select_by_provenance_dimensions() {
        let universe = vec![
            unit(0, "eff_1-f0", "src/a.md", "t1"),
            unit(1, "eff_2-f0", "src/b.md", "t2"),
            unit(2, "cut_x", "notes/c.txt", "t3"),
        ];
        let by_path = eval(&parse("path(src/*.md)").expect("parse"), &universe);
        assert_eq!(by_path, BTreeSet::from([0, 1]));
        let by_effect = eval(&parse("by-effect(eff_2)").expect("parse"), &universe);
        assert_eq!(by_effect, BTreeSet::from([1]));
        let since = eval(&parse("since(t2)").expect("parse"), &universe);
        assert_eq!(since, BTreeSet::from([1, 2]));
        let composed = eval(
            &parse("path(src/*.md) ~ by-effect(eff_2)").expect("parse"),
            &universe,
        );
        assert_eq!(composed, BTreeSet::from([0]));
        let grouped = eval(
            &parse("(path(src/*.md) | path(notes/*)) & until(t2)").expect("parse"),
            &universe,
        );
        assert_eq!(grouped, BTreeSet::from([0, 1]));
    }

    /// The model's fixture, verbatim: e1 wrote p1; e2, e3 wrote p2.
    /// Undoing p1 strands e3's path-level input? No — path-level
    /// dependence binds within a path: undoing e2 strands e3 (later on
    /// p2, retained); undoing p2 entirely (e2 AND e3) strands nothing;
    /// dependents-of(e2) pulls e3 into the selection.
    #[test]
    fn stranding_and_dependents_mirror_the_model() {
        let universe = vec![
            unit(0, "e1", "p1", "t1"),
            unit(1, "e2", "p2", "t2"),
            unit(2, "e3", "p2", "t3"),
        ];
        // Undo e2 alone: e3 is retained and read e2's output — stranded.
        let sel = eval(&parse("cut(e2)").expect("parse"), &universe);
        assert_eq!(stranded_by_undo(&sel, &universe), BTreeSet::from([2]));
        // Undo the whole path: the reader is inside the selection.
        let sel = eval(&parse("path(p2)").expect("parse"), &universe);
        assert!(stranded_by_undo(&sel, &universe).is_empty());
        // The closure operator repairs the stranding selection.
        let sel = eval(&parse("dependents-of(cut(e2))").expect("parse"), &universe);
        assert_eq!(sel, BTreeSet::from([1, 2]));
        assert!(stranded_by_undo(&sel, &universe).is_empty());
        // Undoing e1 strands nothing: nothing later touched p1.
        let sel = eval(&parse("cut(e1)").expect("parse"), &universe);
        assert!(stranded_by_undo(&sel, &universe).is_empty());
    }

    #[test]
    fn glob_semantics() {
        assert!(glob_matches("src/*.md", "src/deep/a.md"));
        assert!(glob_matches("*", "anything"));
        assert!(glob_matches("a?c", "abc"));
        assert!(!glob_matches("a?c", "ac"));
        assert!(!glob_matches("src/*.md", "src/a.txt"));
    }
}
