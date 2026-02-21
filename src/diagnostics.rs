#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Severity {
    Error,
}

#[derive(Debug, Clone)]
pub struct Diagnostic {
    pub message: String,
    pub line: usize,
    pub col: usize, // 1-based column
    pub length: usize,
    pub file: String,
    pub code: String,           // e.g., "E0100"
    pub severity: Severity,
    pub hint: Option<String>,   // Actionable fix suggestion
    pub label: Option<String>,  // Inline label for the underline span
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
        let (sev_color, sev_name) = match self.severity {
            Severity::Error   => ("\x1b[31;1m", "error"),
        };

        // Header: error[E0100]: message
        if self.code.is_empty() {
            eprintln!("{}{}:\x1b[0m \x1b[1m{}\x1b[0m", sev_color, sev_name, self.message);
        } else {
            eprintln!("{}{}[{}]:\x1b[0m \x1b[1m{}\x1b[0m", sev_color, sev_name, self.code, self.message);
        }

        // Location
        eprintln!("  \x1b[34m-->\x1b[0m {}:{}:{}", self.file, self.line, self.col);

        let lines: Vec<&str> = source.lines().collect();
        if self.line > 0 && self.line <= lines.len() {
            let line_content = lines[self.line - 1];
            let line_num_str = self.line.to_string();
            let pad = " ".repeat(line_num_str.len());

            // Context: show line before if available
            if self.line >= 2 {
                let prev_line = lines[self.line - 2];
                let prev_num = (self.line - 1).to_string();
                let prev_pad = " ".repeat(line_num_str.len().saturating_sub(prev_num.len()));
                eprintln!("  \x1b[34m{} |\x1b[0m", pad);
                eprintln!("  \x1b[34m{}{} |\x1b[0m {}", prev_pad, prev_num, prev_line);
            } else {
                eprintln!("  \x1b[34m{} |\x1b[0m", pad);
            }

            // Error line
            eprintln!("  \x1b[34m{} |\x1b[0m {}", line_num_str, line_content);

            // Pointer line with carets
            let mut pointer = String::new();
            for _ in 0..self.col.saturating_sub(1) {
                pointer.push(' ');
            }
            for _ in 0..self.length.max(1) {
                pointer.push('^');
            }

            let inline_label = self.label.as_deref().unwrap_or(&self.message);
            eprintln!("  \x1b[34m{} |\x1b[0m {}{}{}\x1b[0m {}{}\x1b[0m",
                pad, sev_color, pointer, sev_color, inline_label, "");
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
            eprintln!("  \x1b[34m{} |\x1b[0m {}^\x1b[0m {}{}\x1b[0m", pad, sev_color, sev_color, self.message);
            eprintln!("  \x1b[34m{} |\x1b[0m", pad);
            if let Some(hint) = &self.hint {
                eprintln!("  \x1b[34m{} =\x1b[0m \x1b[32;1mhint:\x1b[0m {}", pad, hint);
            }
        } else {
            eprintln!("  (Unexpected line {} with total lines: {})", self.line, lines.len());
        }
        eprintln!();
    }
}
