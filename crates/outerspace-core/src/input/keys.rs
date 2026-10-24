//! Key combos as agents write them ("ctrl+s", "alt+f4", "enter", "shift+tab") → virtual-key codes.

use windows::Win32::UI::Input::KeyboardAndMouse::{
    VIRTUAL_KEY, VK_BACK, VK_CAPITAL, VK_CONTROL, VK_DELETE, VK_DOWN, VK_END, VK_ESCAPE, VK_F1, VK_HOME, VK_INSERT,
    VK_LEFT, VK_LWIN, VK_MENU, VK_NEXT, VK_PRIOR, VK_RETURN, VK_RIGHT, VK_SHIFT, VK_SNAPSHOT, VK_SPACE, VK_TAB, VK_UP,
    VkKeyScanW,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Modifier {
    Ctrl,
    Shift,
    Alt,
    Win,
}

impl Modifier {
    pub fn vk(self) -> VIRTUAL_KEY {
        match self {
            Modifier::Ctrl => VK_CONTROL,
            Modifier::Shift => VK_SHIFT,
            Modifier::Alt => VK_MENU,
            Modifier::Win => VK_LWIN,
        }
    }

    pub fn parse(name: &str) -> Option<Modifier> {
        match name.to_ascii_lowercase().as_str() {
            "ctrl" | "control" => Some(Modifier::Ctrl),
            "shift" => Some(Modifier::Shift),
            "alt" | "option" => Some(Modifier::Alt),
            "win" | "windows" | "meta" | "super" | "cmd" => Some(Modifier::Win),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Combo {
    /// In press order; released in reverse.
    pub modifiers: Vec<Modifier>,
    pub key: VIRTUAL_KEY,
}

/// Parses "ctrl+shift+t". The last part is the key: a name ("enter", "f5", "pagedown") or a single
/// character, which may add Shift (e.g. "?" on a US layout) per the current keyboard layout.
pub fn parse(spec: &str) -> Result<Combo, String> {
    let parts: Vec<&str> = spec.split('+').map(str::trim).filter(|p| !p.is_empty()).collect();
    let Some((last, rest)) = parts.split_last() else {
        return Err("no key given".into());
    };
    let mut modifiers = rest
        .iter()
        .map(|name| Modifier::parse(name).ok_or_else(|| format!("unknown modifier \"{name}\"")))
        .collect::<Result<Vec<_>, _>>()?;
    let key = match named(last) {
        Some(vk) => vk,
        None => {
            let mut chars = last.chars();
            let (Some(c), None) = (chars.next(), chars.next()) else {
                return Err(format!("unknown key \"{last}\""));
            };
            let (vk, needs_shift) = char_key(c).ok_or_else(|| format!("\"{c}\" has no key on this keyboard layout"))?;
            if needs_shift && !modifiers.contains(&Modifier::Shift) {
                modifiers.push(Modifier::Shift);
            }
            vk
        }
    };
    Ok(Combo { modifiers, key })
}

fn named(name: &str) -> Option<VIRTUAL_KEY> {
    let lower = name.to_ascii_lowercase();
    let vk = match lower.as_str() {
        "enter" | "return" => VK_RETURN,
        "tab" => VK_TAB,
        "esc" | "escape" => VK_ESCAPE,
        "space" => VK_SPACE,
        "backspace" => VK_BACK,
        "delete" | "del" => VK_DELETE,
        "insert" | "ins" => VK_INSERT,
        "home" => VK_HOME,
        "end" => VK_END,
        "pageup" | "pgup" => VK_PRIOR,
        "pagedown" | "pgdn" => VK_NEXT,
        "up" => VK_UP,
        "down" => VK_DOWN,
        "left" => VK_LEFT,
        "right" => VK_RIGHT,
        "capslock" => VK_CAPITAL,
        "printscreen" => VK_SNAPSHOT,
        "plus" => return char_key('+').map(|(vk, _)| vk),
        _ => {
            let n: u16 = lower.strip_prefix('f')?.parse().ok().filter(|n| (1..=24).contains(n))?;
            VIRTUAL_KEY(VK_F1.0 + n - 1)
        }
    };
    Some(vk)
}

/// Virtual key for a character on the current layout, and whether it needs Shift.
fn char_key(c: char) -> Option<(VIRTUAL_KEY, bool)> {
    let mut units = [0u16; 2];
    let [unit] = c.encode_utf16(&mut units) else { return None };
    let scan = unsafe { VkKeyScanW(*unit) };
    if scan == -1 {
        return None;
    }
    let vk = (scan & 0xff) as u16;
    let shift = (scan >> 8) & 1 != 0;
    // Letters come back as their key; the combo means the key, not the capital letter.
    let shift = shift && !c.is_ascii_uppercase();
    Some((VIRTUAL_KEY(vk), shift))
}

/// Keys whose scan codes carry the "extended" flag (the cursor block, right-hand modifiers, ...).
pub fn is_extended(vk: VIRTUAL_KEY) -> bool {
    matches!(
        vk,
        VK_INSERT | VK_DELETE | VK_HOME | VK_END | VK_PRIOR | VK_NEXT | VK_UP | VK_DOWN | VK_LEFT | VK_RIGHT | VK_LWIN
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_combos() {
        assert_eq!(parse("ctrl+s").unwrap(), Combo { modifiers: vec![Modifier::Ctrl], key: VIRTUAL_KEY(b'S' as u16) });
        assert_eq!(parse("Alt+F4").unwrap(), Combo { modifiers: vec![Modifier::Alt], key: VIRTUAL_KEY(VK_F1.0 + 3) });
        assert_eq!(parse("enter").unwrap(), Combo { modifiers: vec![], key: VK_RETURN });
        assert_eq!(parse("ctrl + shift + t").unwrap().modifiers, vec![Modifier::Ctrl, Modifier::Shift]);
        assert_eq!(parse("ctrl+plus").unwrap().modifiers, vec![Modifier::Ctrl]);
        assert!(parse("hyper+x").is_err());
        assert!(parse("").is_err());
        assert!(parse("f25").is_err());
    }
}
