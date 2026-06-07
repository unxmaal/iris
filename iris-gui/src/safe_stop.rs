use crate::handle::Status;
use iris::config::MachineConfig;

/// Reasons it is risky to force-stop right now.
#[derive(Debug, Clone, Default)]
pub struct UnsafeReasons {
    /// SCSI IDs of attached disks whose guest writes land directly in their
    /// base image (a plain read-write HDD). Force-stopping mid-write can leave
    /// the filesystem on these inconsistent.
    pub writable_disks: Vec<u8>,
}

impl UnsafeReasons {
    pub fn is_empty(&self) -> bool {
        self.writable_disks.is_empty()
    }
}

/// Evaluate whether stopping the emulator right now is safe.
///
/// The core does not expose live dirty-sector state, so we decide purely from
/// config: an abrupt power-off only risks the on-disk image when some attached
/// device persists guest writes straight into its **base image** — i.e. a
/// plain read-write hard disk. Everything else leaves the base image untouched
/// and is safe to power off without warning:
///
/// - **CD-ROM** — read-only.
/// - **COW overlay** (`overlay = true`) — writes go to a `{path}.overlay`
///   sidecar; the base image is never modified (delete the overlay to reset).
/// - **Scratch volume** (`scratch = true`) — a transient host-side file, not a
///   guest filesystem we need to protect.
/// - **CHD** (`*.chd`) — writes go to a `.diff.chd` sidecar; the base CHD is
///   never modified.
///
/// So when no attached device writes through to its base image, powering off
/// will NOT damage the hard disk and we skip the confirmation dialog entirely.
pub fn evaluate(_status: &Status, cfg: &MachineConfig) -> UnsafeReasons {
    let mut r = UnsafeReasons::default();
    for (id, dev) in &cfg.scsi {
        let persists_to_base = !dev.cdrom
            && !dev.overlay
            && !dev.scratch
            && !dev.path.ends_with(".chd");
        if persists_to_base {
            r.writable_disks.push(*id);
        }
    }
    r.writable_disks.sort();
    r
}

/// Human-readable lines for the confirmation dialog.
pub fn reason_lines(r: &UnsafeReasons) -> Vec<String> {
    r.writable_disks
        .iter()
        .map(|id| {
            format!(
                "scsi{id} is a read-write disk image — force-stopping while IRIX \
                 is running can corrupt its filesystem."
            )
        })
        .collect()
}
