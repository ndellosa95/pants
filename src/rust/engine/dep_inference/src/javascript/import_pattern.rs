// Copyright 2023 Pants project contributors (see CONTRIBUTORS.md).
// Licensed under the Apache License, Version 2.0 (see LICENSE).
use std::iter::{Once, once};
use std::ops::Deref;
use std::option::Option;
use std::path::Path;

use fnv::FnvHashSet as HashSet;
use fnv::{FnvHashMap as HashMap, FnvHashMap};
use itertools::Either;

#[derive(Debug, PartialEq, Eq)]
pub struct StarMatch<'a>(pub &'a str);

/// Implements the matching of "star map" patterns.
/// Essentially it is a static match, with an optional
/// string substitution in place of the '*' token.
///
/// The algorithm is implemented both in node and the typescript compiler,
/// the typescript compiler has an
/// [excellent comment](https://github.com/microsoft/TypeScript/blob/8fae437660ba89353fc7104beae8c6856528e5b6/src/compiler/moduleNameResolver.ts#L1399)
/// outlining the behaviour.
///
/// Nodejs also describes it vaguely in [NodeJS subpath patterns](https://nodejs.org/api/packages.html#subpath-patterns),
/// but also imposes some extra limitations. These are not validated here.
///
#[derive(Debug, PartialEq, Eq)]
pub enum Pattern<'a> {
    Match(usize, Option<StarMatch<'a>>),
    NoMatch,
}

impl<'a> Pattern<'a> {
    fn from_prefix(prefix: &str) -> Self {
        Pattern::Match(prefix.len(), None)
    }

    fn from_prefix_match(prefix: &str, star_match: &'a str) -> Self {
        Pattern::Match(prefix.len(), Some(StarMatch(star_match)))
    }

    pub fn matches(pattern: &str, import: &'a str) -> Self {
        let mut pattern_parts = pattern.split('*');
        let prefix = pattern_parts.next();
        let suffix = pattern_parts.next();
        if pattern_parts.next().is_some() || import.is_empty() {
            // Multiple '*' is not spec compliant, so never match.
            // Empty import strings aren't interesting.
            return Self::NoMatch;
        };
        match (prefix, suffix) {
            (Some(specifier), None) if specifier == import => Self::from_prefix(import),
            (None, _) => Self::NoMatch, // pattern is empty string.
            (Some(prefix), Some("")) => {
                // "<prefix>*", note that a single "*" also matches here.
                if let Some(star_match) = import.strip_prefix(prefix) {
                    Self::from_prefix_match(prefix, star_match)
                } else {
                    Self::NoMatch
                }
            }
            (Some(prefix), Some(suffix)) => {
                // "<prefix>*<suffix>"
                if let Some(star_match) = import
                    .strip_prefix(prefix)
                    .and_then(|prefix_stripped| prefix_stripped.strip_suffix(suffix))
                {
                    Self::from_prefix_match(prefix, star_match)
                } else {
                    Self::NoMatch
                }
            }
            _ => Self::NoMatch,
        }
    }
}

/// One of the results of testing all patterns against an import string.
#[derive(Debug, Eq, PartialEq, Hash)]
pub(crate) enum Import {
    /// A string that matched a pattern, with '*' substituion applied, when applicable.
    Matched(String),
    /// An unchanged string that did not match a pattern.
    UnMatched(String),
}

impl Deref for Import {
    type Target = String;

    fn deref(&self) -> &Self::Target {
        match self {
            Self::Matched(string) | Self::UnMatched(string) => string,
        }
    }
}

impl IntoIterator for Import {
    type Item = String;
    type IntoIter = Once<String>;

    fn into_iter(self) -> Self::IntoIter {
        once(match self {
            Self::Matched(string) | Self::UnMatched(string) => string,
        })
    }
}

/// Replaces patterns provided on the form outlined in
/// [NodeJS subpath patterns](https://nodejs.org/api/packages.html#subpath-patterns).
/// If no pattern matches, the import string is returned unchanged.
pub fn imports_from_patterns(
    root: &str,
    patterns: &HashMap<String, Vec<String>>,
    import: &str,
) -> HashSet<Import> {
    if let Some((star_match, pattern)) = find_best_match(patterns, import) {
        let mut matches = patterns[pattern]
            .iter()
            .filter_map(move |replacement| apply_replacements_to_match(&star_match, replacement))
            .map(|new_import| add_root_dir_to_dot_slash(root, new_import))
            .peekable();
        if matches.peek().is_some() {
            Either::Right(matches.map(Import::Matched))
        } else {
            Either::Left(once(import.to_string()).map(Import::UnMatched))
        }
    } else {
        Either::Left(once(import.to_string()).map(Import::UnMatched))
    }
    .collect()
}

fn apply_replacements_to_match(
    star_match: &Option<StarMatch>,
    replacement: &str,
) -> Option<String> {
    if let Some(StarMatch(star_match)) = star_match {
        if replacement.matches('*').count() != 1 {
            return None;
        }
        Some(replacement.replace('*', star_match))
    } else {
        Some(replacement.to_string())
    }
}

fn add_root_dir_to_dot_slash(root: &str, new_import: String) -> String {
    if let Some(("", rest)) = new_import.split_once("./") {
        Path::new(root).join(rest).to_str().unwrap().to_string()
    } else {
        new_import
    }
}

fn find_best_match<'a, 'b>(
    patterns: &'a FnvHashMap<String, Vec<String>>,
    import: &'b str,
) -> Option<(Option<StarMatch<'b>>, &'a String)> {
    patterns
        .keys()
        .map(|pattern| (pattern, Pattern::matches(pattern, import)))
        .filter_map(|(pattern, matched)| match matched {
            Pattern::Match(rank, star_match) => Some((rank, star_match, pattern)),
            _ => None,
        })
        .max_by(|(rank_x, _, _), (rank_y, _, _)| rank_x.cmp(rank_y))
        .map(|(_, pattern, star_match)| (pattern, star_match))
}
