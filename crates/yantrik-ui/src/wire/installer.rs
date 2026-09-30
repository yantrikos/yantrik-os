//! Real disk installer — partitions, copies live system, installs GRUB, creates user.
//!
//! This runs the same operations as the text-based `yantrik-install` script but
//! from within the Slint UI (installer.slint), reporting progress back via a callback.

use slint::{ComponentHandle, ModelRc, VecModel};
use std::process::Command;

use crate::app_context::AppContext;
use crate::control_installer::step;
use crate::installer_rules;
use crate::wire::ai_onboarding::auth_type_for;
use crate::wire::installer_disk;
use crate::wire::installer_locale;
use crate::wire::settings::{provider_preset, ProviderStore, ProviderStoreEntry};
use crate::{App, InstallerDisk, KeyboardChoice};

mod subids;

/// Wire the installer callbacks.
pub fn wire(ui: &App, _ctx: &AppContext) {
    wire_rules(ui);

    let installer_mode = std::path::Path::new("/opt/yantrik/.installer-mode").exists();
    if installer_mode {
        ui.set_onboard_installer_mode(true);
        // Straight to the Welcome screen. The onboarding's four-second orb animation played
        // here too, before a person who had come to install anything could do so.
        ui.set_onboard_phase(step::WELCOME);
        tracing::info!("Installer mode detected — the installer is screen 2");
        prepare(ui);
    }

    // The person picked a layout on the Welcome screen.
    ui.on_onboard_keyboard_chosen(|layout| {
        let layout = layout.to_string();
        std::thread::spawn(move || installer_locale::apply_to_live_session(&layout));
    });

    // Handle install-to-disk callback from the installer screens and the control surface
    let ui_weak = ui.as_weak();
    ui.on_onboard_install_to_disk(move |username, password, full_name, hostname, keyboard, timezone, target_disk, encrypt| {
        let username = username.to_string();
        let password = password.to_string();
        let full_name = full_name.to_string();
        let hostname = hostname.to_string();
        let keyboard = keyboard.to_string();
        let timezone = timezone.to_string();
        let target_disk = target_disk.to_string();
        let weak = ui_weak.clone();

        std::thread::spawn(move || {
            tracing::info!(
                username = %username,
                target_disk = %target_disk,
                hostname = %hostname,
                keyboard = %keyboard,
                timezone = %timezone,
                encrypt,
                "Installer thread started"
            );

            // Report initial status
            {
                let w = weak.clone();
                let _ = slint::invoke_from_event_loop(move || {
                    if let Some(ui) = w.upgrade() {
                        ui.set_onboard_install_status("Preparing installation...".into());
                    }
                });
            }

            // The installer no longer asks about AI — that is first-boot setup now (#400) —
            // but a person who tried the live desktop first may have chosen a provider there,
            // and wire::ai_onboarding::save_primary keeps that choice in this store. Carried
            // over when present, so a key typed in the live session is not typed twice.
            let wizard = ProviderStore::load().primary().cloned();
            let (ai_provider, ai_base_url, ai_api_key) = match wizard {
                Some(p) => (p.provider_type, p.base_url, p.api_key.unwrap_or_default()),
                None => (String::new(), String::new(), String::new()),
            };
            // The endpoint is not a secret; the key is, so only its presence is logged.
            tracing::info!(
                ai_provider = %ai_provider,
                ai_endpoint = %ai_base_url,
                has_key = !ai_api_key.is_empty(),
                "Installer: AI choice carried over from the live session"
            );

            let state = InstallerState {
                username: username.clone(),
                password,
                full_name,
                hostname,
                language: String::new(),
                locale: String::new(),
                keyboard,
                timezone,
                target_disk: target_disk.clone(),
                encrypt,
                partition_scheme: "auto".into(),
                ai_provider,
                ai_base_url,
                ai_api_key,
            };

            let weak2 = weak.clone();
            let result = run_install(&state, Box::new(move |percent, status| {
                tracing::info!(percent, status, "Install progress");
                let weak3 = weak2.clone();
                let status = status.to_string();
                let _ = slint::invoke_from_event_loop(move || {
                    if let Some(ui) = weak3.upgrade() {
                        ui.set_onboard_install_progress(percent);
                        ui.set_onboard_install_status(status.into());
                    }
                });
            }));

            let weak_final = weak.clone();
            let _ = slint::invoke_from_event_loop(move || {
                if let Some(ui) = weak_final.upgrade() {
                    // Whatever happened, the work has stopped. `installing` is what the
                    // Install button and the control surface both check before starting one,
                    // and nothing used to clear it: a failed install left the flag set and
                    // every retry was refused as "already running".
                    ui.set_onboard_installing(false);
                    match result {
                        Ok(()) => {
                            ui.set_onboard_install_status("Installation complete!".into());
                            ui.set_onboard_install_progress(100);
                            // The installed step (and its restart countdown) is shown by the
                            // screen when install-progress reaches 100.
                        }
                        Err(e) => {
                            tracing::error!(error = %e, "Installation failed");
                            ui.set_onboard_install_status(format!("Installation failed: {e}").into());
                            ui.set_onboard_install_error(format!("Installation failed: {e}").into());
                            // Back to Review, where the error is displayed and the Install
                            // button lives. The progress screen has no way off it, so failing
                            // there left the machine staring at a stalled bar.
                            ui.set_onboard_phase(step::REVIEW);
                        }
                    }
                }
            });
        });
    });

    // Handle reboot button from install-complete screen
    ui.on_onboard_install_reboot(move || {
        tracing::info!("User requested reboot after installation");
        std::thread::spawn(|| {
            // Force reboot to avoid squashfs unmount loop
            let _ = Command::new("sudo")
                .args(["reboot", "-f"])
                .env("PATH", "/usr/local/sbin:/usr/local/bin:/usr/sbin:/usr/bin:/sbin:/bin")
                .status();
        });
    });
}

/// The installer's rules, handed to its screens. One set, in installer_rules.rs, so the field
/// that turns amber, the Next button that stays disabled and the control surface's
/// `blocked_by` cannot disagree.
fn wire_rules(ui: &App) {
    let problem = |p: Option<String>| p.unwrap_or_default().into();
    ui.on_installer_derive_username(|name| installer_rules::derive_username(&name).into());
    ui.on_installer_hostname_for(|username| installer_rules::hostname_for(&username).into());
    ui.on_installer_check_full_name(move |name| problem(installer_rules::full_name_problem(&name)));
    ui.on_installer_check_username(move |u| problem(installer_rules::username_problem(&u)));
    ui.on_installer_check_hostname(move |h| problem(installer_rules::hostname_problem(&h)));
    ui.on_installer_check_password(move |pw, confirm| {
        problem(installer_rules::password_problem(&pw, &confirm))
    });
    ui.on_installer_check_timezone(move |tz| {
        problem(installer_rules::timezone_problem(&tz, std::path::Path::new("/usr/share/zoneinfo")))
    });
}

/// What the installer finds before anyone asks: the disks, the keyboard in use, and where the
/// machine is. Each on its own worker, because two of them wait on other programs and one on
/// the network, and the Welcome screen should not.
fn prepare(ui: &App) {
    // Keyboard: the list at once, with the detected layout chosen.
    let detected = installer_locale::detect_layout();
    let choices: Vec<KeyboardChoice> = installer_rules::layout_choices(&detected)
        .into_iter()
        .map(|(code, label)| KeyboardChoice { code: code.into(), label: label.into() })
        .collect();
    ui.set_onboard_keyboards(ModelRc::new(VecModel::from(choices)));
    ui.set_onboard_keyboard(detected.clone().into());
    tracing::info!(layout = %detected, "Installer: keyboard layout detected");

    // Disks.
    let weak = ui.as_weak();
    std::thread::spawn(move || {
        let disks = detect_disks();
        let _ = slint::invoke_from_event_loop(move || {
            let Some(ui) = weak.upgrade() else { return };
            // The first disk is preselected: on the machine most people install on there is
            // exactly one, and the Disk screen says plainly that it will be erased.
            if let Some(first) = disks.first() {
                if ui.get_onboard_selected_disk().is_empty() {
                    ui.set_onboard_selected_disk(first.name.clone().into());
                }
            }
            let rows: Vec<InstallerDisk> = disks
                .iter()
                .map(|d| InstallerDisk {
                    name: d.name.clone().into(),
                    size: d.size.clone().into(),
                    model: d.model.clone().into(),
                    contents: d.contents.clone().into(),
                    has_data: d.has_data,
                })
                .collect();
            ui.set_onboard_disks(ModelRc::new(VecModel::from(rows)));
            ui.set_onboard_disks_scanned(true);
        });
    });

    // Timezone.
    let weak = ui.as_weak();
    std::thread::spawn(move || {
        let zone = installer_locale::detect_timezone();
        let _ = slint::invoke_from_event_loop(move || {
            let Some(ui) = weak.upgrade() else { return };
            // Not over one the person (or an agent) already typed.
            if ui.get_onboard_timezone().is_empty() {
                ui.set_onboard_timezone(zone.into());
            }
        });
    });
}

