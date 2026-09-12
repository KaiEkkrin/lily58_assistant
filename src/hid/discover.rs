//! Finds the keyboard's Vial raw-HID node through sysfs.

use std::fs;
use std::io;
use std::path::{Path, PathBuf};

/// Vial firmware puts this in the USB serial number.
pub const VIAL_SERIAL_MARKER: &str = "vial:f64c2b3c";
const RAW_USAGE_PAGE: u32 = 0xFF60;
const RAW_USAGE: u32 = 0x61;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VialDevice {
    /// `/dev/hidrawN`
    pub dev_node: PathBuf,
    /// Canonical sysfs directory of the USB device (the one holding `idVendor` and `serial`).
    pub usb_dir: PathBuf,
    pub vendor_id: u16,
    pub product_id: u16,
    pub product: String,
}

/// First Vial raw-HID interface. `sys_root` is normally `/sys`, `dev_root` `/dev`.
/// A missing `class/hidraw` means no device. Any other error is passed on on purpose: the
/// worker reports it once ("scanning for the keyboard failed"), which beats waiting silently.
pub fn find_vial_device(sys_root: &Path, dev_root: &Path) -> io::Result<Option<VialDevice>> {
    let class = sys_root.join("class/hidraw");
    let mut names: Vec<_> = match fs::read_dir(&class) {
        Ok(entries) => entries.filter_map(|e| e.ok()).map(|e| e.file_name()).collect(),
        Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(None),
        Err(e) => return Err(e),
    };
    names.sort();
    for name in names {
        let Ok(hid_dir) = fs::canonicalize(class.join(&name).join("device")) else { continue };
        let Ok(descriptor) = fs::read(hid_dir.join("report_descriptor")) else { continue };
        if !is_vial_raw_interface(&descriptor) {
            continue;
        }
        let Some(usb_dir) = usb_device_dir(&hid_dir) else { continue };
        if !read_trimmed(&usb_dir.join("serial")).unwrap_or_default().contains(VIAL_SERIAL_MARKER) {
            continue;
        }
        return Ok(Some(VialDevice {
            dev_node: dev_root.join(&name),
            vendor_id: read_hex(&usb_dir.join("idVendor")).unwrap_or(0),
            product_id: read_hex(&usb_dir.join("idProduct")).unwrap_or(0),
            product: read_trimmed(&usb_dir.join("product")).unwrap_or_default(),
            usb_dir,
        }));
    }
    Ok(None)
}

/// Walks up from a sysfs device path to the USB device directory (the one with `idVendor`).
pub fn usb_device_dir(path: &Path) -> Option<PathBuf> {
    path.ancestors().find(|p| p.join("idVendor").is_file()).map(Path::to_path_buf)
}

/// True if a HID report descriptor declares usage page 0xFF60 with usage 0x61 (QMK raw HID).
pub fn is_vial_raw_interface(descriptor: &[u8]) -> bool {
    let (mut page, mut saw_page, mut saw_usage) = (0u32, false, false);
    let mut i = 0;
    while i < descriptor.len() {
        let prefix = descriptor[i];
        if prefix == 0xFE {
            // Long item: 0xFE, data size, tag, data.
            i += 3 + *descriptor.get(i + 1).unwrap_or(&0) as usize;
            continue;
        }
        let size = match prefix & 0x03 {
            3 => 4,
            n => n as usize,
        };
        let data = descriptor.get(i + 1..i + 1 + size).unwrap_or(&[]);
        let value = data.iter().rev().fold(0u32, |acc, &b| (acc << 8) | b as u32);
        match prefix & 0xFC {
            0x04 => {
                // Usage Page (global item)
                page = value;
                saw_page |= page == RAW_USAGE_PAGE;
            }
            0x08 => saw_usage |= page == RAW_USAGE_PAGE && value == RAW_USAGE, // Usage (local item)
            _ => {}
        }
        i += 1 + size;
    }
    saw_page && saw_usage
}

/// Processes other than `own_pid` that hold `dev_node` open, as `"name (pid)"`.
/// Processes of other users are unreadable and silently skipped.
pub fn other_holders(proc_root: &Path, dev_node: &Path, own_pid: u32) -> Vec<String> {
    let mut found = Vec::new();
    let Ok(procs) = fs::read_dir(proc_root) else { return found };
    for entry in procs.flatten() {
        let Some(pid) = entry.file_name().to_str().and_then(|s| s.parse::<u32>().ok()) else { continue };
        if pid == own_pid {
            continue;
        }
        let Ok(fds) = fs::read_dir(entry.path().join("fd")) else { continue };
        if fds.flatten().any(|fd| fs::read_link(fd.path()).is_ok_and(|target| target == dev_node)) {
            let name = read_trimmed(&entry.path().join("comm")).unwrap_or_else(|| "?".into());
            found.push(format!("{name} ({pid})"));
        }
    }
    found.sort();
    found
}

