//! Search filter and Google-like dork parser for OmniFind.
//!
//! Supports:
//! - File extension search: `ext:pdf`, `filetype:png`, `type:docx`, `ext:jpg,png`, `*.rs`, `.toml`
//! - Filename search: `name:report`, `filename:config`, `intitle:readme`
//! - Path search: `path:finance`, `dir:rust`, `folder:projects`, `inpath:src`, `inurl:test`
//! - Drive search: `drive:c`, `site:d:`
//! - Exclude / NOT: `-draft`, `-ext:tmp`, `-path:node_modules`, `-name:test`, `-"secret file"`
//! - Exact phrase: `"exact phrase"`
//! - OR search: `report OR invoice`, `cat | dog`
//! - Multi-word Google AND: `quarterly budget 2024`

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct SearchFilter {
    /// Allowed file extensions (lowercase, without leading dot), e.g. ["pdf", "docx"]
    pub extensions: Vec<String>,
    /// Excluded file extensions, e.g. ["tmp", "log"]
    pub excluded_extensions: Vec<String>,

    /// Allowed drives (lowercase single letter), e.g. ["c", "d"]
    pub drives: Vec<String>,
    /// Excluded drives
    pub excluded_drives: Vec<String>,

    /// Substrings that the directory path must contain (lowercase)
    pub path_includes: Vec<String>,
    /// Substrings that the directory path must NOT contain
    pub path_excludes: Vec<String>,

    /// Substrings that filename must contain (lowercase)
    pub name_includes: Vec<String>,
    /// Substrings that filename must NOT contain
    pub name_excludes: Vec<String>,

    /// General terms grouped for boolean logic:
    /// Outer Vec is AND: every group must match.
    /// Inner Vec is OR: at least one term in the group must match.
    pub term_groups: Vec<Vec<String>>,

    /// Excluded terms (cannot appear anywhere in the full path)
    pub excluded_terms: Vec<String>,
}

impl SearchFilter {
    /// Returns true if the filter has no search criteria whatsoever.
    pub fn is_empty(&self) -> bool {
        self.extensions.is_empty()
            && self.excluded_extensions.is_empty()
            && self.drives.is_empty()
            && self.excluded_drives.is_empty()
            && self.path_includes.is_empty()
            && self.path_excludes.is_empty()
            && self.name_includes.is_empty()
            && self.name_excludes.is_empty()
            && self.term_groups.is_empty()
            && self.excluded_terms.is_empty()
    }

