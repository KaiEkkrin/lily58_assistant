//! Unfocused tier: reads the Lily58's own `/dev/input/eventN` nodes (needs the optional udev rule).

use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::sync::mpsc::Sender;
use std::thread;

use ::evdev::{Device, EventSummary};

use super::{InputMsg, OsKey, OsSource};
use crate::hid::discover::usb_device_dir;
use crate::hidmap::evdev_to_hid;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum EvdevStatus {
    Active,
    /// Nodes exist but none could be opened (no udev rule yet).
    NoAccess(Vec<PathBuf>),
    NotFound,
}

/// `/dev/input/eventN` nodes that belong to the USB device at `usb_dir` (canonical sysfs path).
pub fn find_event_nodes(sys_root: &Path, dev_root: &Path, usb_dir: &Path) -> Vec<PathBuf> {
    let Ok(entries) = fs::read_dir(sys_root.join("class/input")) else { return Vec::new() };
    let mut nodes: Vec<PathBuf> = entries
        .flatten()
        .filter(|e| e.file_name().to_string_lossy().starts_with("event"))
        .filter(|e| {
            fs::canonicalize(e.path().join("device")).ok().and_then(|p| usb_device_dir(&p)).as_deref() == Some(usb_dir)
        })
        .map(|e| dev_root.join("input").join(e.file_name()))
        .collect();
    nodes.sort();
    nodes
}

/// Opens every node and, only if none was denied, spawns one reader thread per node.
pub fn start(nodes: &[PathBuf], tx: Sender<InputMsg>, notify: impl Fn() + Send + Clone + 'static) -> EvdevStatus {
    let (mut opened, mut denied) = (Vec::new(), Vec::new());
    for node in nodes {
        match Device::open(node) {
            Ok(dev) => opened.push((node.clone(), dev)),
            Err(e) if e.kind() == io::ErrorKind::PermissionDenied => denied.push(node.clone()),
            Err(e) => log::warn!("cannot open {}: {e}", node.display()),
        }
    }
    let status = status_for(opened.len(), denied);
    if status == EvdevStatus::Active {
        for (node, dev) in opened {
            spawn_reader(node, dev, tx.clone(), notify.clone());
        }
    }
    status
}

/// Some of the keyboard's nodes can be readable without the udev rule (systemd gives the
/// seat user its joystick nodes), and those carry no key events. So tracking counts as
/// active only when no node was denied.
fn status_for(opened: usize, denied: Vec<PathBuf>) -> EvdevStatus {
    if !denied.is_empty() {
        EvdevStatus::NoAccess(denied)
    } else if opened > 0 {
        EvdevStatus::Active
    } else {
        EvdevStatus::NotFound
    }
}

pub fn key_msg(code: u16, pressed: bool) -> OsKey {
    OsKey { source: OsSource::Evdev, usages: evdev_to_hid(code), pressed, name: format!("evdev {code}") }
}

fn spawn_reader(node: PathBuf, mut dev: Device, tx: Sender<InputMsg>, notify: impl Fn() + Send + 'static) {
    let name = format!("evdev {}", node.display());
    let spawned = thread::Builder::new().name(name).spawn(move || {
        loop {
            let events = match dev.fetch_events() {
                Ok(events) => events,
                Err(e) => {
                    log::info!("{} closed: {e}", node.display());
                    let _ = tx.send(InputMsg::EvdevGone(node));
                    notify();
                    return;
                }
            };
            for ev in events {
                if let EventSummary::Key(_, code, value) = ev.destructure() {
                    if value == 2 {
                        continue; // auto-repeat
                    }
                    if tx.send(InputMsg::Key(key_msg(code.code(), value == 1))).is_err() {
                        return; // UI gone
                    }
                    notify();
                }
            }
        }
    });
    if let Err(e) = spawned {
        log::error!("cannot start evdev reader: {e}");
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::os::unix::fs::symlink;

    #[test]
    fn finds_only_event_nodes_of_the_same_usb_device() {
        let t = tempfile::tempdir().unwrap();
        let root = t.path();
        let ours = root.join("devices/usb1/1-3");
        let other = root.join("devices/usb1/1-4");
        for usb in [&ours, &other] {
            fs::create_dir_all(usb).unwrap();
            fs::write(usb.join("idVendor"), "7171\n").unwrap();
        }
        let links = [
            ("event5", ours.join("1-3:1.0/0003:7171:0012.0001/input/input7")),
            ("event6", other.join("1-4:1.0/0003:1234:5678.0003/input/input9")),
            ("mouse0", ours.join("1-3:1.2/0003:7171:0012.0003/input/input8")),
        ];
        for (name, input_dir) in &links {
            fs::create_dir_all(input_dir).unwrap();
            let class_dir = root.join("class/input").join(name);
            fs::create_dir_all(&class_dir).unwrap();
            symlink(input_dir, class_dir.join("device")).unwrap();
        }
        let usb_dir = fs::canonicalize(&ours).unwrap();
        assert_eq!(find_event_nodes(root, Path::new("/dev"), &usb_dir), vec![PathBuf::from("/dev/input/event5")]);
    }

    #[test]
    fn a_denied_node_means_no_access_even_if_another_opened() {
        // The Lily58's joystick node is world-readable (systemd's uaccess rule for joysticks),
        // but its keyboard node is not: that's no all-windows tracking.
        let kbd = PathBuf::from("/dev/input/event259");
        assert_eq!(status_for(1, vec![kbd.clone()]), EvdevStatus::NoAccess(vec![kbd]));
        assert_eq!(status_for(4, vec![]), EvdevStatus::Active);
        assert_eq!(status_for(0, vec![]), EvdevStatus::NotFound);
    }

    #[test]
    fn no_nodes_means_not_found() {
        let (tx, _rx) = std::sync::mpsc::channel();
        assert_eq!(start(&[], tx, || {}), EvdevStatus::NotFound);
    }

    #[test]
    fn key_messages_carry_hid_candidates() {
        let k = key_msg(43, true);
        assert_eq!(k.usages, vec![0x31, 0x32]);
        assert!(k.pressed);
        assert_eq!(k.source, OsSource::Evdev);
        assert_eq!(k.name, "evdev 43");
    }
}