/// Shared installer state collected across onboarding callbacks.
#[derive(Debug, Clone, Default)]
pub struct InstallerState {
    pub language: String,
    pub locale: String,
    pub username: String,
    pub full_name: String,
    pub password: String,
    pub hostname: String,
    /// XKB layout chosen on the Welcome screen, e.g. "de". Empty means US.
    pub keyboard: String,
    /// IANA zone from the Review screen, e.g. "Europe/Berlin". Empty means UTC.
    pub timezone: String,
    pub target_disk: String,      // e.g. "sda"
    /// LUKS2 on the root with the password as its passphrase (#400 step b).
    pub encrypt: bool,
    pub partition_scheme: String,  // "auto" or "manual"
    pub ai_provider: String,
    /// The endpoint the wizard saved for the provider. Empty means nobody chose
    /// one, and the provider's well-known URL is used if a provider is set.
    pub ai_base_url: String,
    pub ai_api_key: String,
}

/// Progress callback: (percent 0-100, status message).
type ProgressFn = Box<dyn Fn(i32, &str) + Send>;

/// Run the full installation. Blocks the calling thread.
pub fn run_install(state: &InstallerState, progress: ProgressFn) -> Result<(), String> {
    progress(1, "Detecting target disk...");

    tracing::info!(target_disk = %state.target_disk, "Installer: starting, target_disk from UI");

    let disk_name = if state.target_disk.is_empty() {
        tracing::info!("Installer: no disk selected, auto-detecting...");
        auto_detect_disk()?
    } else {
        state.target_disk.clone()
    };
    let disk = format!("/dev/{}", disk_name);

    tracing::info!(disk = %disk, "Installer: will use disk");

    // Validate block device exists
    if !std::path::Path::new(&disk).exists() {
        return Err(format!("{disk} does not exist. Available devices:\n{}",
            list_block_devices()));
    }

    tracing::info!(disk = %disk, "Installer: target disk resolved");

    // Detect boot mode: EFI if /sys/firmware/efi exists, BIOS otherwise
    let is_efi = std::path::Path::new("/sys/firmware/efi").exists();
    tracing::info!(efi = is_efi, "Installer: boot mode detected");

    // ── Steps 1-3: partition, encrypt if asked, format, mount (installer_disk.rs) ──
    let layout = installer_disk::prepare(&disk, is_efi, state.encrypt, &state.password, &*progress)?;
    let mount_dir = "/mnt/yantrik-install";
    progress(16, "Mounting target filesystem...");
    if let Err(e) = installer_disk::mount(&layout, mount_dir) {
        installer_disk::release(&layout, mount_dir);
        return Err(e);
    }
    progress(18, "Target mounted");

    // From here, ensure we clean up on failure
    let result = install_to_target(state, &disk, &layout, is_efi, mount_dir, &progress);

    // ── Cleanup: unmount everything (lazy to avoid "device busy") ──
    // Kill any processes still using the mount
    let _ = run_cmd("fuser", &["-km", mount_dir]);
    std::thread::sleep(std::time::Duration::from_secs(1));

    // Unmount in reverse order, lazy flag to avoid EBUSY
    let _ = run_cmd("umount", &["-Rl", &format!("{mount_dir}/sys")]);
    let _ = run_cmd("umount", &["-Rl", &format!("{mount_dir}/proc")]);
    let _ = run_cmd("umount", &["-Rl", &format!("{mount_dir}/dev")]);
    // /boot/efi, /boot and the root, then the encrypted root closed.
    installer_disk::release(&layout, mount_dir);

    result
}

