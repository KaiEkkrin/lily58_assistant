//! `--probe`: prints what the app can see, using only guarded read commands.

use std::io::{self, Write};
use std::path::Path;

use crate::hid::{discover, hidraw};
use crate::input::evdev::find_event_nodes;
use crate::keycodes;
use crate::keymap::Keymap;
use crate::layout::Layout;
use crate::vial::{self, VialClient};

pub fn run(out: &mut impl Write, dump_definition: Option<&Path>) -> anyhow::Result<()> {
    let (sys, dev_root) = (Path::new("/sys"), Path::new("/dev"));
    let Some(dev) = discover::find_vial_device(sys, dev_root)? else {
        writeln!(out, "No Vial keyboard found (looked for a USB serial containing {}).", discover::VIAL_SERIAL_MARKER)?;
        return Ok(());
    };
    writeln!(out, "Device:      {} ({:04x}:{:04x}) at {}", dev.product, dev.vendor_id, dev.product_id, dev.dev_node.display())?;
    let holders = discover::other_holders(Path::new("/proc"), &dev.dev_node, std::process::id());
    if !holders.is_empty() {
        writeln!(out, "WARNING:     also open in {}; close it for reliable results", holders.join(", "))?;
    }
    let guard = match hidraw::open_guarded(&dev.dev_node) {
        Ok(guard) => guard,
        Err(e) if e.kind() == io::ErrorKind::PermissionDenied => {
            writeln!(out, "Raw HID:     NO ACCESS ({e}); see README, \"Permissions\"")?;
            return Ok(());
        }
        Err(e) => return Err(e.into()),
    };
    writeln!(out, "Raw HID:     accessible")?;

    let mut client = VialClient::new(guard);
    let id = client.keyboard_id()?;
    writeln!(out, "Vial:        protocol {} (keyboard uid {:016x})", id.vial_protocol, id.uid)?;
    vial::check_protocol(&id)?;
    let status = client.unlock_status()?;
    let lock = match (status.unlocked, status.in_progress) {
        (true, _) => "unlocked (live layer tracking available)",
        (false, true) => "UNLOCK IN PROGRESS: VIA reads are blocked until it completes or the keyboard is replugged",
        (false, false) => "locked",
    };
    writeln!(out, "Unlock:      {lock}; unlock keys {:?}", status.keys)?;
    if status.in_progress && !status.unlocked {
        return Ok(());
    }
    writeln!(out, "VIA:         protocol {:#06x}", client.via_protocol_version()?)?;

    let definition = client.definition()?;
    if let Some(path) = dump_definition {
        std::fs::write(path, serde_json::to_string_pretty(&definition)? + "\n")?;
        writeln!(out, "Definition:  written to {}", path.display())?;
    }
    let layout = Layout::from_definition(&definition)?;
    writeln!(out, "Layout:      {}: {} keys, matrix {}x{}", layout.name, layout.keys.len(), layout.rows, layout.cols)?;

    let layers = client.layer_count()?;
    let buf = client.keymap_buffer(Keymap::buffer_len(layers, layout.rows, layout.cols))?;
    let keymap = Keymap::from_buffer(layers, layout.rows, layout.cols, &buf)?;
    for layer in 0..layers {
        writeln!(out, "Layer {layer}:")?;
        for row in 0..layout.rows {
            let cells: Vec<String> =
                (0..layout.cols).map(|col| format!("{:>14}", keycodes::label(keymap.get(layer, row, col)))).collect();
            writeln!(out, "  row {row}:{}", cells.concat())?;
        }
    }

    let mut denied = false;
    for node in find_event_nodes(sys, dev_root, &dev.usb_dir) {
        let access = match std::fs::File::open(&node) {
            Ok(_) => "readable".to_string(),
            Err(e) if e.kind() == io::ErrorKind::PermissionDenied => {
                denied = true;
                "no access".to_string()
            }
            Err(e) => format!("error: {e}"),
        };
        writeln!(out, "Input node:  {} {access}", node.display())?;
    }
    let all_windows = if denied { "unavailable (optional udev rule not installed)" } else { "available" };
    writeln!(out, "All windows: {all_windows}")?;
    Ok(())
}
