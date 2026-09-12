//! Setup instructions shown in the app. README.md repeats them verbatim (checked by a test).

pub const HIDRAW_RULE_FILE: &str = "/etc/udev/rules.d/59-vial.rules";
/// Vial's standard rule: raw-HID access to Vial keyboards for the logged-in user.
pub const HIDRAW_RULE: &str = r#"KERNEL=="hidraw*", SUBSYSTEM=="hidraw", ATTRS{serial}=="*vial:f64c2b3c*", MODE="0660", GROUP="users", TAG+="uaccess", TAG+="udev-acl""#;
/// Must sort before systemd's 73-seat-late.rules for `uaccess` to apply.
pub const EVDEV_RULE_FILE: &str = "/etc/udev/rules.d/70-lily58-assistant.rules";
pub const EVDEV_RULE: &str = r#"SUBSYSTEM=="input", KERNEL=="event*", ATTRS{idVendor}=="7171", ATTRS{idProduct}=="0012", TAG+="uaccess""#;
pub const RELOAD_COMMAND: &str = "sudo udevadm control --reload-rules && sudo udevadm trigger";

pub struct Hint {
    pub title: &'static str,
    pub explanation: &'static str,
    pub commands: Vec<String>,
}

pub fn install_command(file: &str, rule: &str) -> String {
    format!("echo '{rule}' | sudo tee {file}")
}

pub fn hidraw_hint() -> Hint {
    Hint {
        title: "Allow access to the keyboard",
        explanation: "The assistant reads the layout and keymap over the keyboard's raw-HID interface \
                      (read-only). Vial needs the same rule, so if Vial works on this PC you already have it.",
        commands: vec![install_command(HIDRAW_RULE_FILE, HIDRAW_RULE), RELOAD_COMMAND.to_owned()],
    }
}

pub fn evdev_hint() -> Hint {
    Hint {
        title: "Track keys typed into other windows",
        explanation: "Lets the assistant read the Lily58's own input device, so highlights keep working while \
                      another window has focus. Trade-off: any program running as you can then read keystrokes \
                      from this keyboard.",
        commands: vec![install_command(EVDEV_RULE_FILE, EVDEV_RULE), RELOAD_COMMAND.to_owned()],
    }
}

pub fn unlock_hint() -> Hint {
    Hint {
        title: "Unlock the keyboard for live layer tracking",
        explanation: "Vial only reports the switch matrix after its physical unlock. Click the button below, then \
                      hold the highlighted keys for about 10 seconds. The keyboard forgets the unlock when unplugged. \
                      An unlock can't be cancelled once started; unplug the keyboard to abort.",
        commands: Vec::new(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn readme_contains_every_setup_command_verbatim() {
        let readme = include_str!("../README.md");
        for hint in [hidraw_hint(), evdev_hint()] {
            assert!(!hint.commands.is_empty());
            for cmd in &hint.commands {
                assert!(readme.contains(cmd.as_str()), "README.md is missing:\n{cmd}");
            }
        }
    }

    #[test]
    fn install_command_quotes_the_rule() {
        assert_eq!(install_command("/etc/x.rules", "A==\"b\""), "echo 'A==\"b\"' | sudo tee /etc/x.rules");
    }
}