/// Core installation steps (after mount, before unmount).
fn install_to_target(
    state: &InstallerState,
    disk: &str,
    layout: &installer_disk::Layout,
    is_efi: bool,
    mount_dir: &str,
    progress: &ProgressFn,
) -> Result<(), String> {
    // ── Step 4: Copy live system via rsync ──────────────────────
    copy_system(mount_dir, progress)?;
    progress(55, "System files copied");

    // ── Step 5: Generate fstab ──────────────────────────────────
    progress(58, "Configuring filesystem table...");
    // fstab, and for an encrypted root its crypttab and the initramfs settings that ask for it.
    installer_disk::write_system_files(layout, mount_dir)?;

    // ── Step 6: Set hostname ────────────────────────────────────
    progress(60, "Setting hostname...");
    let hostname = if state.hostname.is_empty() {
        "yantrik"
    } else {
        &state.hostname
    };
    sudo_write(&format!("{mount_dir}/etc/hostname"), &format!("{hostname}\n"))?;
    sudo_write(
        &format!("{mount_dir}/etc/hosts"),
        &format!("127.0.0.1\tlocalhost\n127.0.1.1\t{hostname}\n"),
    )?;

    // The clock, from the Review screen. Written here rather than left to the desktop's own
    // geolocation, which only fills a machine still at UTC and only after the first login.
    progress(61, "Setting the timezone...");
    let zone = installer_locale::configure_target_timezone(
        mount_dir,
        if state.timezone.is_empty() { installer_locale::FALLBACK_TIMEZONE } else { &state.timezone },
    )?;
    tracing::info!(timezone = %zone, "Installer: timezone set");

    // ── Step 7: Bind-mount for chroot (needed BEFORE any chroot commands) ──
    progress(62, "Preparing chroot environment...");
    // --rbind, not --bind. A plain bind mounts the one filesystem and leaves every submount
    // behind, so the chroot got a /sys with no /sys/firmware/efi/efivars under it. grub-install
    // then could not write a UEFI boot entry, which is how this installer ended up carrying a
    // `--no-nvram` flag and producing disks no firmware would boot.
    for dir in ["dev", "proc", "sys"] {
        let target = format!("{mount_dir}/{dir}");
        run_cmd("mount", &["--rbind", &format!("/{dir}"), &target])?;
        // And then make it a slave. Mounts under /sys and /dev propagate by default, so the
        // recursive unmount at the end of the install travelled back up the bind and tore
        // efivarfs off the running system: the first install worked and a second one in the
        // same session would find no EFI variables and quietly install an unbootable disk.
        // A slave mount receives changes from the host and sends none back, which is exactly
        // the relationship a chroot wants.
        run_cmd("mount", &["--make-rslave", &target])?;
    }

    // ── Step 8: Create user account ─────────────────────────────
    progress(65, "Creating user account...");
    create_user(mount_dir, state)?;

    // ── Step 9: Set locale ──────────────────────────────────────
    progress(68, "Configuring locale...");
    let locale = if state.locale.is_empty() {
        "en_US.UTF-8"
    } else {
        &state.locale
    };
    // Uncomment the locale in locale.gen
    let locale_gen_path = format!("{mount_dir}/etc/locale.gen");
    if let Ok(content) = std::fs::read_to_string(&locale_gen_path) {
        let uncommented = content.replace(&format!("# {locale}"), locale);
        let _ = sudo_write(&locale_gen_path, &uncommented);
    }
    let _ = chroot_cmd(mount_dir, &["locale-gen"]);
    let _ = sudo_write(
        &format!("{mount_dir}/etc/default/locale"),
        &format!("LANG={locale}\n"),
    );

    // The keyboard, from the Welcome screen: the console's here, the desktop's in the person's
    // labwc environment (create_user). A failure costs the console its layout, not the install.
    progress(69, "Configuring the keyboard...");
    if let Err(e) = installer_locale::configure_target_keyboard(mount_dir, installed_layout(state)) {
        // On an encrypted disk the layout is how the passphrase is read at every start; a wrong
        // one is a disk nobody can open, so it fails the install rather than warning.
        if layout.encrypted() {
            return Err(format!("could not set the keyboard the disk's passphrase is typed on: {e}"));
        }
        tracing::warn!(error = %e, "Could not write the installed keyboard layout");
    }

    // ── Step 10: Configure AI provider ───────────────────────────
    progress(70, "Configuring AI provider...");
    configure_ai(mount_dir, state);

    // ── Step 11: Remove live-boot packages (not needed on installed system) ──
    progress(73, "Removing live-boot packages...");
    let _ = chroot_cmd(mount_dir, &["apt-get", "remove", "-y", "--purge",
        "live-boot", "live-config", "live-config-systemd"]);
    let _ = chroot_cmd(mount_dir, &["apt-get", "autoremove", "-y"]);

    // ── Step 12: Install GRUB ───────────────────────────────────
    progress(75, "Installing bootloader...");
    if is_efi {
        // Two installs, and both are needed.
        //
        // The first writes \EFI\yantrik and asks the firmware to remember it. The second writes
        // \EFI\BOOT\BOOTX64.EFI, the removable-media path every UEFI implementation tries when
        // it has no entry of its own.
        //
        // Only the first used to run, and with `--no-nvram` on it, so it left a disk with a
        // bootloader in a directory nothing had been told to look in. The machine installed
        // cleanly, reported 100%, rebooted, and came straight back up on the installation
        // media — the firmware had no entry for the disk and no fallback file to find.
        tracing::info!("Installer: installing GRUB for EFI");
        let named = chroot_cmd(
            mount_dir,
            &[
                "grub-install",
                "--target=x86_64-efi",
                "--efi-directory=/boot/efi",
                "--bootloader-id=yantrik",
            ],
        );
        if let Err(e) = named {
            // Firmware that will not take a new entry is normal enough — a locked-down board,
            // or efivars mounted read-only. It costs us the named entry, not the install,
            // because the removable path below does not need NVRAM at all.
            tracing::warn!(error = %e, "could not register a UEFI boot entry; the removable path will carry the boot");
            chroot_cmd(
                mount_dir,
                &[
                    "grub-install",
                    "--target=x86_64-efi",
                    "--efi-directory=/boot/efi",
                    "--bootloader-id=yantrik",
                    "--no-nvram",
                ],
            )?;
        }

        // This one is not optional and its failure is the install's failure.
        chroot_cmd(
            mount_dir,
            &["grub-install", "--target=x86_64-efi", "--efi-directory=/boot/efi", "--removable"],
        )?;

        // Check the file, not the exit code. grub-install has been known to report success
        // having written nothing useful, and "the installer said it worked" is exactly the
        // claim that cost us a boot.
        let fallback = format!("{mount_dir}/boot/efi/EFI/BOOT/BOOTX64.EFI");
        if !std::path::Path::new(&fallback).exists() {
            return Err(
                "grub-install reported success but left no EFI/BOOT/BOOTX64.EFI on the \
                 EFI partition; the disk would not boot"
                    .to_string(),
            );
        }
        tracing::info!("Installer: EFI fallback bootloader present");
    } else {
        tracing::info!("Installer: installing GRUB for BIOS on {disk}");
        chroot_cmd(
            mount_dir,
            &["grub-install", "--target=i386-pc", disk],
        )?;
    }

    // Brand the installed system as Yantrik OS (so GRUB says "Yantrik OS" not "Debian")
    //
    // VERSION_ID was the literal "0.3.0", so every machine installed from the desktop said
    // "Yantrik OS 0.3.0" on its getty banner and to every tool that reads os-release, whatever
    // build it was carrying. It is the build being installed — the same string yantrik-install.sh
    // takes out of the BUILD marker for the same file.
    let _ = sudo_write(
        &format!("{mount_dir}/etc/os-release"),
        &format!(
            "PRETTY_NAME=\"Yantrik OS\"\nNAME=\"Yantrik OS\"\nID=yantrik\nID_LIKE=debian\nVERSION_ID=\"{}\"\nHOME_URL=\"https://yantrikos.com\"\n",
            yantrik_version::version()
        ),
    );

    // Configure GRUB defaults. loglevel=3: `quiet` still prints the kernel's error-level lines,
    // and on real hardware (and in a VM) those scroll over the disk's passphrase prompt.
    let _ = sudo_write(
        &format!("{mount_dir}/etc/default/grub"),
        "GRUB_DEFAULT=0\nGRUB_TIMEOUT=3\nGRUB_DISTRIBUTOR=\"Yantrik OS\"\nGRUB_CMDLINE_LINUX_DEFAULT=\"quiet splash loglevel=3\"\nGRUB_CMDLINE_LINUX=\"console=ttyS0,115200 console=tty1\"\nGRUB_TERMINAL=\"console serial\"\nGRUB_SERIAL_COMMAND=\"serial --speed=115200\"\n",
    );

    progress(85, "Updating GRUB configuration...");
    chroot_cmd(mount_dir, &["update-grub"])?;

    // ── Step 13: Remove installer autostart from installed system ─
    progress(90, "Finalizing installed system...");
    // The installed system should boot to desktop, not installer
    // Remove yantrik.install=true from any boot config if present
    let grub_default = format!("{mount_dir}/etc/default/grub");
    if let Ok(content) = std::fs::read_to_string(&grub_default) {
        let cleaned = content.replace("yantrik.install=true", "");
        let _ = sudo_write(&grub_default, &cleaned);
    }

    // Ensure the installed system boots to desktop (not installer)
    let marker = format!("{mount_dir}/opt/yantrik/.installer-mode");
    let _ = run_cmd("rm", &["-f", &marker]);

    // The session's log directory, the person's and written by them alone. It was 0777, which
    // let any other account plant a name the session appends to — a symlink to ~/.bashrc, say
    // (yantrik-update's reconcile_private_dirs repairs machines installed that way).
    let logs = format!("{mount_dir}/opt/yantrik/logs");
    let owner = if state.username.is_empty() { "yantrik" } else { &state.username };
    let _ = run_cmd("install", &["-d", "-m", "0755", &logs]);
    let _ = run_cmd("chmod", &["0755", &logs]);
    let _ = chroot_cmd(mount_dir, &["chown", &format!("{owner}:{owner}"), "/opt/yantrik/logs"]);

    // Regenerate initramfs without live-boot hooks, and with the unlock when the root is
    // encrypted. There a failure is the install's failure, and so is an image that came out
    // without the unlock: the machine would stop at boot with nothing asking for the passphrase.
    progress(93, "Rebuilding initramfs...");
    if layout.encrypted() {
        chroot_cmd(mount_dir, &["update-initramfs", "-u", "-k", "all"])?;
        installer_disk::verify_initramfs(mount_dir)?;
    } else {
        let _ = chroot_cmd(mount_dir, &["update-initramfs", "-u"]);
    }

    progress(100, "Installation complete!");
    Ok(())
}

