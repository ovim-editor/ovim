/// Line ending style for the buffer
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum LineEnding {
    /// Unix-style line endings (LF, \n)
    #[default]
    Lf,
    /// Windows-style line endings (CRLF, \r\n)
    Crlf,
}

impl LineEnding {
    /// Detects the line ending style from file content bytes
    pub fn detect(content: &[u8]) -> Self {
        // Look for \r\n first (Windows)
        for window in content.windows(2) {
            if window == b"\r\n" {
                return LineEnding::Crlf;
            }
        }
        // Default to LF (Unix) - this handles \n only or no line endings
        LineEnding::Lf
    }

    /// Returns the string representation of this line ending
    pub fn as_str(&self) -> &'static str {
        match self {
            LineEnding::Lf => "\n",
            LineEnding::Crlf => "\r\n",
        }
    }

    /// Returns a short display name for the status line
    pub fn display_name(&self) -> &'static str {
        match self {
            LineEnding::Lf => "LF",
            LineEnding::Crlf => "CRLF",
        }
    }
}
