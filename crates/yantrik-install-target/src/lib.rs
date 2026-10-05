//! Installing Yantrik OS into one partition, or one run of free space, on a disk that keeps
//! everything else: a Mac whose APFS container was shrunk in macOS, a PC with Windows.
//!
//! The whole-disk install starts with `parted mklabel gpt`. Nothing here ever does. The rules:
//!
//! - **What can be chosen** ([`classify`]): an empty partition, a placeholder (FAT or exFAT
//!   labelled `YANTRIK` or `YANTRIK-*`, the kind macOS's Disk Utility makes), or a run of free
//!   space of at least 20 GB. Never APFS, HFS+, NTFS, a Linux filesystem, the EFI system
//!   partition, the installer's own medium, or anything mounted.
//! - **What is written** ([`plan`]): one `parted mkpart` with exact start and end sectors inside
//!   the free run, or a reformat of the chosen placeholder alone. The plan is checked against
//!   the table before it is returned: nothing outside the chosen region, never the EFI
//!   partition, never a table rewrite.
//! - **Re-checked before writing** ([`apply`]): the table is read again right before anything
//!   changes and must match, by [`DiskTable::fingerprint`], the one the person was shown. After
//!   writing it is read once more, and every partition that was not the target must be exactly
//!   where it was.
//! - **Booting** ([`efi`]): the existing EFI system partition is mounted, never formatted. The
//!   `\EFI\BOOT\BOOTX64.EFI` fallback is written only where none exists or the one there is
//!   ours, and a Mac's NVRAM is never touched.
//!
//! Pure logic and fixtures in `table`, `classify`, `plan` and `efi`; the commands that read and
//! write a real disk in `apply`, behind a runner the caller supplies (the desktop installer
//! runs them through sudo, the command-line tool directly).

pub mod apply;
pub mod classify;
pub mod efi;
pub mod plan;
pub mod table;

pub use classify::{segments, Kind, Segment, MIN_BYTES};
pub use plan::{parse_target_id, plan, target_id, PartRef, Plan, Role, Step, TargetSpec};
pub use table::{DiskTable, Part, Run};
