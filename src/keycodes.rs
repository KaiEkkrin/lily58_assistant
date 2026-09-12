//! QMK keycodes as stored by Vial protocol v6+ (vial-qmk `quantum/keycodes.h`).

pub const KC_NO: u16 = 0x0000;
pub const KC_TRNS: u16 = 0x0001;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Action {
    Basic(u8),
    Modded { mods: u8, basic: u8 },
    ModTap { mods: u8, basic: u8 },
    LayerTap { layer: u8, basic: u8 },
    LayerMod { layer: u8, mods: u8 },
    To(u8),
    Momentary(u8),
    DefaultLayer(u8),
    Toggle(u8),
    OneShotLayer(u8),
    OneShotMod(u8),
    TapToggle(u8),
    PersistentDefault(u8),
    TriLayerLower,
    TriLayerUpper,
    Other(u16),
}

pub fn decode(code: u16) -> Action {
    let low = (code & 0xFF) as u8;
    let low5 = (code & 0x1F) as u8;
    let mods8 = ((code >> 8) & 0x1F) as u8;
    match code {
        0x0000..=0x00FF => Action::Basic(low),
        0x0100..=0x1FFF => Action::Modded { mods: mods8, basic: low },
        0x2000..=0x3FFF => Action::ModTap { mods: mods8, basic: low },
        0x4000..=0x4FFF => Action::LayerTap { layer: ((code >> 8) & 0x0F) as u8, basic: low },
        0x5000..=0x51FF => Action::LayerMod { layer: ((code >> 5) & 0x0F) as u8, mods: low5 },
        0x5200..=0x521F => Action::To(low5),
        0x5220..=0x523F => Action::Momentary(low5),
        0x5240..=0x525F => Action::DefaultLayer(low5),
        0x5260..=0x527F => Action::Toggle(low5),
        0x5280..=0x529F => Action::OneShotLayer(low5),
        0x52A0..=0x52BF => Action::OneShotMod(low5),
        0x52C0..=0x52DF => Action::TapToggle(low5),
        0x52E0..=0x52FF => Action::PersistentDefault(low5),
        0x7C77 => Action::TriLayerLower,
        0x7C78 => Action::TriLayerUpper,
        _ => Action::Other(code),
    }
}

/// QMK-style name, e.g. `KC_A`, `LCTL(KC_C)`, `LT(2,KC_SPC)`, `MO(1)`; unknown codes as hex.
pub fn label(code: u16) -> String {
    match decode(code) {
        Action::Basic(b) => basic_name(b).map(str::to_owned).unwrap_or_else(|| format!("{code:#06x}")),
        Action::Modded { mods, basic } => format!("{}({})", mods_name(mods), label(basic as u16)),
        Action::ModTap { mods, basic } => format!("MT({},{})", mods_name(mods), label(basic as u16)),
        Action::LayerTap { layer, basic } => format!("LT({layer},{})", label(basic as u16)),
        Action::LayerMod { layer, mods } => format!("LM({layer},{})", mods_name(mods)),
        Action::To(l) => format!("TO({l})"),
        Action::Momentary(l) => format!("MO({l})"),
        Action::DefaultLayer(l) => format!("DF({l})"),
        Action::Toggle(l) => format!("TG({l})"),
        Action::OneShotLayer(l) => format!("OSL({l})"),
        Action::OneShotMod(m) => format!("OSM({})", mods_name(m)),
        Action::TapToggle(l) => format!("TT({l})"),
        Action::PersistentDefault(l) => format!("PDF({l})"),
        Action::TriLayerLower => "TL_LOWR".into(),
        Action::TriLayerUpper => "TL_UPPR".into(),
        Action::Other(c) => format!("{c:#06x}"),
    }
}

/// 5-bit QMK modifier mask (bit 4 = right-hand) as `LCTL|LSFT` etc.
pub fn mods_name(mods: u8) -> String {
    let side = if mods & 0x10 != 0 { 'R' } else { 'L' };
    let parts: Vec<String> = [(0x01, "CTL"), (0x02, "SFT"), (0x04, "ALT"), (0x08, "GUI")]
        .into_iter()
        .filter(|(bit, _)| mods & bit != 0)
        .map(|(_, name)| format!("{side}{name}"))
        .collect();
    if parts.is_empty() { "NONE".into() } else { parts.join("|") }
}