    /// Parses a user search query string into a structured `SearchFilter`.
    pub fn parse(query: &str) -> Self {
        let mut filter = SearchFilter::default();
        let tokens = tokenize(query);

        let mut current_or_group: Vec<String> = Vec::new();
        let mut expect_or = false;

        for token in &tokens {
            let token_trimmed = token.trim();
            if token_trimmed.is_empty() {
                continue;
            }

            // Check for OR operator: `OR` or `|`
            if token_trimmed.eq_ignore_ascii_case("or") || token_trimmed == "|" {
                if !current_or_group.is_empty() {
                    expect_or = true;
                }
                continue;
            }

            // Check if token has a dork prefix before any quotes (e.g. `ext:pdf`, `path:"my docs"`)
            let colon_idx = token_trimmed.find(':');
            let quote_idx = token_trimmed.find(|c| c == '"' || c == '\'');

            let is_dork = match (colon_idx, quote_idx) {
                (Some(c), Some(q)) => c < q,
                (Some(_), None) => true,
                _ => false,
            };

            if is_dork {
                let c_idx = colon_idx.unwrap();
                let prefix = &token_trimmed[..c_idx];
                let raw_val = &token_trimmed[c_idx + 1..];
                let val = strip_quotes(raw_val).trim();

                let is_negated = prefix.starts_with('-');
                let prefix_name = if is_negated {
                    prefix[1..].to_ascii_lowercase()
                } else {
                    prefix.to_ascii_lowercase()
                };

                match prefix_name.as_str() {
                    "ext" | "filetype" | "type" | "extension" => {
                        for part in val.split(',') {
                            let clean = part.trim().trim_start_matches('.').to_ascii_lowercase();
                            if !clean.is_empty() {
                                if is_negated {
                                    if !filter.excluded_extensions.contains(&clean) {
                                        filter.excluded_extensions.push(clean);
                                    }
                                } else if !filter.extensions.contains(&clean) {
                                    filter.extensions.push(clean);
                                }
                            }
                        }
                    }
                    "path" | "dir" | "folder" | "inpath" | "inurl" => {
                        let clean = val.to_ascii_lowercase().replace('/', "\\");
                        if !clean.is_empty() {
                            if is_negated {
                                filter.path_excludes.push(clean);
                            } else {
                                filter.path_includes.push(clean);
                            }
                        }
                    }
                    "name" | "filename" | "intitle" | "file" => {
                        let clean = val.to_ascii_lowercase();
                        if !clean.is_empty() {
                            if is_negated {
                                filter.name_excludes.push(clean);
                            } else {
                                filter.name_includes.push(clean);
                            }
                        }
                    }
                    "drive" | "site" => {
                        let clean = val
                            .trim_end_matches('\\')
                            .trim_end_matches('/')
                            .trim_end_matches(':')
                            .to_ascii_lowercase();
                        if !clean.is_empty() {
                            if is_negated {
                                filter.excluded_drives.push(clean);
                            } else {
                                filter.drives.push(clean);
                            }
                        }
                    }
                    _ => {
                        // Unrecognized dork prefix: treat token as literal term
                        let clean = strip_quotes(token_trimmed).to_ascii_lowercase();
                        if !clean.is_empty() {
                            add_term(&mut filter, &mut current_or_group, &mut expect_or, clean);
                        }
                    }
                }
                continue;
            }

            // Excluded term or phrase: -term or -"quoted term"
            if token_trimmed.starts_with('-') && token_trimmed.len() > 1 {
                let val = strip_quotes(&token_trimmed[1..]).trim().to_ascii_lowercase();
                if !val.is_empty() {
                    filter.excluded_terms.push(val);
                }
                continue;
            }

            // Wildcard extension: *.ext (e.g. *.pdf, *.rs)
            if token_trimmed.starts_with("*.") && token_trimmed.len() > 2 {
                let ext = token_trimmed[2..].trim().to_ascii_lowercase();
                if !ext.is_empty() && !filter.extensions.contains(&ext) {
                    filter.extensions.push(ext);
                }
                continue;
            }

            // Shorthand extension: .ext (e.g. .pdf, .rs, .json)
            if token_trimmed.starts_with('.')
                && token_trimmed.len() > 1
                && !token_trimmed[1..].contains('.')
                && !token_trimmed[1..].contains('\\')
                && !token_trimmed[1..].contains('/')
            {
                let ext = token_trimmed[1..].trim().to_ascii_lowercase();
                if !ext.is_empty() && !filter.extensions.contains(&ext) {
                    filter.extensions.push(ext);
                }
                continue;
            }

            // Normal term or quoted phrase
            let clean = strip_quotes(token_trimmed).trim().to_ascii_lowercase();
            if !clean.is_empty() {
                add_term(&mut filter, &mut current_or_group, &mut expect_or, clean);
            }
        }

        if !current_or_group.is_empty() {
            filter.term_groups.push(current_or_group);
        }

        filter
    }

