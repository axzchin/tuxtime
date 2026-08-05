use std::io::{self, Write};

/// Copy `content` to the system clipboard. Uses `arboard` (native system
/// clipboard via X11/Wayland/macOS/Windows) when available, falling back to
/// the OSC 52 escape sequence for terminal-based clipboard support.
pub fn copy(content: &str) -> io::Result<()> {
    // Try the native system clipboard first.
    if try_arboard(content) {
        return Ok(());
    }
    // Fall back to OSC 52 terminal escape sequence.
    let mut stdout = io::stdout();
    stdout.write_all(format_osc52(content).as_bytes())?;
    stdout.flush()
}

fn try_arboard(content: &str) -> bool {
    // Each call opens a fresh clipboard connection so the global clipboard
    // state reflects the most recent copy, even across multiple TUI frames.
    match arboard::Clipboard::new() {
        Ok(mut cb) => {
            // arboard::Clipboard::set_text can fail if the content is
            // empty on some platforms, so guard that edge case.
            if content.is_empty() {
                return false;
            }
            cb.set_text(content).is_ok()
        }
        Err(_) => false,
    }
}

/// Build an OSC 52 escape sequence that asks the controlling terminal to
/// place `content` on the system clipboard. Most modern terminals (kitty,
/// alacritty, wezterm, iTerm2, foot, modern xterm) honor this directly;
/// tmux forwards it when `set-clipboard on` is configured. Older terminals
/// silently ignore the sequence.
#[must_use]
pub fn format_osc52(content: &str) -> String {
    let encoded = base64_encode(content.as_bytes());
    format!("\x1b]52;c;{encoded}\x1b\\")
}

fn base64_encode(input: &[u8]) -> String {
    const ALPHABET: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::with_capacity(input.len().div_ceil(3) * 4);
    for chunk in input.chunks(3) {
        let b0 = u32::from(chunk[0]);
        let b1 = u32::from(chunk.get(1).copied().unwrap_or(0));
        let b2 = u32::from(chunk.get(2).copied().unwrap_or(0));
        let n = (b0 << 16) | (b1 << 8) | b2;
        out.push(ALPHABET[((n >> 18) & 0x3F) as usize] as char);
        out.push(ALPHABET[((n >> 12) & 0x3F) as usize] as char);
        out.push(if chunk.len() > 1 {
            ALPHABET[((n >> 6) & 0x3F) as usize] as char
        } else {
            '='
        });
        out.push(if chunk.len() > 2 {
            ALPHABET[(n & 0x3F) as usize] as char
        } else {
            '='
        });
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn osc52_wraps_base64_payload_in_escape_sequence() {
        assert_eq!(format_osc52("hi"), "\x1b]52;c;aGk=\x1b\\");
    }

    #[test]
    fn osc52_handles_empty_input() {
        assert_eq!(format_osc52(""), "\x1b]52;c;\x1b\\");
    }

    #[test]
    fn base64_known_vectors() {
        assert_eq!(base64_encode(b""), "");
        assert_eq!(base64_encode(b"f"), "Zg==");
        assert_eq!(base64_encode(b"fo"), "Zm8=");
        assert_eq!(base64_encode(b"foo"), "Zm9v");
        assert_eq!(base64_encode(b"foob"), "Zm9vYg==");
        assert_eq!(base64_encode(b"fooba"), "Zm9vYmE=");
        assert_eq!(base64_encode(b"foobar"), "Zm9vYmFy");
        assert_eq!(base64_encode("café".as_bytes()), "Y2Fmw6k=");
    }
}
