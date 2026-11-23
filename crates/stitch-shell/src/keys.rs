//! Named keys, as the bytes a terminal sends for them.

/// The bytes for a key name (`ctrl-c`, `enter`, `up`, ...), case-insensitive.
pub fn key(name: &str) -> Option<&'static [u8]> {
  Some(match name.to_ascii_lowercase().as_str() {
    "enter" | "return" => b"\r",
    "tab" => b"\t",
    "esc" | "escape" => b"\x1b",
    "backspace" => b"\x7f",
    "space" => b" ",
    "up" => b"\x1b[A",
    "down" => b"\x1b[B",
    "right" => b"\x1b[C",
    "left" => b"\x1b[D",
    "home" => b"\x1b[H",
    "end" => b"\x1b[F",
    "pageup" => b"\x1b[5~",
    "pagedown" => b"\x1b[6~",
    "delete" => b"\x1b[3~",
    "ctrl-a" => b"\x01",
    "ctrl-b" => b"\x02",
    "ctrl-c" => b"\x03",
    "ctrl-d" => b"\x04",
    "ctrl-e" => b"\x05",
    "ctrl-k" => b"\x0b",
    "ctrl-l" => b"\x0c",
    "ctrl-r" => b"\x12",
    "ctrl-u" => b"\x15",
    "ctrl-w" => b"\x17",
    "ctrl-z" => b"\x1a",
    "ctrl-\\" => b"\x1c",
    _ => return None,
  })
}
