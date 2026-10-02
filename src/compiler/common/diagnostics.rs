#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Severity {
    Error,
    Warning,
}

#[derive(Debug, Clone)]
pub struct Diagnostic {
    pub message: String,
    pub line: usize,
    pub col: usize, // 1-based column
    pub length: usize,
    pub file: String,
    pub code: String, // e.g., "E0100"
    pub severity: Severity,
    pub hint: Option<String>,  // Actionable fix suggestion
    pub label: Option<String>, // Inline label for the underline span
}

impl Diagnostic {
    pub fn new(message: String, line: usize, col: usize, file: String) -> Self {
        Self {
            message,
            line,
            col,
            length: 1,
            file,
            code: String::new(),
            severity: Severity::Error,
            hint: None,
            label: None,
        }
    }

    pub fn warning(message: String, line: usize, col: usize, file: String) -> Self {
        Self {
            message,
            line,
            col,
            length: 1,
            file,
            code: String::new(),
            severity: Severity::Warning,
            hint: None,
            label: None,
        }
    }

    pub fn with_code(mut self, code: &str) -> Self {
        self.code = code.to_string();
        self
    }

    pub fn with_hint(mut self, hint: &str) -> Self {
        self.hint = Some(hint.to_string());
        self
    }

    pub fn with_label(mut self, label: &str) -> Self {
        self.label = Some(label.to_string());
        self
    }

    pub fn report(&self, source: &str) {
        self.report_with_source(Some(source));
    }

    pub fn report_with_source(&self, source: Option<&str>) {
        let (sev_color, sev_name) = match self.severity {
            Severity::Error => ("\x1b[31;1m", "error"),
            Severity::Warning => ("\x1b[33;1m", "warning"),
        };

        // Header: error[E0100]: message
        if self.code.is_empty() {
            eprintln!(
                "{}{}:\x1b[0m \x1b[1m{}\x1b[0m",
                sev_color, sev_name, self.message
            );
        } else {
            eprintln!(
                "{}{}[{}]:\x1b[0m \x1b[1m{}\x1b[0m",
                sev_color, sev_name, self.code, self.message
            );
        }

        // Location
        eprintln!(
            "  \x1b[34m-->\x1b[0m {}:{}:{}",
            self.file, self.line, self.col
        );

        let Some(source) = source else {
            eprintln!("  \x1b[34m |\x1b[0m");
            eprintln!("  \x1b[34m =\x1b[0m source unavailable for diagnostic rendering");
            if let Some(hint) = &self.hint {
                eprintln!("  \x1b[34m =\x1b[0m \x1b[32;1mhint:\x1b[0m {}", hint);
            }
            eprintln!();
            return;
        };

        let lines: Vec<&str> = source.lines().collect();
        if self.line > 0 && self.line <= lines.len() {
            let line_content = lines[self.line - 1];
            let line_num_str = self.line.to_string();
            let pad = " ".repeat(line_num_str.len());

            let tab_width = 4;
            let mut rendered_line = String::new();
            let mut visual_col = 0;
            let mut current_char_idx = 1;

            for ch in line_content.chars() {
                if ch == '\t' {
                    let spaces = tab_width - (rendered_line.len() % tab_width);
                    rendered_line.push_str(&" ".repeat(spaces));
                    if current_char_idx < self.col {
                        visual_col += spaces;
                    }
                } else {
                    rendered_line.push(ch);
                    if current_char_idx < self.col {
                        visual_col += 1;
                    }
                }
                current_char_idx += 1;
            }

            eprintln!("  \x1b[34m{} |\x1b[0m", pad);

            // Error line
            eprintln!("  \x1b[34m{} |\x1b[0m {}", line_num_str, rendered_line);

            // Pointer line with carets
            let mut pointer = " ".repeat(visual_col);
            for _ in 0..self.length.max(1) {
                pointer.push('^');
            }

            if let Some(inline_label) = self.label.as_deref().filter(|label| *label != self.message)
            {
                eprintln!(
                    "  \x1b[34m{} |\x1b[0m {}{}{}\x1b[0m {}\x1b[0m",
                    pad, sev_color, pointer, sev_color, inline_label
                );
            } else {
                eprintln!("  \x1b[34m{} |\x1b[0m {}{}\x1b[0m", pad, sev_color, pointer);
            }
            eprintln!("  \x1b[34m{} |\x1b[0m", pad);

            // Hint line
            if let Some(hint) = &self.hint {
                eprintln!("  \x1b[34m{} =\x1b[0m \x1b[32;1mhint:\x1b[0m {}", pad, hint);
            }
        } else if self.line > lines.len() {
            // EOF error
            let line_num = self.line;
            let pad = " ".repeat(line_num.to_string().len());
            eprintln!("  \x1b[34m{} |\x1b[0m", pad);
            eprintln!("  \x1b[34m{} |\x1b[0m (EOF)", line_num);
            eprintln!(
                "  \x1b[34m{} |\x1b[0m {}^\x1b[0m {}{}\x1b[0m",
                pad, sev_color, sev_color, self.message
            );
            eprintln!("  \x1b[34m{} |\x1b[0m", pad);
            if let Some(hint) = &self.hint {
                eprintln!("  \x1b[34m{} =\x1b[0m \x1b[32;1mhint:\x1b[0m {}", pad, hint);
            }
        } else {
            eprintln!("  \x1b[34m |\x1b[0m");
            eprintln!(
                "  \x1b[34m =\x1b[0m could not render source context (line {} in file with {} lines)",
                self.line,
                lines.len()
            );
            if let Some(hint) = &self.hint {
                eprintln!("  \x1b[34m =\x1b[0m \x1b[32;1mhint:\x1b[0m {}", hint);
            }
        }
        eprintln!();
    }
}

