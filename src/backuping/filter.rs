#[cfg(test)] use std::fs::File;
use std::path::Path;

use cow_utils::CowUtils;
use globset::{GlobBuilder, GlobMatcher};
use serde::Deserialize;
#[cfg(test)] use serde_derive::Deserialize;
use serde::de::{Deserializer, Error};

use crate::core::GenericResult;

pub struct PathFilter {
    rules: Vec<Rule>,
}

impl PathFilter {
    fn new(spec: &str) -> GenericResult<PathFilter> {
        let mut rules = Vec::new();

        for line in spec.lines() {
            if let Some((matcher, allow)) = parse_rule_line(line)? {
                rules.push(Rule {
                    matcher: matcher.compile()?,
                    allow,
                })
            }
        }

        Ok(PathFilter {rules})
    }

    // FIXME(konishchev): Implement
    pub fn check(&self, path: &Path) -> GenericResult<bool> {
        for rule in &self.rules {
            match rule.matcher {
                CompiledMatcher::Glob(ref glob) => {
                    if glob.is_match(path) {
                        return Ok(rule.allow);
                    }
                },
            }
        }

        Ok(true)
    }
}

impl<'de> Deserialize<'de> for PathFilter {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error> where D: Deserializer<'de> {
        let spec: String = Deserialize::deserialize(deserializer)?;
        PathFilter::new(&spec).map_err(D::Error::custom)
    }
}

struct Rule {
    matcher: CompiledMatcher,
    allow: bool,
}

enum CompiledMatcher {
    Glob(GlobMatcher)
}

#[derive(Debug, PartialEq)]
enum Matcher<'a> {
    Glob(&'a str),
    Regex(&'a str),
}

impl<'a> Matcher<'a> {
    // FIXME(konishchev): Implement
    fn compile(self) -> GenericResult<CompiledMatcher> {
        Ok(match self {
            Matcher::Glob(text) => {
                // The glob library supports escaping only of glob control characters, so unescape
                // other common sequences manually.
                let unescaped =      text.cow_replace(r"\t", "\t");
                let unescaped = unescaped.cow_replace(r"\n", "\n");
                let unescaped = unescaped.cow_replace(r"\r", "\r");
                let unescaped = unescaped.cow_replace(r"\ ", " ");

                CompiledMatcher::Glob(GlobBuilder::new(&unescaped)
                    .literal_separator(true).backslash_escape(true)
                    .build().map_err(|e| format!("Invalid glob ({:?}): {}", text, e))?
                    .compile_matcher())
            },

            Matcher::Regex(_) => unimplemented!(),
        })
    }
}

fn parse_rule_line(mut line: &str) -> GenericResult<Option<(Matcher, bool)>> {
    let is_whitespace = |c| matches!(c, ' ' | '\t');

    line = line.trim_start_matches(is_whitespace);
    if line.is_empty() || line.starts_with('#') {
        return Ok(None);
    }

    let mut end_pos = line.len();
    let mut rev_chars = line.char_indices().rev().peekable();

    loop {
        let (index, char) = rev_chars.next().unwrap();
        if !is_whitespace(char) {
            break;
        }

        if rev_chars.peek().unwrap().1 == '\\' {
            break;
        }

        end_pos = index;
    }

    let rule = &line[..end_pos];
    Ok(Some(parse_rule(rule).ok_or_else(|| format!("Invalid filter rule: {:?}", rule))?))
}

fn parse_rule(rule: &str) -> Option<(Matcher, bool)> {
    let mut chars = rule.chars();

    let allow = match chars.next()? {
        '+' => true,
        '-' => false,
        _ => return None,
    };

    let matcher = chars.next()?;
    let space = chars.next()?;
    let text = chars.as_str();

    if space != ' ' || text.is_empty() {
        return None;
    }

    Some((match matcher {
        '*' => Matcher::Glob(text),
        '~' => Matcher::Regex(text),
        _ => return None,
    }, allow))
}

#[cfg(test)]
mod tests {
    use rstest::{rstest, fixture};
    use super::*;

    #[fixture]
    #[once]
    fn filter() -> PathFilter {
        #[derive(Deserialize)]
        #[serde(deny_unknown_fields)]
        struct Config {
            filter: PathFilter,
        }

        let path = Path::new(file!()).parent().unwrap().join("testdata/filter.yaml");
        let filter = serde_yaml::from_reader::<_, Config>(File::open(path).unwrap()).unwrap().filter;
        assert!(!filter.rules.is_empty());

        filter
    }

    #[rstest(path, expected,
        case("some-file", true),
        case("some-dir/some-file", true),
        case("some/excluded/file", false),

        case("Icon\r", false),
        case("dir/Icon\r", false),

        case(".DS_Store", false),
        case("dir/.DS_Store", false),
    )]
    fn filtering(filter: &PathFilter, path: &str, expected: bool) {
        let path = Path::new(path);
        let allow = filter.check(path).unwrap();
        assert_eq!(allow, expected, "{:?} -> {}", path, allow);
    }

    #[rstest(line, result,
        case("", None),
        case(" ", None),
        case(" # Some comment ", None),

        case("+* glob", Some((Matcher::Glob("glob"), true))),
        case("-* glob", Some((Matcher::Glob("glob"), false))),

        case("+*  with spaces ", Some((Matcher::Glob(" with spaces"), true))),
        case("+* non-comment # rule ", Some((Matcher::Glob("non-comment # rule"), true))),
        case(r"+~ space at the end \  ", Some((Matcher::Regex(r"space at the end \ "), true))),
    )]
    fn parsing(line: &str, result: Option<(Matcher, bool)>) {
        assert_eq!(parse_rule_line(line).unwrap(), result);
    }
}