fn read_trimmed(path: &Path) -> Option<String> {
    fs::read_to_string(path).ok().map(|s| s.trim().to_owned())
}

fn read_hex(path: &Path) -> Option<u16> {
    u16::from_str_radix(&read_trimmed(path)?, 16).ok()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::os::unix::fs::symlink;

    // Real descriptors read from the Lily58 (sysfs report_descriptor of hidraw15 / hidraw14).
    const RAW_DESCRIPTOR: [u8; 34] = [
        0x06, 0x60, 0xff, 0x09, 0x61, 0xa1, 0x01, 0x09, 0x62, 0x15, 0x00, 0x26, 0xff, 0x00, 0x95, 0x20, 0x75,
        0x08, 0x81, 0x02, 0x09, 0x63, 0x15, 0x00, 0x26, 0xff, 0x00, 0x95, 0x20, 0x75, 0x08, 0x91, 0x02, 0xc0,
    ];
    const KEYBOARD_DESCRIPTOR: [u8; 7] = [0x05, 0x01, 0x09, 0x06, 0xa1, 0x01, 0xc0];

    fn fake_sys(serial: &str) -> tempfile::TempDir {
        let t = tempfile::tempdir().unwrap();
        let root = t.path();
        let usb = root.join("devices/usb1/1-3");
        let hid_kbd = usb.join("1-3:1.0/0003:7171:0012.0001");
        let hid_raw = usb.join("1-3:1.1/0003:7171:0012.0002");
        fs::create_dir_all(&hid_kbd).unwrap();
        fs::create_dir_all(&hid_raw).unwrap();
        fs::write(usb.join("idVendor"), "7171\n").unwrap();
        fs::write(usb.join("idProduct"), "0012\n").unwrap();
        fs::write(usb.join("serial"), format!("{serial}\n")).unwrap();
        fs::write(usb.join("product"), "Lily58 Pro R2G\n").unwrap();
        fs::write(hid_kbd.join("report_descriptor"), KEYBOARD_DESCRIPTOR).unwrap();
        fs::write(hid_raw.join("report_descriptor"), RAW_DESCRIPTOR).unwrap();
        for (name, target) in [("hidraw14", &hid_kbd), ("hidraw15", &hid_raw)] {
            let dir = root.join("class/hidraw").join(name);
            fs::create_dir_all(&dir).unwrap();
            symlink(target, dir.join("device")).unwrap();
        }
        t
    }

    #[test]
    fn finds_the_raw_interface_not_the_keyboard_interface() {
        let sys = fake_sys("vial:f64c2b3c");
        let dev = find_vial_device(sys.path(), Path::new("/dev")).unwrap().unwrap();
        assert_eq!(dev.dev_node, PathBuf::from("/dev/hidraw15"));
        assert_eq!((dev.vendor_id, dev.product_id), (0x7171, 0x0012));
        assert_eq!(dev.product, "Lily58 Pro R2G");
        assert_eq!(dev.usb_dir, fs::canonicalize(sys.path().join("devices/usb1/1-3")).unwrap());
    }

    #[test]
    fn ignores_devices_without_the_vial_serial() {
        let sys = fake_sys("0123456789");
        assert_eq!(find_vial_device(sys.path(), Path::new("/dev")).unwrap(), None);
    }

    #[test]
    fn missing_hidraw_class_means_no_device() {
        let t = tempfile::tempdir().unwrap();
        assert_eq!(find_vial_device(t.path(), Path::new("/dev")).unwrap(), None);
    }

    #[test]
    fn descriptor_parsing() {
        assert!(is_vial_raw_interface(&RAW_DESCRIPTOR));
        assert!(!is_vial_raw_interface(&KEYBOARD_DESCRIPTOR));
        assert!(!is_vial_raw_interface(&[0x06, 0x60])); // truncated
    }

    #[test]
    fn unreadable_hidraw_class_is_an_error() {
        let t = tempfile::tempdir().unwrap();
        fs::create_dir_all(t.path().join("class")).unwrap();
        fs::write(t.path().join("class/hidraw"), "").unwrap(); // a file, so read_dir fails
        assert!(find_vial_device(t.path(), Path::new("/dev")).is_err());
    }

    #[test]
    fn finds_other_processes_holding_the_node() {
        let t = tempfile::tempdir().unwrap();
        let proc_root = t.path();
        for (pid, name, target) in [(100, "vial", "/dev/hidraw15"), (200, "me", "/dev/hidraw15"), (300, "bash", "/dev/null")] {
            let fd = proc_root.join(pid.to_string()).join("fd");
            fs::create_dir_all(&fd).unwrap();
            fs::write(proc_root.join(pid.to_string()).join("comm"), format!("{name}\n")).unwrap();
            symlink(target, fd.join("3")).unwrap();
        }
        let holders = other_holders(proc_root, Path::new("/dev/hidraw15"), 200);
        assert_eq!(holders, vec!["vial (100)".to_string()]);
    }
}