    /// Evaluates if `path` (lowercased) satisfies all search criteria.
    /// If it matches, returns `Some(score)` where higher scores indicate better relevance.
    /// Returns `None` if any filter fails.
    #[inline]
    pub fn matches_and_score(&self, path: &str) -> Option<i32> {
        // 1. Check drive constraint
        if !self.drives.is_empty() || !self.excluded_drives.is_empty() {
            if let Some(dc) = path.chars().next() {
                let d_str = dc.to_ascii_lowercase().to_string();
                if !self.drives.is_empty() && !self.drives.iter().any(|d| d == &d_str) {
                    return None;
                }
                if self.excluded_drives.iter().any(|d| d == &d_str) {
                    return None;
                }
            }
        }

        // 2. Separate directory and filename
        let (dir_part, filename) = match path.rfind('\\') {
            Some(idx) => (&path[..idx], &path[idx + 1..]),
            None => ("", path),
        };

        // 3. Check extension
        let file_ext = match filename.rfind('.') {
            Some(idx) => &filename[idx + 1..],
            None => "",
        };

        if !self.extensions.is_empty() {
            let matched_ext = self.extensions.iter().any(|ext| {
                ext == file_ext
                    || (filename.len() > ext.len() + 1
                        && filename.ends_with(ext)
                        && filename.as_bytes()[filename.len() - ext.len() - 1] == b'.')
            });
            if !matched_ext {
                return None;
            }
        }

        if !self.excluded_extensions.is_empty() {
            let matched_exc = self.excluded_extensions.iter().any(|ext| {
                ext == file_ext
                    || (filename.len() > ext.len() + 1
                        && filename.ends_with(ext)
                        && filename.as_bytes()[filename.len() - ext.len() - 1] == b'.')
            });
            if matched_exc {
                return None;
            }
        }

        // 4. Path includes & excludes
        for inc in &self.path_includes {
            if !path.contains(inc) {
                return None;
            }
        }
        for exc in &self.path_excludes {
            if path.contains(exc) {
                return None;
            }
        }

        // 5. Name includes & excludes
        for inc in &self.name_includes {
            if !filename.contains(inc) {
                return None;
            }
        }
        for exc in &self.name_excludes {
            if filename.contains(exc) {
                return None;
            }
        }

        // 6. Excluded terms (anywhere in path)
        for exc in &self.excluded_terms {
            if path.contains(exc) {
                return None;
            }
        }

        // 7. Term groups (AND of OR groups)
        for group in &self.term_groups {
            let mut group_matched = false;
            for term in group {
                if path.contains(term) {
                    group_matched = true;
                    break;
                }
            }
            if !group_matched {
                return None;
            }
        }

        // Relevance Scoring
        let mut score: i32 = 100;

        let filename_no_ext = match filename.rfind('.') {
            Some(idx) => &filename[..idx],
            None => filename,
        };

        for group in &self.term_groups {
            for term in group {
                if filename == term || filename_no_ext == term {
                    score += 10000; // Exact filename match
                } else if filename.starts_with(term) {
                    score += 5000; // Filename prefix match
                } else if filename.contains(term) {
                    score += 1000; // Filename substring match
                } else if dir_part.contains(term) {
                    score += 50; // Path directory match
                }
            }
        }

        for inc in &self.name_includes {
            if filename == inc || filename_no_ext == inc {
                score += 8000;
            } else if filename.starts_with(inc) {
                score += 4000;
            } else {
                score += 1000;
            }
        }

        if !self.extensions.is_empty() {
            score += 200;
        }

        Some(score)
    }

    /// Provides badge summaries of active filters for UI display.
    pub fn badges(&self) -> Vec<(String, String)> {
        let mut badges = Vec::new();

        if !self.extensions.is_empty() {
            badges.push(("ext".to_string(), self.extensions.join(", ")));
        }
        if !self.name_includes.is_empty() {
            badges.push(("name".to_string(), self.name_includes.join(", ")));
        }
        if !self.path_includes.is_empty() {
            badges.push(("path".to_string(), self.path_includes.join(", ")));
        }
        if !self.drives.is_empty() {
            badges.push(("drive".to_string(), self.drives.join(", ")));
        }
        if !self.excluded_terms.is_empty() || !self.excluded_extensions.is_empty() || !self.path_excludes.is_empty() {
            let mut excludes = Vec::new();
            for ext in &self.excluded_extensions {
                excludes.push(format!(".{}", ext));
            }
            excludes.extend(self.excluded_terms.clone());
            excludes.extend(self.path_excludes.clone());
            badges.push(("exclude".to_string(), excludes.join(", ")));
        }

        badges
    }
}

fn add_term(
    filter: &mut SearchFilter,
    current_or_group: &mut Vec<String>,
    expect_or: &mut bool,
    term: String,
) {
    if *expect_or {
        current_or_group.push(term);
        *expect_or = false;
    } else {
        if !current_or_group.is_empty() {
            filter.term_groups.push(std::mem::take(current_or_group));
        }
        current_or_group.push(term);
    }
}