/// Create user account inside the chroot.
fn create_user(mount_dir: &str, state: &InstallerState) -> Result<(), String> {
    let username = if state.username.is_empty() {
        "yantrik"
    } else {
        &state.username
    };

    // Delete the live system's default user if creating a different one
    if username != "yantrik" {
        let _ = chroot_cmd(mount_dir, &["userdel", "-r", "yantrik"]);
    }

    // Create user with home directory
    let _ = chroot_cmd(
        mount_dir,
        &[
            "useradd",
            "-m",
            "-s", "/bin/bash",
            "-G", "sudo,video,audio,input",
            username,
        ],
    );

    // Subordinate ids for rootless podman (#401). useradd normally gives the new account a range
    // from the image's /etc/subuid, and the live user keeps the one the image gave it; checked.
    subids::ensure(mount_dir, username);

    // Set password — use openssl to generate hash, then usermod to set it.
    // chpasswd inside chroot can fail silently with PAM issues.
    if !state.password.is_empty() {
        // Generate the password hash on the host
        let hash_output = Command::new("openssl")
            .args(["passwd", "-6", "-stdin"])
            .stdin(std::process::Stdio::piped())
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped())
            .spawn()
            .and_then(|mut child| {
                if let Some(ref mut stdin) = child.stdin {
                    use std::io::Write;
                    let _ = stdin.write_all(state.password.as_bytes());
                }
                drop(child.stdin.take()); // close stdin so openssl finishes
                child.wait_with_output()
            });

        match hash_output {
            Ok(output) if output.status.success() => {
                let hash = String::from_utf8_lossy(&output.stdout).trim().to_string();
                tracing::info!("Password hash generated, setting via usermod");
                let res = chroot_cmd(mount_dir, &["usermod", "-p", &hash, username]);
                match res {
                    Ok(o) => tracing::info!(output = %o, "usermod password set"),
                    Err(e) => tracing::error!(error = %e, "usermod password failed"),
                }
            }
            Ok(output) => {
                let stderr = String::from_utf8_lossy(&output.stderr);
                tracing::error!(stderr = %stderr, "openssl passwd failed");
            }
            Err(e) => tracing::error!(error = %e, "openssl passwd spawn failed"),
        }
    }

    // Set full name via chfn
    if !state.full_name.is_empty() {
        let _ = chroot_cmd(mount_dir, &["chfn", "-f", &state.full_name, username]);
    }

    // Write all desktop startup files directly (don't rely on copy from deleted user)
    let dst_home = format!("{mount_dir}/home/{username}");

    // .bash_profile — auto-start labwc on tty1
    sudo_write(
        &format!("{dst_home}/.bash_profile"),
        r#"# Auto-start Yantrik desktop on tty1
if [ "$(tty)" = "/dev/tty1" ] && [ -z "$WAYLAND_DISPLAY" ]; then
    export XDG_RUNTIME_DIR="/run/user/$(id -u)"
    mkdir -p "$XDG_RUNTIME_DIR"

    if [ -f "$HOME/.config/labwc/environment" ]; then
        set -a
        . "$HOME/.config/labwc/environment"
        set +a
    else
        export WLR_RENDERER=pixman
        export WLR_RENDERER_ALLOW_SOFTWARE=1
        export SLINT_BACKEND=winit
        export LIBGL_ALWAYS_SOFTWARE=1
    fi

    # Crash guard
    # The stamp lives in the home, not /tmp: any account can write a fresh one in /tmp, and a
    # fresh one there is enough to keep this desktop from starting at all.
    mkdir -p "$HOME/.cache"
    CRASH_FILE="$HOME/.cache/yantrik-labwc-crash"
    if [ -f "$CRASH_FILE" ]; then
        LAST_CRASH=$(cat "$CRASH_FILE" 2>/dev/null || echo 0)
        NOW=$(date +%s)
        if [ $((NOW - LAST_CRASH)) -lt 10 ]; then
            echo "  Yantrik OS — Desktop failed to start"
            echo "  Check: cat /opt/yantrik/logs/labwc.log"
            exec /bin/bash --login
        fi
    fi

    START_TIME=$(date +%s)
    # The session every Yantrik machine runs, whatever installed it: the shipped compositor
    # config, fullscreen, and the shell as the session client.
    /opt/yantrik/bin/yantrik-session 2>>/opt/yantrik/logs/labwc.log
    EXIT_TIME=$(date +%s)

    if [ $((EXIT_TIME - START_TIME)) -lt 5 ]; then
        echo "$EXIT_TIME" > "$CRASH_FILE"
    else
        rm -f "$CRASH_FILE"
        # A session that ran and then ended (labwc crashed hours in, or was killed) ends the
        # login, so tty1's autologin starts the desktop again. Left to fall through, it left a
        # bash prompt on tty1 and no desktop until a reboot. A quick crash still falls through to
        # the prompt, as it always has.
        exit 0
    fi
fi
"#,
    )?;

    // labwc environment — software rendering for VBox/headless
    let labwc_dir = format!("{dst_home}/.config/labwc");
    let _ = run_cmd("mkdir", &["-p", &labwc_dir]);

    // No YANTRIK_START_SCREEN: the shell starts an installed machine behind the session lock,
    // asking for this password (#415). It used to name the login screen (32) here, which is drawn
    // in the shell's own window, so an app window could sit over it; and this file is the
    // user's, so anything running as them could name another start screen.
    //
    // XKB_DEFAULT_LAYOUT is the layout chosen on the Welcome screen: labwc reads it from this
    // file, and it is what decides the keys at the lock screen where this password is asked.
    sudo_write(
        &format!("{labwc_dir}/environment"),
        &installer_rules::environment_with_layout(
            "WLR_RENDERER=pixman\nWLR_RENDERER_ALLOW_SOFTWARE=1\nXDG_SESSION_TYPE=wayland\nQT_QPA_PLATFORM=wayland\nMOZ_ENABLE_WAYLAND=1\nSLINT_BACKEND=winit\nLIBGL_ALWAYS_SOFTWARE=1\n",
            installed_layout(state),
        ),
    )?;

    // No autostart or rc.xml here. `yantrik-session` installs the shipped ones from
    // /opt/yantrik/share at every login. This installer used to write its own, and they drifted:
    // an installed machine drew the shell inside a window with a title bar and minimise, maximise
    // and close buttons, with none of the key bindings the real config carries.

    // The installer asked only what installing needs (#400). What it used to ask on the way —
    // interests, how to be reached, which AI — is the first boot's to offer, as optional setup.
    // This marker says the account is done, so that setup opens on a welcome with Skip rather
    // than asking for the person's name a second time. `.onboarding_complete` is deliberately
    // not written: it would skip the setup altogether.
    let _ = run_cmd("mkdir", &["-p", &format!("{dst_home}/.yantrik")]);
    // Kept as `yantrik`, the account is the live user's copied home, which carries the live
    // session's marker if someone finished or skipped the onboarding there.
    let _ = run_cmd("rm", &["-f", &format!("{dst_home}/.yantrik/.onboarding_complete")]);
    sudo_write(
        &format!("{dst_home}/{}", crate::onboarding::AFTER_INSTALL_MARKER),
        "installed\n",
    )?;

    // Fix ownership of everything in home
    let _ = chroot_cmd(mount_dir, &["chown", "-R", &format!("{username}:{username}"), &format!("/home/{username}")]);

    // Update XDG runtime dir tmpfiles for the new user's UID
    let uid_output = chroot_cmd(mount_dir, &["id", "-u", username]).unwrap_or_else(|_| "1000".into());
    let uid = uid_output.trim();
    let _ = sudo_write(
        &format!("{mount_dir}/etc/tmpfiles.d/yantrik-xdg.conf"),
        &format!("d /run/user/{uid} 0700 {username} {username} -\n"),
    );

    // Root stays locked on an installed machine. The live image once shipped `root:root` with SSH
    // root login allowed, and copying the live system carried both onto every install; the image
    // no longer does, and this holds for any image that still might.
    let _ = chroot_cmd(mount_dir, &["passwd", "-l", "root"]);
    let _ = run_cmd("sed", &["-i", "/^PermitRootLogin/d", &format!("{mount_dir}/etc/ssh/sshd_config.d/yantrik.conf")]);

    // The OS's own code is root's on an installed machine (#397): the shell, the updater and every
    // binary used to belong to the desktop's user, so anything running as them could replace the
    // OS. The updater moves the copied tree over and puts its narrow sudo rule in place; the
    // desktop's user keeps logs/, data/ and config.yaml. Refused or missing, the machine's first
    // update does the same.
    if let Err(e) = chroot_cmd(mount_dir, &["/opt/yantrik/bin/yantrik-update", "migrate-ownership"]) {
        tracing::warn!(error = %e, "Could not make /opt/yantrik root's at install; the first update will");
    }

    // No passwordless sudo for everything on an installed machine (#397): the migration above
    // put the narrow rule in place (the updater, the Package Manager's helper, the timezone) and
    // removed the blanket one the live image carries. The person's own password does the rest.

    // Autologin to start labwc + yantrik-ui automatically (no TTY shown to user).
    // Yantrik UI shows its own graphical login screen (screen 32) for authentication.
    let autologin_dir = format!("{mount_dir}/etc/systemd/system/getty@tty1.service.d");
    let _ = run_cmd("mkdir", &["-p", &autologin_dir]);
    let _ = sudo_write(
        &format!("{autologin_dir}/autologin.conf"),
        &format!(
            "[Service]\nExecStart=\nExecStart=-/sbin/agetty --autologin {username} --noclear %I $TERM\n"
        ),
    );

    Ok(())
}

