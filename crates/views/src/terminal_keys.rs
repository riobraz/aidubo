//! Pure encoding of key events into terminal input bytes.
//!
//! Kept free of GPUI so the table in the M14-03 spec can be tested directly.

/// The modifiers that affect terminal input.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct KeyMods {
    pub control: bool,
    pub alt: bool,
    pub shift: bool,
    /// Command on macOS, Windows key on Windows, Super on Linux.
    pub platform: bool,
}

/// Encode one key press, or `None` when the view should handle it itself.
///
/// `key` is the printed key name (the GPUI convention: `enter`, `escape`,
/// `pageup`, a single character, …). `key_char` is the character the platform
/// says could have been typed, which keeps layouts and AltGr working.
pub fn encode_key(modifiers: KeyMods, key: &str, key_char: Option<&str>) -> Option<Vec<u8>> {
    if modifiers.platform {
        return None;
    }

    if modifiers.control
        && let Some(byte) = control_byte(key)
    {
        return Some(vec![byte]);
    }

    if modifiers.alt {
        let mut bytes = vec![0x1b];
        bytes.extend(encode_key(
            KeyMods {
                alt: false,
                ..modifiers
            },
            key,
            key_char,
        )?);
        return Some(bytes);
    }

    let named = match key {
        "enter" => Some(b"\r".as_slice()),
        "backspace" => Some(b"\x7f".as_slice()),
        "tab" if modifiers.shift => Some(b"\x1b[Z".as_slice()),
        "tab" => Some(b"\t".as_slice()),
        "escape" => Some(b"\x1b".as_slice()),
        "up" => Some(b"\x1b[A".as_slice()),
        "down" => Some(b"\x1b[B".as_slice()),
        "right" => Some(b"\x1b[C".as_slice()),
        "left" => Some(b"\x1b[D".as_slice()),
        "home" => Some(b"\x1b[H".as_slice()),
        "end" => Some(b"\x1b[F".as_slice()),
        "pageup" => Some(b"\x1b[5~".as_slice()),
        "pagedown" => Some(b"\x1b[6~".as_slice()),
        "delete" => Some(b"\x1b[3~".as_slice()),
        _ => None,
    };
    if let Some(bytes) = named {
        return Some(bytes.to_vec());
    }

    let text = key_char.filter(|text| !text.is_empty()).or({
        let mut chars = key.chars();
        match (chars.next(), chars.next()) {
            (Some(_), None) => Some(key),
            _ => None,
        }
    })?;
    Some(text.as_bytes().to_vec())
}

/// Ctrl+letter uses the low five bits; Ctrl+Space is NUL.
fn control_byte(key: &str) -> Option<u8> {
    let mut chars = key.chars();
    match (chars.next(), chars.next()) {
        (Some(ch), None) => {
            let lower = ch.to_ascii_lowercase();
            if lower.is_ascii_lowercase() {
                Some((lower as u8) & 0x1f)
            } else if ch == ' ' {
                Some(0)
            } else {
                None
            }
        }
        _ => None,
    }
}

/// Paste text, guarded by the bracketed-paste sequences when the terminal
/// requested them.
pub fn encode_paste(text: &str, bracketed: bool) -> Vec<u8> {
    if bracketed {
        let mut bytes = Vec::with_capacity(text.len() + 12);
        bytes.extend_from_slice(b"\x1b[200~");
        bytes.extend_from_slice(text.as_bytes());
        bytes.extend_from_slice(b"\x1b[201~");
        bytes
    } else {
        text.as_bytes().to_vec()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const NONE: KeyMods = KeyMods {
        control: false,
        alt: false,
        shift: false,
        platform: false,
    };

    #[test]
    fn keys_encode_control_and_arrows() {
        assert_eq!(encode_key(NONE, "enter", None), Some(b"\r".to_vec()));
        assert_eq!(encode_key(NONE, "backspace", None), Some(b"\x7f".to_vec()));
        assert_eq!(encode_key(NONE, "tab", None), Some(b"\t".to_vec()));
        assert_eq!(
            encode_key(
                KeyMods {
                    shift: true,
                    ..NONE
                },
                "tab",
                None
            ),
            Some(b"\x1b[Z".to_vec())
        );
        assert_eq!(encode_key(NONE, "escape", None), Some(b"\x1b".to_vec()));
        assert_eq!(encode_key(NONE, "up", None), Some(b"\x1b[A".to_vec()));
        assert_eq!(encode_key(NONE, "down", None), Some(b"\x1b[B".to_vec()));
        assert_eq!(encode_key(NONE, "right", None), Some(b"\x1b[C".to_vec()));
        assert_eq!(encode_key(NONE, "left", None), Some(b"\x1b[D".to_vec()));
        assert_eq!(encode_key(NONE, "home", None), Some(b"\x1b[H".to_vec()));
        assert_eq!(encode_key(NONE, "end", None), Some(b"\x1b[F".to_vec()));
        assert_eq!(encode_key(NONE, "pageup", None), Some(b"\x1b[5~".to_vec()));
        assert_eq!(
            encode_key(NONE, "pagedown", None),
            Some(b"\x1b[6~".to_vec())
        );

        assert_eq!(
            encode_key(
                KeyMods {
                    control: true,
                    ..NONE
                },
                "c",
                None
            ),
            Some(vec![0x03])
        );
        assert_eq!(
            encode_key(KeyMods { alt: true, ..NONE }, "x", Some("x")),
            Some(b"\x1bx".to_vec())
        );
        assert_eq!(encode_key(NONE, "a", Some("a")), Some(b"a".to_vec()));
        assert_eq!(
            encode_key(NONE, "a", Some("ç")),
            Some("ç".as_bytes().to_vec())
        );
        assert_eq!(
            encode_key(
                KeyMods {
                    platform: true,
                    ..NONE
                },
                "c",
                Some("c")
            ),
            None
        );
    }

    #[test]
    fn keys_paste_uses_bracketed_mode() {
        assert_eq!(encode_paste("ls", true), b"\x1b[200~ls\x1b[201~".to_vec());
        assert_eq!(encode_paste("ls", false), b"ls".to_vec());
    }
}