/// Tokenizer that preserves quoted strings and attaches dork prefixes correctly.
/// Uses zero heap allocations for character scanning.
fn tokenize(input: &str) -> Vec<String> {
    let mut tokens = Vec::new();
    let mut current = String::new();
    let mut in_quotes = false;
    let mut quote_char = '"';

    for (idx, ch) in input.char_indices() {
        if in_quotes {
            current.push(ch);
            if ch == quote_char {
                in_quotes = false;
            }
        } else if ch == '"' || ch == '\'' {
            // Check if there is a matching closing quote later in the string slice
            let remainder = &input[idx + ch.len_utf8()..];
            if remainder.contains(ch) {
                in_quotes = true;
                quote_char = ch;
                current.push(ch);
            }
            // If dangling/unclosed quote, we do not enter in_quotes mode
        } else if ch.is_whitespace() {
            if !current.is_empty() {
                tokens.push(std::mem::take(&mut current));
            }
        } else {
            current.push(ch);
        }
    }
    if !current.is_empty() {
        tokens.push(current);
    }
    tokens
}

fn strip_quotes(s: &str) -> &str {
    let s = s.trim();
    if (s.starts_with('"') && s.ends_with('"') && s.len() >= 2)
        || (s.starts_with('\'') && s.ends_with('\'') && s.len() >= 2)
    {
        &s[1..s.len() - 1]
    } else if s.starts_with('"') || s.starts_with('\'') {
        &s[1..]
    } else {
        s
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_extension_parsing() {
        let f = SearchFilter::parse("ext:pdf,docx");
        assert_eq!(f.extensions, vec!["pdf", "docx"]);

        let f2 = SearchFilter::parse("filetype:png");
        assert_eq!(f2.extensions, vec!["png"]);

        let f3 = SearchFilter::parse("*.rs");
        assert_eq!(f3.extensions, vec!["rs"]);

        let f4 = SearchFilter::parse(".toml");
        assert_eq!(f4.extensions, vec!["toml"]);

        let f5 = SearchFilter::parse("-ext:tmp");
        assert_eq!(f5.excluded_extensions, vec!["tmp"]);
    }

    #[test]
    fn test_dorks_parsing() {
        let f = SearchFilter::parse("path:work name:budget drive:c -temp");
        assert_eq!(f.path_includes, vec!["work"]);
        assert_eq!(f.name_includes, vec!["budget"]);
        assert_eq!(f.drives, vec!["c"]);
        assert_eq!(f.excluded_terms, vec!["temp"]);
    }

    #[test]
    fn test_quotes_and_or() {
        let f = SearchFilter::parse(r#""annual report" OR summary ext:pdf"#);
        assert_eq!(f.term_groups, vec![vec!["annual report", "summary"]]);
        assert_eq!(f.extensions, vec!["pdf"]);
    }

    #[test]
    fn test_matching_and_scoring() {
        let f = SearchFilter::parse("budget ext:xlsx path:finance -draft");
        let path1 = r"c:\documents\finance\budget_2024.xlsx";
        let path2 = r"c:\documents\finance\draft_budget.xlsx";
        let path3 = r"c:\documents\marketing\budget_2024.xlsx";
        let path4 = r"c:\documents\finance\budget_2024.pdf";

        assert!(f.matches_and_score(path1).is_some());
        assert!(f.matches_and_score(path2).is_none()); // excluded by -draft
        assert!(f.matches_and_score(path3).is_none()); // missing path:finance
        assert!(f.matches_and_score(path4).is_none()); // wrong extension
    }

    #[test]
    fn test_or_matching() {
        let f = SearchFilter::parse("cat OR dog ext:png");
        assert!(f.matches_and_score(r"c:\images\my_cat.png").is_some());
        assert!(f.matches_and_score(r"c:\images\good_dog.png").is_some());
        assert!(f.matches_and_score(r"c:\images\bird.png").is_none());
    }

    #[test]
    fn test_unclosed_quotes() {
        let f = SearchFilter::parse(r#""report ext:pdf"#);
        assert_eq!(f.term_groups, vec![vec!["report".to_string()]]);
        assert_eq!(f.extensions, vec!["pdf".to_string()]);
    }
}