/// Write AI provider config and user_name into the installed system's config.yaml,
/// and the provider entry — key included — into the new user's providers.yaml.
fn configure_ai(mount_dir: &str, state: &InstallerState) {
    let config_path = format!("{mount_dir}/opt/yantrik/config.yaml");
    if let Ok(content) = std::fs::read_to_string(&config_path) {
        let _ = sudo_write(&config_path, &installed_config_yaml(&content, state));
    }

    let display_name = installed_display_name(state);
    let username = if state.username.is_empty() { "yantrik" } else { &state.username };
    let settings_dir = format!("{mount_dir}/home/{username}/.config/yantrik");
    let settings_path = format!("{settings_dir}/settings.yaml");
    let _ = run_cmd("mkdir", &["-p", &settings_dir]);

    // ── The wizard's provider, key included, into the user's providers.yaml ──
    // The key goes where Settings keeps provider keys — the person's own
    // ~/.config/yantrik/providers.yaml — and not into config.yaml above, which
    // is world-readable under /opt. It is written fresh rather than left to the
    // rsync'd copy of the live session: when the person chose a username other
    // than the live user's, create_user deleted that home with `userdel -r` and
    // the store went with it.
    if let Some(yaml) = installed_providers_yaml(state) {
        let providers_path = format!("{settings_dir}/providers.yaml");
        if sudo_write(&providers_path, &yaml).is_ok() {
            // sudo tee creates the file world-readable; it holds a secret.
            let _ = run_cmd("chmod", &["600", &providers_path]);
        }
    }

    // ── Also write user_name to per-user settings.yaml ──────────
    let settings_content = if let Ok(existing) = std::fs::read_to_string(&settings_path) {
        let mut s = existing;
        if let Some(start) = s.find("user_name:") {
            if let Some(end) = s[start..].find('\n') {
                let line_end = start + end;
                s.replace_range(start..line_end, &format!("user_name: \"{display_name}\""));
            }
        } else {
            s.insert_str(0, &format!("user_name: \"{display_name}\"\n"));
        }
        s
    } else {
        format!("user_name: \"{display_name}\"\n")
    };

    let _ = sudo_write(&settings_path, &settings_content);
    // Fix ownership so the user can read/write their settings
    let _ = chroot_cmd(mount_dir, &["chown", "-R",
        &format!("{username}:{username}"),
        &format!("/home/{username}/.config/yantrik")]);
}

/// The layout to install: the chosen one if it is a layout at all, US otherwise. Checked here
/// as well as on the screen because it is written into a file the session sources.
fn installed_layout(state: &InstallerState) -> &str {
    if installer_rules::is_plausible_layout(&state.keyboard) {
        &state.keyboard
    } else {
        installer_rules::DEFAULT_LAYOUT
    }
}

/// Whose name the installed system should show: the full name if one was given,
/// the username if not, and a neutral fallback after that.
fn installed_display_name(state: &InstallerState) -> &str {
    if !state.full_name.is_empty() {
        &state.full_name
    } else if !state.username.is_empty() {
        &state.username
    } else {
        "User"
    }
}

/// Produce the installed /opt/yantrik/config.yaml: the image's file with the
/// wizard's answers written over the defaults.
///
/// The person's name replaces `user_name`; their AI choice replaces the primary
/// `api_base_url` and `api_model`. The endpoint the wizard saved wins over the
/// provider's well-known URL, because for a local runtime it may be a remote
/// host the person configured — the same preference save_primary applies. The
/// API key is never written here: this file is world-readable under /opt, and
/// the key belongs in the person's providers.yaml (`installed_providers_yaml`).
fn installed_config_yaml(content: &str, state: &InstallerState) -> String {
    let mut out = content.to_string();

    let display_name = installed_display_name(state);
    if !replace_yaml_line(&mut out, "user_name:", &format!("user_name: \"{display_name}\"")) {
        // No user_name line exists — insert at the top of the file
        out.insert_str(0, &format!("user_name: \"{display_name}\"\n"));
    }

    if !state.ai_provider.is_empty() {
        let base_url = if !state.ai_base_url.is_empty() {
            state.ai_base_url.as_str()
        } else {
            provider_base_url(&state.ai_provider)
        };
        // The wizard does not ask for a model — the AI pages pick a provider and
        // a key, and Settings offers the model list later — so the provider's
        // default is what the installed system starts on.
        let model = provider_default_model(&state.ai_provider);
        replace_yaml_line(&mut out, "api_base_url:", &format!("api_base_url: \"{base_url}\""));
        replace_yaml_line(&mut out, "api_model:", &format!("api_model: \"{model}\""));
    }

    out
}

/// Replace the first line containing `key` with `line`, keeping the indentation
/// before it. Reports whether anything was replaced.
fn replace_yaml_line(content: &mut String, key: &str, line: &str) -> bool {
    let Some(start) = content.find(key) else { return false };
    let Some(end) = content[start..].find('\n') else { return false };
    content.replace_range(start..start + end, line);
    true
}

/// The wizard's provider as an entry of the installed person's providers.yaml —
/// the same shape, id and auth scheme `save_primary` wrote in the live session,
/// so the installed machine's Settings shows the provider they chose, with the
/// key they typed. `None` when they never got to the AI pages.
fn installed_providers_yaml(state: &InstallerState) -> Option<String> {
    if state.ai_provider.is_empty() {
        return None;
    }
    let (name, preset_url) = provider_preset(&state.ai_provider);
    let base_url = if !state.ai_base_url.is_empty() {
        state.ai_base_url.as_str()
    } else {
        preset_url
    };
    if base_url.is_empty() {
        return None;
    }
    let entry = ProviderStoreEntry {
        id: format!("{}-onboarding", state.ai_provider),
        name: name.to_string(),
        provider_type: state.ai_provider.clone(),
        base_url: base_url.to_string(),
        api_key: (!state.ai_api_key.is_empty()).then(|| state.ai_api_key.clone()),
        auth_type: auth_type_for(&state.ai_provider).to_string(),
        is_primary: true,
        is_fallback: false,
    };
    let store = ProviderStore { entries: vec![entry] };
    serde_yaml::to_string(&store).ok()
}