/// Calculates the Levenshtein distance between two strings using optimal O(min(m, n)) space DP.
pub fn levenshtein_distance(a: &str, b: &str) -> usize {
    let a_chars: Vec<char> = a.chars().collect();
    let b_chars: Vec<char> = b.chars().collect();
    let (m, n) = (a_chars.len(), b_chars.len());
    if m == 0 {
        return n;
    }
    if n == 0 {
        return m;
    }

    let mut prev_row: Vec<usize> = (0..=n).collect();
    let mut curr_row: Vec<usize> = vec![0; n + 1];

    for i in 1..=m {
        curr_row[0] = i;
        for j in 1..=n {
            let cost = if a_chars[i - 1] == b_chars[j - 1] { 0 } else { 1 };
            curr_row[j] = (prev_row[j] + 1)
                .min(curr_row[j - 1] + 1)
                .min(prev_row[j - 1] + cost);
        }
        prev_row.copy_from_slice(&curr_row);
    }

    prev_row[n]
}

/// Computes the maximum allowed edit distance threshold based on query length.
pub fn max_allowed_distance(len: usize) -> usize {
    match len {
        0..=3 => 1,
        4..=7 => 2,
        _ => 3,
    }
}

/// Finds the best matching candidate from an iterator of string-like items for a given target.
/// Performs case-insensitive matching and edit distance ranking.
pub fn find_best_match<I, S>(target: &str, candidates: I) -> Option<String>
where
    I: IntoIterator<Item = S>,
    S: AsRef<str>,
{
    find_best_match_internal(target, candidates.into_iter().map(|s| s.as_ref().to_string()))
}

/// Alias for find_best_match.
pub fn find_best_match_str<I, S>(target: &str, candidates: I) -> Option<String>
where
    I: IntoIterator<Item = S>,
    S: AsRef<str>,
{
    find_best_match(target, candidates)
}

fn find_best_match_internal<I>(target: &str, candidates: I) -> Option<String>
where
    I: IntoIterator<Item = String>,
{
    let target_trimmed = target.trim();
    if target_trimmed.is_empty() {
        return None;
    }
    let target_lower = target_trimmed.to_lowercase();
    let target_len = target_lower.chars().count();
    let max_dist = max_allowed_distance(target_len);

    let mut best_candidate: Option<String> = None;
    let mut min_distance = usize::MAX;

    for cand in candidates {
        let cand_trimmed = cand.trim();
        if cand_trimmed.is_empty() || cand_trimmed == target_trimmed {
            continue;
        }

        let cand_lower = cand_trimmed.to_lowercase();
        // Exact case-insensitive match takes top priority
        if cand_lower == target_lower {
            return Some(cand_trimmed.to_string());
        }

        let dist = levenshtein_distance(&target_lower, &cand_lower);
        if dist <= max_dist && dist < min_distance {
            min_distance = dist;
            best_candidate = Some(cand_trimmed.to_string());
        }
    }

    best_candidate
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_levenshtein_distance() {
        assert_eq!(levenshtein_distance("kitten", "sitting"), 3);
        assert_eq!(levenshtein_distance("flaw", "lawn"), 2);
        assert_eq!(levenshtein_distance("", "abc"), 3);
        assert_eq!(levenshtein_distance("abc", ""), 3);
        assert_eq!(levenshtein_distance("same", "same"), 0);
    }

    #[test]
    fn test_find_best_match() {
        let candidates = ["println", "print", "panic", "sizeof", "length"];
        assert_eq!(find_best_match("pringln", candidates), Some("println".to_string()));
        assert_eq!(find_best_match("lengh", candidates), Some("length".to_string()));
        assert_eq!(find_best_match("Print", candidates), Some("print".to_string()));
        assert_eq!(find_best_match("completely_unrelated", candidates), None);
    }
}

