#[derive(Debug, Clone)]
pub struct Diagnostic {
    pub message: String,
    pub line: usize,
    pub col: usize, // 1-based column
    pub length: usize,
    pub file: String,
}

impl Diagnostic {
    pub fn new(message: String, line: usize, col: usize, file: String) -> Self {
        Self {
            message,
            line,
            col,
            length: 1, // Default length
            file,
        }
    }
    
    pub fn report(&self, source: &str) {
        // Red: \x1b[31m, Blue: \x1b[34m, Bold: \x1b[1m, Reset: \x1b[0m
        eprintln!("\x1b[31;1mError:\x1b[0m \x1b[1m{}\x1b[0m", self.message);
        eprintln!("  \x1b[34m-->\x1b[0m {}:{}:{}", self.file, self.line, self.col);
        
        let lines: Vec<&str> = source.lines().collect();
        if self.line > 0 && self.line <= lines.len() {
            let line_content = lines[self.line - 1];
            let line_num_str = self.line.to_string();
            let pad = " ".repeat(line_num_str.len());
            
            eprintln!("  \x1b[34m{} |\x1b[0m", pad);
            eprintln!("  \x1b[34m{} |\x1b[0m {}", line_num_str, line_content);
            
            let mut pointer = String::new();
            for _ in 0..self.col.saturating_sub(1) {
                pointer.push(' ');
            }
            // Pointer length based on self.length
            for _ in 0..self.length.max(1) {
                pointer.push('^');
            }
            
            eprintln!("  \x1b[34m{} |\x1b[0m \x1b[31;1m{}\x1b[0m \x1b[31m{}\x1b[0m", pad, pointer, self.message);
            eprintln!("  \x1b[34m{} |\x1b[0m", pad);
        } else if self.line > lines.len() {
            // EOF error
            let line_num = self.line;
            let pad = " ".repeat(line_num.to_string().len());
            eprintln!("  \x1b[34m{} |\x1b[0m", pad);
            eprintln!("  \x1b[34m{} |\x1b[0m (EOF)", line_num);
            eprintln!("  \x1b[34m{} |\x1b[0m \x1b[31;1m^\x1b[0m \x1b[31m{}\x1b[0m", pad, self.message);
            eprintln!("  \x1b[34m{} |\x1b[0m", pad);
        } else {
             eprintln!("  (Unexpected line {} with total lines: {})", self.line, lines.len());
        }
    }
}