/// Resolve provider name to default API base URL.
fn provider_base_url(provider: &str) -> &'static str {
    match provider {
        "ollama" => "http://localhost:11434/v1",
        "openai" => "https://api.openai.com/v1",
        "anthropic" | "claude" => "https://api.anthropic.com/v1",
        "google" | "gemini" => "https://generativelanguage.googleapis.com/v1beta/openai",
        "deepseek" => "https://api.deepseek.com/v1",
        "groq" => "https://api.groq.com/openai/v1",
        "mistral" => "https://api.mistral.ai/v1",
        "xai" | "grok" => "https://api.x.ai/v1",
        "perplexity" => "https://api.perplexity.ai",
        "cerebras" => "https://api.cerebras.ai/v1",
        "sambanova" => "https://api.sambanova.ai/v1",
        "openrouter" => "https://openrouter.ai/api/v1",
        "together" => "https://api.together.xyz/v1",
        "fireworks" => "https://api.fireworks.ai/inference/v1",
        "huggingface" => "https://api-inference.huggingface.co/v1",
        "nanogpt" => "https://api.nano-gpt.com/v1",
        "qwen" => "https://dashscope.aliyuncs.com/compatible-mode/v1",
        "minimax" => "https://api.minimax.chat/v1",
        "kimi" | "moonshot" => "https://api.moonshot.cn/v1",
        "baidu" => "https://qianfan.baidubce.com/v2",
        "zhipu" => "https://open.bigmodel.cn/api/paas/v4",
        _ => "http://localhost:11434/v1",
    }
}

/// Resolve provider name to a reasonable default model.
fn provider_default_model(provider: &str) -> &'static str {
    match provider {
        "ollama" => "nemotron-3-nano:4b",
        "openai" => "gpt-4o-mini",
        "anthropic" | "claude" => "claude-sonnet-4-20250514",
        "google" | "gemini" => "gemini-2.0-flash",
        "deepseek" => "deepseek-chat",
        "groq" => "llama-3.3-70b-versatile",
        "mistral" => "mistral-small-latest",
        "xai" | "grok" => "grok-2-latest",
        "perplexity" => "sonar",
        "cerebras" => "llama-3.3-70b",
        "sambanova" => "Meta-Llama-3.3-70B-Instruct",
        "openrouter" => "meta-llama/llama-3.3-70b-instruct",
        "together" => "meta-llama/Llama-3.3-70B-Instruct-Turbo",
        "fireworks" => "accounts/fireworks/models/llama-v3p3-70b-instruct",
        "huggingface" => "meta-llama/Llama-3.3-70B-Instruct",
        "qwen" => "qwen-plus",
        "minimax" => "MiniMax-Text-01",
        "kimi" | "moonshot" => "moonshot-v1-8k",
        _ => "auto",
    }
}

/// Auto-detect the installation target disk.
/// Picks the first non-removable, non-CD block device (excludes loop, sr, fd).
fn auto_detect_disk() -> Result<String, String> {
    let output = Command::new("lsblk")
        .args(["-dn", "-o", "NAME,TYPE,RO,RM", "-e", "7,11"])
        .env("PATH", "/usr/local/sbin:/usr/local/bin:/usr/sbin:/usr/bin:/sbin:/bin")
        .output()
        .map_err(|e| format!("lsblk failed: {e}"))?;

    let text = String::from_utf8_lossy(&output.stdout);
    for line in text.lines() {
        let parts: Vec<&str> = line.split_whitespace().collect();
        if parts.len() < 4 { continue; }
        let name = parts[0];
        let dtype = parts[1];
        let ro = parts[2];
        let rm = parts[3];

        // Only disk type, not read-only, not removable
        if dtype != "disk" { continue; }
        if ro == "1" || rm == "1" { continue; }
        // Skip loop, sr (CD), fd (floppy)
        if name.starts_with("loop") || name.starts_with("sr") || name.starts_with("fd") {
            continue;
        }

        tracing::info!(disk = name, "Installer: auto-detected target disk");
        return Ok(name.to_string());
    }

    // Fallback: try any disk that's not the live media
    // Live media is usually sr0 or the device mounted at /run/live/medium
    let fallback = Command::new("lsblk")
        .args(["-dn", "-o", "NAME", "-e", "7,11"])
        .env("PATH", "/usr/local/sbin:/usr/local/bin:/usr/sbin:/usr/bin:/sbin:/bin")
        .output()
        .map_err(|e| format!("lsblk fallback: {e}"))?;

    let text = String::from_utf8_lossy(&fallback.stdout);
    for line in text.lines() {
        let name = line.trim();
        if name.is_empty() || name.starts_with("sr") || name.starts_with("loop") || name.starts_with("fd") {
            continue;
        }
        tracing::info!(disk = name, "Installer: fallback disk detected");
        return Ok(name.to_string());
    }

    Err("No suitable disk found. Ensure a hard disk is attached.".into())
}

/// Get a single partition device name by number.

/// Write a file via sudo tee (since direct fs::write lacks root perms).
pub(super) fn sudo_write(path: &str, content: &str) -> Result<(), String> {
    let mut child = Command::new("sudo")
        .args(["tee", path])
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::piped())
        .env("PATH", "/usr/local/sbin:/usr/local/bin:/usr/sbin:/usr/bin:/sbin:/bin")
        .spawn()
        .map_err(|e| format!("sudo tee {path}: {e}"))?;

    if let Some(stdin) = child.stdin.as_mut() {
        use std::io::Write;
        let _ = stdin.write_all(content.as_bytes());
    }

    let output = child.wait_with_output().map_err(|e| format!("sudo tee wait: {e}"))?;
    if output.status.success() {
        Ok(())
    } else {
        Err(format!("sudo tee {path}: {}", String::from_utf8_lossy(&output.stderr)))
    }
}

/// Run a command via sudo, returning Ok(stdout) or Err(stderr).
/// All installer commands need root privileges for disk/mount/chroot operations.
/// Copy the live filesystem onto the target, reporting how far along it is.
///
/// This used to be one `run_cmd` call, which waits for the process and hands back its output in
/// a single piece. The consequence was that `progress` sat at 20 for the whole multi-minute
/// copy — and a number that does not move is indistinguishable from a hang, whether you are a
/// person watching a bar or an agent reading `installer.progress` off the control surface. It
/// was the longest step in the install and the only one that said nothing while it ran.
///
/// `--info=progress2` reports one running percentage for the whole transfer rather than
/// per-file, and `--no-inc-recursive` makes rsync build the file list up front so that
/// percentage means something instead of drifting against an estimate that keeps growing. The
/// scan costs maybe a quarter of a minute before the first number appears, which is why the
/// step announces itself before it starts.
fn copy_system(mount_dir: &str, progress: &ProgressFn) -> Result<(), String> {
    use std::io::Read;
    use std::process::Stdio;

    // The band this step owns. Partitioning and formatting finish at 20; fstab starts at 58.
    const START: i32 = 20;
    const END: i32 = 55;

    progress(START, "Scanning the live system...");

    let target = format!("{mount_dir}/");
    let mut child = Command::new("sudo")
        .args([
            "rsync",
            "-aAXH",
            "--info=progress2",
            "--no-inc-recursive",
            "--exclude=/proc/*",
            "--exclude=/sys/*",
            "--exclude=/dev/*",
            "--exclude=/run/*",
            "--exclude=/tmp/*",
            "--exclude=/mnt/*",
            "--exclude=/live/*",
            "--exclude=/cdrom/*",
            "/",
            &target,
        ])
        .env("PATH", "/usr/local/sbin:/usr/local/bin:/usr/sbin:/usr/bin:/sbin:/bin")
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|e| format!("failed to run rsync: {e}"))?;

    // Drained on its own thread. rsync can be noisy about unreadable files and a full stderr
    // pipe would block it forever while we sit here reading stdout.
    let errors = child.stderr.take().map(|mut pipe| {
        std::thread::spawn(move || {
            let mut text = String::new();
            let _ = pipe.read_to_string(&mut text);
            text
        })
    });

    let mut out = child.stdout.take().ok_or("rsync gave us no output to read")?;
    let mut buf = [0u8; 4096];
    let mut field = String::new();
    let mut last = START;
    loop {
        let read = match out.read(&mut buf) {
            Ok(0) => break,
            Ok(n) => n,
            Err(e) => return Err(format!("reading rsync progress: {e}")),
        };
        for &byte in &buf[..read] {
            // rsync rewrites its progress line in place, so updates are separated by carriage
            // returns; splitting on newlines alone would yield one enormous line at the end.
            if byte == b'\r' || byte == b'\n' {
                if let Some(percent) = rsync_percent(&field) {
                    let mapped = START + (percent * (END - START)) / 100;
                    // Only ever forwards: rsync's figure can twitch backwards near the end and
                    // a bar that retreats reads as something going wrong.
                    if mapped > last {
                        last = mapped;
                        progress(mapped, "Copying system files...");
                    }
                }
                field.clear();
            } else {
                field.push(byte as char);
            }
        }
    }

    let status = child.wait().map_err(|e| format!("waiting for rsync: {e}"))?;
    let stderr = errors.and_then(|h| h.join().ok()).unwrap_or_default();
    if !status.success() {
        let detail = stderr.trim();
        return Err(if detail.is_empty() {
            format!("rsync failed ({status})")
        } else {
            format!("rsync failed: {detail}")
        });
    }
    Ok(())
}

