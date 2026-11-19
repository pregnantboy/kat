//! Inline app icons via the iTerm2 image protocol (also understood by WezTerm).

use std::io::Write;

/// tmux passes the escape through badly, and inherits a stale TERM_PROGRAM, so icons are off inside it.
pub fn supported() -> bool {
    let term = std::env::var("TERM_PROGRAM").unwrap_or_default();
    std::env::var("KAT_ICONS").as_deref() != Ok("0")
        && std::env::var_os("TMUX").is_none()
        && matches!(term.as_str(), "iTerm.app" | "WezTerm")
}

pub fn base64(bytes: &[u8]) -> String {
    const T: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::with_capacity(bytes.len().div_ceil(3) * 4);
    for c in bytes.chunks(3) {
        let n = (c[0] as u32) << 16 | (*c.get(1).unwrap_or(&0) as u32) << 8 | *c.get(2).unwrap_or(&0) as u32;
        for i in 0..4 {
            out.push(if i <= c.len() { T[(n >> (18 - 6 * i) & 63) as usize] as char } else { '=' });
        }
    }
    out
}

/// Draws each (column, row, png) two cells wide and one row high.
pub fn draw(slots: &[(u16, u16, &[u8])]) -> std::io::Result<()> {
    let mut out = std::io::stdout().lock();
    for (x, y, png) in slots {
        write!(
            out,
            "\x1b[{};{}H\x1b]1337;File=inline=1;size={};width=2;height=1;preserveAspectRatio=1;doNotMoveCursor=1:{}\x07",
            y + 1,
            x + 1,
            png.len(),
            base64(png)
        )?;
    }
    out.flush()
}

#[cfg(test)]
mod tests {
    #[test]
    fn base64_matches_rfc4648() {
        assert_eq!(super::base64(b""), "");
        assert_eq!(super::base64(b"f"), "Zg==");
        assert_eq!(super::base64(b"fo"), "Zm8=");
        assert_eq!(super::base64(b"foo"), "Zm9v");
        assert_eq!(super::base64(b"foobar"), "Zm9vYmFy");
    }
}