/// The HID usage a key sends when tapped, if it sends one.
pub fn tap_basic(code: u16) -> Option<u8> {
    match decode(code) {
        Action::Basic(b)
        | Action::Modded { basic: b, .. }
        | Action::ModTap { basic: b, .. }
        | Action::LayerTap { basic: b, .. }
            if b >= 0x04 =>
        {
            Some(b)
        }
        _ => None,
    }
}

/// True if the keycode itself applies Shift (e.g. `KC_EXLM` = `LSFT(KC_1)`).
pub fn adds_shift(code: u16) -> bool {
    matches!(decode(code), Action::Modded { mods, .. } if mods & 0x02 != 0)
}

pub fn basic_name(code: u8) -> Option<&'static str> {
    BASIC_NAMES.iter().find(|(c, _)| *c == code).map(|(_, name)| *name)
}

/// Short QMK names for the basic range, generated from vial-qmk `quantum/keycodes.h`.
const BASIC_NAMES: &[(u8, &str)] = &[
    (0x00, "KC_NO"), (0x01, "KC_TRNS"), (0x04, "KC_A"), (0x05, "KC_B"), (0x06, "KC_C"), (0x07, "KC_D"),
    (0x08, "KC_E"), (0x09, "KC_F"), (0x0A, "KC_G"), (0x0B, "KC_H"), (0x0C, "KC_I"), (0x0D, "KC_J"),
    (0x0E, "KC_K"), (0x0F, "KC_L"), (0x10, "KC_M"), (0x11, "KC_N"), (0x12, "KC_O"), (0x13, "KC_P"),
    (0x14, "KC_Q"), (0x15, "KC_R"), (0x16, "KC_S"), (0x17, "KC_T"), (0x18, "KC_U"), (0x19, "KC_V"),
    (0x1A, "KC_W"), (0x1B, "KC_X"), (0x1C, "KC_Y"), (0x1D, "KC_Z"), (0x1E, "KC_1"), (0x1F, "KC_2"),
    (0x20, "KC_3"), (0x21, "KC_4"), (0x22, "KC_5"), (0x23, "KC_6"), (0x24, "KC_7"), (0x25, "KC_8"),
    (0x26, "KC_9"), (0x27, "KC_0"), (0x28, "KC_ENT"), (0x29, "KC_ESC"), (0x2A, "KC_BSPC"), (0x2B, "KC_TAB"),
    (0x2C, "KC_SPC"), (0x2D, "KC_MINS"), (0x2E, "KC_EQL"), (0x2F, "KC_LBRC"), (0x30, "KC_RBRC"),
    (0x31, "KC_BSLS"), (0x32, "KC_NUHS"), (0x33, "KC_SCLN"), (0x34, "KC_QUOT"), (0x35, "KC_GRV"),
    (0x36, "KC_COMM"), (0x37, "KC_DOT"), (0x38, "KC_SLSH"), (0x39, "KC_CAPS"), (0x3A, "KC_F1"),
    (0x3B, "KC_F2"), (0x3C, "KC_F3"), (0x3D, "KC_F4"), (0x3E, "KC_F5"), (0x3F, "KC_F6"), (0x40, "KC_F7"),
    (0x41, "KC_F8"), (0x42, "KC_F9"), (0x43, "KC_F10"), (0x44, "KC_F11"), (0x45, "KC_F12"),
    (0x46, "KC_PSCR"), (0x47, "KC_SCRL"), (0x48, "KC_PAUS"), (0x49, "KC_INS"), (0x4A, "KC_HOME"),
    (0x4B, "KC_PGUP"), (0x4C, "KC_DEL"), (0x4D, "KC_END"), (0x4E, "KC_PGDN"), (0x4F, "KC_RGHT"),
    (0x50, "KC_LEFT"), (0x51, "KC_DOWN"), (0x52, "KC_UP"), (0x53, "KC_NUM"), (0x54, "KC_PSLS"),
    (0x55, "KC_PAST"), (0x56, "KC_PMNS"), (0x57, "KC_PPLS"), (0x58, "KC_PENT"), (0x59, "KC_P1"),
    (0x5A, "KC_P2"), (0x5B, "KC_P3"), (0x5C, "KC_P4"), (0x5D, "KC_P5"), (0x5E, "KC_P6"), (0x5F, "KC_P7"),
    (0x60, "KC_P8"), (0x61, "KC_P9"), (0x62, "KC_P0"), (0x63, "KC_PDOT"), (0x64, "KC_NUBS"),
    (0x65, "KC_APP"), (0x66, "KC_KB_POWER"), (0x67, "KC_PEQL"), (0x68, "KC_F13"), (0x69, "KC_F14"),
    (0x6A, "KC_F15"), (0x6B, "KC_F16"), (0x6C, "KC_F17"), (0x6D, "KC_F18"), (0x6E, "KC_F19"),
    (0x6F, "KC_F20"), (0x70, "KC_F21"), (0x71, "KC_F22"), (0x72, "KC_F23"), (0x73, "KC_F24"),
    (0x74, "KC_EXEC"), (0x75, "KC_HELP"), (0x76, "KC_MENU"), (0x77, "KC_SLCT"), (0x78, "KC_STOP"),
    (0x79, "KC_AGIN"), (0x7A, "KC_UNDO"), (0x7B, "KC_CUT"), (0x7C, "KC_COPY"), (0x7D, "KC_PSTE"),
    (0x7E, "KC_FIND"), (0x7F, "KC_KB_MUTE"), (0x80, "KC_KB_VOLUME_UP"), (0x81, "KC_KB_VOLUME_DOWN"),
    (0x85, "KC_PCMM"), (0x87, "KC_INT1"), (0x88, "KC_INT2"), (0x89, "KC_INT3"), (0x8A, "KC_INT4"),
    (0x8B, "KC_INT5"), (0x8C, "KC_INT6"), (0x8D, "KC_INT7"), (0x8E, "KC_INT8"), (0x8F, "KC_INT9"),
    (0x90, "KC_LNG1"), (0x91, "KC_LNG2"), (0x92, "KC_LNG3"), (0x93, "KC_LNG4"), (0x94, "KC_LNG5"),
    (0x95, "KC_LNG6"), (0x96, "KC_LNG7"), (0x97, "KC_LNG8"), (0x98, "KC_LNG9"), (0x99, "KC_ERAS"),
    (0x9A, "KC_SYRQ"), (0x9B, "KC_CNCL"), (0x9C, "KC_CLR"), (0x9D, "KC_PRIR"), (0x9E, "KC_RETN"),
    (0x9F, "KC_SEPR"), (0xA0, "KC_OUT"), (0xA1, "KC_OPER"), (0xA2, "KC_CLAG"), (0xA3, "KC_CRSL"),
    (0xA4, "KC_EXSL"), (0xA5, "KC_PWR"), (0xA6, "KC_SLEP"), (0xA7, "KC_WAKE"), (0xA8, "KC_MUTE"),
    (0xA9, "KC_VOLU"), (0xAA, "KC_VOLD"), (0xAB, "KC_MNXT"), (0xAC, "KC_MPRV"), (0xAD, "KC_MSTP"),
    (0xAE, "KC_MPLY"), (0xAF, "KC_MSEL"), (0xB0, "KC_EJCT"), (0xB1, "KC_MAIL"), (0xB2, "KC_CALC"),
    (0xB3, "KC_MYCM"), (0xB4, "KC_WSCH"), (0xB5, "KC_WHOM"), (0xB6, "KC_WBAK"), (0xB7, "KC_WFWD"),
    (0xB8, "KC_WSTP"), (0xB9, "KC_WREF"), (0xBA, "KC_WFAV"), (0xBB, "KC_MFFD"), (0xBC, "KC_MRWD"),
    (0xBD, "KC_BRIU"), (0xBE, "KC_BRID"), (0xBF, "KC_CPNL"), (0xC0, "KC_ASST"), (0xC1, "KC_MCTL"),
    (0xC2, "KC_LPAD"), (0xCD, "MS_UP"), (0xCE, "MS_DOWN"), (0xCF, "MS_LEFT"), (0xD0, "MS_RGHT"),
    (0xD1, "MS_BTN1"), (0xD2, "MS_BTN2"), (0xD3, "MS_BTN3"), (0xD4, "MS_BTN4"), (0xD5, "MS_BTN5"),
    (0xD6, "MS_BTN6"), (0xD7, "MS_BTN7"), (0xD8, "MS_BTN8"), (0xD9, "MS_WHLU"), (0xDA, "MS_WHLD"),
    (0xDB, "MS_WHLL"), (0xDC, "MS_WHLR"), (0xDD, "MS_ACL0"), (0xDE, "MS_ACL1"), (0xDF, "MS_ACL2"),
    (0xE0, "KC_LCTL"), (0xE1, "KC_LSFT"), (0xE2, "KC_LALT"), (0xE3, "KC_LGUI"), (0xE4, "KC_RCTL"),
    (0xE5, "KC_RSFT"), (0xE6, "KC_RALT"), (0xE7, "KC_RGUI"),
];

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn decodes_qmk_ranges() {
        let cases: [(u16, Action); 15] = [
            (0x0004, Action::Basic(0x04)),                              // KC_A
            (0x0106, Action::Modded { mods: 0x01, basic: 0x06 }),       // LCTL(KC_C)
            (0x121E, Action::Modded { mods: 0x12, basic: 0x1E }),       // RSFT(KC_1)
            (0x2204, Action::ModTap { mods: 0x02, basic: 0x04 }),       // MT(MOD_LSFT, KC_A)
            (0x422C, Action::LayerTap { layer: 2, basic: 0x2C }),       // LT(2, KC_SPC)
            (0x5021, Action::LayerMod { layer: 1, mods: 0x01 }),        // LM(1, MOD_LCTL)
            (0x5203, Action::To(3)),
            (0x5221, Action::Momentary(1)),
            (0x5240, Action::DefaultLayer(0)),
            (0x5262, Action::Toggle(2)),
            (0x5281, Action::OneShotLayer(1)),
            (0x52A2, Action::OneShotMod(0x02)),
            (0x52C1, Action::TapToggle(1)),
            (0x7C77, Action::TriLayerLower),
            (0x7C00, Action::Other(0x7C00)),                            // QK_BOOT
        ];
        for (code, action) in cases {
            assert_eq!(decode(code), action, "{code:#06x}");
        }
    }

    #[test]
    fn labels() {
        assert_eq!(label(0x0004), "KC_A");
        assert_eq!(label(0x0001), "KC_TRNS");
        assert_eq!(label(0x0106), "LCTL(KC_C)");
        assert_eq!(label(0x121E), "RSFT(KC_1)");
        assert_eq!(label(0x2204), "MT(LSFT,KC_A)");
        assert_eq!(label(0x422C), "LT(2,KC_SPC)");
        assert_eq!(label(0x5221), "MO(1)");
        assert_eq!(label(0x7C78), "TL_UPPR");
        assert_eq!(label(0x7C00), "0x7c00");
        assert_eq!(mods_name(0x03), "LCTL|LSFT");
    }

    #[test]
    fn tap_basic_and_shift() {
        assert_eq!(tap_basic(0x0004), Some(0x04));
        assert_eq!(tap_basic(0x021E), Some(0x1E)); // KC_EXLM = LSFT(KC_1)
        assert_eq!(tap_basic(0x422C), Some(0x2C));
        assert_eq!(tap_basic(KC_NO), None);
        assert_eq!(tap_basic(KC_TRNS), None);
        assert_eq!(tap_basic(0x5221), None);
        assert!(adds_shift(0x021E));
        assert!(!adds_shift(0x0106));
    }
}