/// The percentage out of an `--info=progress2` line, if the line has one.
///
/// The line looks like `  1,234,567,890  42%  245.67MB/s    0:01:12`, so the percentage is the
/// one field ending in `%`. Anything else rsync prints is ignored rather than guessed at.
fn rsync_percent(line: &str) -> Option<i32> {
    line.split_whitespace()
        .find_map(|word| word.strip_suffix('%'))
        .and_then(|digits| digits.parse::<i32>().ok())
        .filter(|p| (0..=100).contains(p))
}

pub(super) fn run_cmd(cmd: &str, args: &[&str]) -> Result<String, String> {
    tracing::debug!(cmd = cmd, args = ?args, "installer: running command (via sudo)");

    let mut sudo_args = vec![cmd];
    sudo_args.extend_from_slice(args);

    let output = Command::new("sudo")
        .args(&sudo_args)
        .env("PATH", "/usr/local/sbin:/usr/local/bin:/usr/sbin:/usr/bin:/sbin:/bin")
        .output()
        .map_err(|e| format!("failed to run sudo {cmd}: {e}"))?;

    if output.status.success() {
        Ok(String::from_utf8_lossy(&output.stdout).to_string())
    } else {
        let stderr = String::from_utf8_lossy(&output.stderr);
        Err(format!("{cmd} failed: {stderr}"))
    }
}

/// Run a command inside a chroot (via sudo).
pub(super) fn chroot_cmd(mount_dir: &str, args: &[&str]) -> Result<String, String> {
    let mut full_args = vec!["chroot", mount_dir];
    full_args.extend_from_slice(args);

    tracing::debug!(args = ?full_args, "installer: chroot command (via sudo)");

    let output = Command::new("sudo")
        .args(&full_args)
        .env("PATH", "/usr/local/sbin:/usr/local/bin:/usr/sbin:/usr/bin:/sbin:/bin")
        .output()
        .map_err(|e| format!("failed to run sudo chroot: {e}"))?;

    if output.status.success() {
        Ok(String::from_utf8_lossy(&output.stdout).to_string())
    } else {
        let stderr = String::from_utf8_lossy(&output.stderr);
        Err(format!("chroot {}: {stderr}", args.first().unwrap_or(&"")))
    }
}

/// List block devices for error messages.
fn list_block_devices() -> String {
    Command::new("lsblk")
        .args(["-o", "NAME,SIZE,TYPE,MODEL"])
        .env("PATH", "/usr/local/sbin:/usr/local/bin:/usr/sbin:/usr/bin:/sbin:/bin")
        .output()
        .map(|o| String::from_utf8_lossy(&o.stdout).to_string())
        .unwrap_or_else(|_| "lsblk unavailable".into())
}

pub(crate) struct DiskInfo {
    /// Kernel name, e.g. "sda" — the picker shows it and the installer takes it.
    pub(crate) name: String,
    size: String,
    model: String,
    /// What is on the disk now, in words: "Empty", "3 partitions (ext4, ntfs)".
    contents: String,
    /// Anything at all is on it — erasing it loses something. Unknown counts as yes.
    has_data: bool,
}

/// Detect available disks and return structured info. Also what the onboarding
/// hardware scan measures its Disk row against, so the scan and the picker can
/// never disagree about which disks are candidates.
pub(crate) fn detect_disks() -> Vec<DiskInfo> {
    // Not `-d`: the partitions under each disk are what say whether it holds anything, and
    // "this disk has a Windows partition on it" is the one thing worth knowing before erasing it.
    let output = Command::new("lsblk")
        .args(["-J", "-o", "NAME,SIZE,MODEL,TYPE,RO,RM,FSTYPE", "-e", "7,11"])
        .env("PATH", "/usr/local/sbin:/usr/local/bin:/usr/sbin:/usr/bin:/sbin:/bin")
        .output();

    let Ok(output) = output else {
        tracing::warn!("lsblk failed for disk detection");
        return detect_disks_fallback();
    };
    if !output.status.success() {
        return detect_disks_fallback();
    }

    let disks = disks_from_lsblk(&String::from_utf8_lossy(&output.stdout));
    for d in &disks {
        tracing::info!(name = %d.name, size = %d.size, model = %d.model, contents = %d.contents, "Detected disk");
    }
    disks
}

/// The install candidates in `lsblk -J` output: whole, writable, fixed disks, each with a line
/// about what is on it. Kept apart from running lsblk so it can be tested on fixture text.
fn disks_from_lsblk(json: &str) -> Vec<DiskInfo> {
    let Ok(json) = serde_json::from_str::<serde_json::Value>(json) else { return Vec::new() };
    let Some(devices) = json["blockdevices"].as_array() else { return Vec::new() };

    // util-linux wrote these flags as "0"/"1" before it wrote them as booleans; reading only
    // one form turned every disk read-only on the other, and the installer found nothing.
    let flag = |v: &serde_json::Value| match v {
        serde_json::Value::Bool(b) => *b,
        serde_json::Value::String(s) => s == "1" || s == "true",
        serde_json::Value::Number(n) => n.as_u64() == Some(1),
        _ => true, // unknown: treat as read-only / removable, and skip it
    };

    let mut disks = Vec::new();
    for dev in devices {
        if dev["type"].as_str() != Some("disk") || flag(&dev["ro"]) || flag(&dev["rm"]) {
            continue;
        }
        let name = dev["name"].as_str().unwrap_or("").to_string();
        if name.is_empty() || name.starts_with("loop") || name.starts_with("sr") || name.starts_with("fd") {
            continue;
        }
        let size = dev["size"].as_str().unwrap_or("?").to_string();
        let model = dev["model"].as_str().unwrap_or("").trim().to_string();
        let model = if model.is_empty() { "Disk".to_string() } else { model };

        let parts: Vec<&serde_json::Value> = dev["children"]
            .as_array()
            .map(|c| c.iter().filter(|p| p["type"].as_str() == Some("part")).collect())
            .unwrap_or_default();
        let mut kinds: Vec<String> = Vec::new();
        for p in &parts {
            if let Some(fs) = p["fstype"].as_str().filter(|f| !f.is_empty()) {
                if !kinds.iter().any(|k| k == fs) {
                    kinds.push(fs.to_string());
                }
            }
        }
        let whole_disk_fs = dev["fstype"].as_str().filter(|f| !f.is_empty());
        let (contents, has_data) = match (parts.len(), whole_disk_fs) {
            (0, Some(fs)) => (format!("A whole-disk filesystem ({fs})"), true),
            (0, None) => ("Empty".to_string(), false),
            (n, _) => {
                let count = if n == 1 { "1 partition".to_string() } else { format!("{n} partitions") };
                if kinds.is_empty() {
                    (count, true)
                } else {
                    (format!("{count} ({})", kinds.join(", ")), true)
                }
            }
        };
        disks.push(DiskInfo { name, size, model, contents, has_data });
    }
    disks
}

