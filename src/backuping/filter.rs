#[cfg(test)] use std::fs::File;
use std::path::Path;

use cow_utils::CowUtils;
use globset::{GlobBuilder, GlobMatcher};
use serde::Deserialize;
#[cfg(test)] use serde_derive::Deserialize;
use serde::de::{Deserializer, Error};

use crate::core::GenericResult;

#[derive(Default)]
pub struct PathFilter {
    rules: Vec<Rule>,
}

impl PathFilter {
    pub fn new(spec: &str) -> GenericResult<PathFilter> {
        let mut rules = Vec::new();

        for line in spec.lines() {
            if let Some((glob, allow)) = parse_rule_line(line)? {
                rules.push(Rule::new(glob, allow)?);
            }
        }

        Ok(PathFilter {rules})
    }

    pub fn check(&self, path: &Path) -> GenericResult<bool> {
        let path = path.to_str().ok_or("Invalid path")?;

        for rule in &self.rules {
            if rule.matcher.is_match(path) {
                return Ok(rule.allow);
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
    matcher: GlobMatcher,
    allow: bool,
}

impl Rule {
    fn new(glob: &str, allow: bool) -> GenericResult<Rule> {
        // The glob library supports escaping only of glob control characters, so unescape other
        // common sequences manually.
        let unescaped =      glob.cow_replace(r"\t", "\t");
        let unescaped = unescaped.cow_replace(r"\n", "\n");
        let unescaped = unescaped.cow_replace(r"\r", "\r");
        let unescaped = unescaped.cow_replace(r"\ ", " ");

        let matcher = GlobBuilder::new(&unescaped)
            .literal_separator(true).backslash_escape(true)
            .build().map_err(|e| format!("Invalid glob ({:?}): {}", glob, e))?
            .compile_matcher();

        Ok(Rule {matcher, allow})
    }
}

fn parse_rule_line(mut line: &str) -> GenericResult<Option<(&str, bool)>> {
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

fn parse_rule(rule: &str) -> Option<(&str, bool)> {
    let mut chars = rule.chars();

    let allow = match chars.next()? {
        '+' => true,
        '-' => false,
        _ => return None,
    };

    if chars.next()? != ' ' {
        return None;
    }

    let glob = chars.as_str();
    if glob.is_empty() {
        return None;
    }

    Some((glob, allow))
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

        case("Icon\r", false),
        case("dir/Icon\r", false),

        case(".DS_Store", false),
        case("dir/.DS_Store", false),

        case("Downloads", false),
        case("Downloads/some-file", true), // But we won't get there because of blacklisted parent directory
        case("Other/Downloads", true),

        case(".investments", true),
        case(".investments/db.sqlite", false),
        case(".investments/config.yaml", true),

        case(".ssh", true),
        case(".ssh/config", true),
        case(".ssh/agent.socket", false),

        case(".vim", true),
        case(".vim/bundle", false),
        case(".vim/ftplugin", true),
        case(".vim/view", false),

        case(".vscode", true),
        case(".vscode/ssh", true),
        case(".vscode/ssh/config", true),
        case(".vscode/extensions", false),

        case("src/project", true),

        case("src/project/dir", true),
        case("src/project/dir/sub-project", true),
        case("src/project/dir/sub-project/buildtools", true),
        case("src/project/dir/sub-project/buildtools/bin", true),
        case("src/project/dir/sub-project/buildtools/darwin-bin", false),
        case("src/project/dir/sub-project/buildtools/linux-bin", false),

        case("src/project/.vagrant", true),
        case("src/project/.vagrant/machines", true),
        case("src/project/.vagrant/machines/default", true),
        case("src/project/.vagrant/machines/default/virtualbox", true),
        case("src/project/.vagrant/machines/default/virtualbox/id", true),
        case("src/project/.vagrant/machines/default/virtualbox/disk-1.vdi", false),

        case("src/obj.o", false),
        case("src/lib.so", false),
        case("src/project/obj.o", false),
        case("src/project/lib.so", false),
        case("src/project/inner/obj.o", false),
        case("src/project/inner/lib.so", false),
        case("src/project/obj.obj", true),
        case("src/project/lib.soc", true),
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

        case("+ ab/*/cd", Some(("ab/*/cd", true))),
        case("- ab/*/cd", Some(("ab/*/cd", false))),

        case("+  with spaces ", Some((" with spaces", true))),
        case("+ non-comment # rule ", Some(("non-comment # rule", true))),

        case(r"+ space at the end\ ", Some((r"space at the end\ ", true))),
        case(r"+ space at the end \ ", Some((r"space at the end \ ", true))),
        case(r"+ space at the end \  ", Some((r"space at the end \ ", true))),
    )]
    fn parsing(line: &str, result: Option<(&str, bool)>) {
        assert_eq!(parse_rule_line(line).unwrap(), result);
    }
}