/// Fallback disk detection without JSON.
fn detect_disks_fallback() -> Vec<DiskInfo> {
    let output = Command::new("lsblk")
        .args(["-dn", "-o", "NAME,SIZE,TYPE,RO,RM", "-e", "7,11"])
        .env("PATH", "/usr/local/sbin:/usr/local/bin:/usr/sbin:/usr/bin:/sbin:/bin")
        .output();

    let mut disks = Vec::new();
    let Ok(output) = output else { return disks; };

    let text = String::from_utf8_lossy(&output.stdout);
    for line in text.lines() {
        let parts: Vec<&str> = line.split_whitespace().collect();
        if parts.len() < 5 { continue; }
        let (name, size, dtype, ro, rm) = (parts[0], parts[1], parts[2], parts[3], parts[4]);
        if dtype != "disk" || ro == "1" || rm == "1" { continue; }
        if name.starts_with("loop") || name.starts_with("sr") || name.starts_with("fd") { continue; }

        disks.push(DiskInfo {
            name: name.to_string(),
            size: size.to_string(),
            model: "Disk".to_string(),
            // Nothing here can say what is on it, so it is not claimed to be empty.
            contents: "contents unknown".to_string(),
            has_data: true,
        });
    }
    disks
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Shaped like the llm section of config-default.yaml, which the image ships
    /// as /opt/yantrik/config.yaml: the primary endpoint first, a fallback block
    /// after it.
    const IMAGE_CONFIG: &str = "\
user_name: \"User\"

llm:
  backend: \"api\"
  api_base_url: \"http://127.0.0.1:8341/v1\"
  api_model: \"yantrik-4b\"
  max_tokens: 1024
  fallback:
    backend: \"api\"
    api_base_url: \"http://127.0.0.1:8341/v1\"
    api_model: \"yantrik-4b\"
";

    fn wizard_state(provider: &str, base_url: &str, key: &str) -> InstallerState {
        InstallerState {
            username: "ada".into(),
            full_name: "Ada Lovelace".into(),
            ai_provider: provider.into(),
            ai_base_url: base_url.into(),
            ai_api_key: key.into(),
            ..InstallerState::default()
        }
    }

    #[test]
    fn installed_config_carries_the_wizard_choice() {
        let out = installed_config_yaml(
            IMAGE_CONFIG,
            &wizard_state("openai", "https://api.openai.com/v1", "sk-secret-123"),
        );
        assert!(out.contains("user_name: \"Ada Lovelace\""), "{out}");
        assert!(out.contains("api_base_url: \"https://api.openai.com/v1\""), "{out}");
        assert!(out.contains("api_model: \"gpt-4o-mini\""), "{out}");
        // Only the primary endpoint is replaced; the fallback block is the
        // image's own and the wizard said nothing about it.
        assert!(out.contains("    api_base_url: \"http://127.0.0.1:8341/v1\""), "{out}");
    }

    #[test]
    fn saved_endpoint_wins_over_the_preset() {
        // A remote Ollama the wizard saved, where the preset says localhost.
        let out = installed_config_yaml(
            IMAGE_CONFIG,
            &wizard_state("ollama", "http://192.168.4.35:11434/v1", ""),
        );
        assert!(out.contains("api_base_url: \"http://192.168.4.35:11434/v1\""), "{out}");
    }

    #[test]
    fn the_key_never_reaches_the_world_readable_config() {
        let out = installed_config_yaml(
            IMAGE_CONFIG,
            &wizard_state("openai", "https://api.openai.com/v1", "sk-secret-123"),
        );
        assert!(!out.contains("sk-secret-123"), "{out}");
        assert!(!out.contains("api_key"), "{out}");
    }

    #[test]
    fn skipped_ai_leaves_the_image_default() {
        let out = installed_config_yaml(IMAGE_CONFIG, &InstallerState::default());
        assert!(out.contains("api_base_url: \"http://127.0.0.1:8341/v1\""), "{out}");
        assert!(installed_providers_yaml(&InstallerState::default()).is_none());
    }

    #[test]
    fn providers_yaml_carries_the_key_where_settings_keeps_it() {
        let yaml = installed_providers_yaml(&wizard_state(
            "openai",
            "https://api.openai.com/v1",
            "sk-secret-123",
        ))
        .expect("a chosen provider produces a store");
        let store: ProviderStore = serde_yaml::from_str(&yaml).expect("valid YAML");
        let primary = store.primary().expect("the entry is primary");
        assert_eq!(primary.provider_type, "openai");
        assert_eq!(primary.base_url, "https://api.openai.com/v1");
        assert_eq!(primary.api_key.as_deref(), Some("sk-secret-123"));
        assert_eq!(primary.auth_type, "bearer");
    }

    #[test]
    fn anthropic_keys_are_marked_for_the_header_they_use() {
        let yaml = installed_providers_yaml(&wizard_state(
            "anthropic",
            "https://api.anthropic.com/v1",
            "sk-ant-1",
        ))
        .expect("a chosen provider produces a store");
        let store: ProviderStore = serde_yaml::from_str(&yaml).expect("valid YAML");
        assert_eq!(store.primary().expect("primary").auth_type, "x-api-key");
    }

    /// `lsblk -J -o NAME,SIZE,MODEL,TYPE,RO,RM,FSTYPE` on a machine with a used disk, an
    /// empty one, a USB stick and the ISO's CD drive.
    const LSBLK: &str = r#"{"blockdevices": [
        {"name":"sda","size":"80G","model":"VBOX HARDDISK","type":"disk","ro":false,"rm":false,"fstype":null,
         "children":[
            {"name":"sda1","size":"512M","model":null,"type":"part","ro":false,"rm":false,"fstype":"vfat"},
            {"name":"sda2","size":"60G","model":null,"type":"part","ro":false,"rm":false,"fstype":"ntfs"},
            {"name":"sda3","size":"19G","model":null,"type":"part","ro":false,"rm":false,"fstype":"ntfs"}
         ]},
        {"name":"nvme0n1","size":"476.9G","model":"Samsung SSD 980  ","type":"disk","ro":false,"rm":false,"fstype":null},
        {"name":"sdb","size":"14.6G","model":"USB Stick","type":"disk","ro":false,"rm":true,"fstype":"iso9660"},
        {"name":"sr0","size":"1.2G","model":"CD-ROM","type":"rom","ro":false,"rm":true,"fstype":"iso9660"}
    ]}"#;

    #[test]
    fn the_disk_list_says_what_each_disk_holds() {
        let disks = disks_from_lsblk(LSBLK);
        let names: Vec<&str> = disks.iter().map(|d| d.name.as_str()).collect();
        assert_eq!(names, ["sda", "nvme0n1"], "removable media and the CD are not candidates");

        assert_eq!(disks[0].contents, "3 partitions (vfat, ntfs)");
        assert!(disks[0].has_data);
        assert_eq!(disks[1].contents, "Empty");
        assert!(!disks[1].has_data);
        assert_eq!(disks[1].model, "Samsung SSD 980", "lsblk pads models with spaces");
    }

    #[test]
    fn older_lsblk_flags_are_read_too() {
        // util-linux before 2.33 printed the flags as strings.
        let old = r#"{"blockdevices":[
            {"name":"sda","size":"20G","model":"QEMU HARDDISK","type":"disk","ro":"0","rm":"0","fstype":"ext4"},
            {"name":"sdb","size":"8G","model":"Flash","type":"disk","ro":"0","rm":"1","fstype":null}
        ]}"#;
        let disks = disks_from_lsblk(old);
        assert_eq!(disks.len(), 1);
        assert_eq!(disks[0].contents, "A whole-disk filesystem (ext4)");
        assert!(disks[0].has_data);
    }

    #[test]
    fn nonsense_from_lsblk_is_no_disks_not_a_panic() {
        assert!(disks_from_lsblk("").is_empty());
        assert!(disks_from_lsblk("{}").is_empty());
        assert!(disks_from_lsblk(r#"{"blockdevices":[{"type":"disk"}]}"#).is_empty());
    }

    #[test]
    fn an_unusable_layout_installs_as_us() {
        let mut state = InstallerState { keyboard: "de".into(), ..InstallerState::default() };
        assert_eq!(installed_layout(&state), "de");
        state.keyboard = "de\nXKB_DEFAULT_OPTIONS=x".into();
        assert_eq!(installed_layout(&state), "us");
        state.keyboard.clear();
        assert_eq!(installed_layout(&state), "us");
    }
}
