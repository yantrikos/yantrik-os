//! Real disk installer — partitions, copies live system, installs GRUB, creates user.
//!
//! This runs the same operations as the text-based `yantrik-install` script but
//! from within the Slint UI, reporting progress back via a callback.

use slint::ComponentHandle;
use std::process::Command;

use crate::app_context::AppContext;
use crate::App;

/// Wire the installer callbacks.
pub fn wire(ui: &App, _ctx: &AppContext) {
    // Detect installer mode
    if std::path::Path::new("/opt/yantrik/.installer-mode").exists() {
        ui.set_onboard_installer_mode(true);
        tracing::info!("Installer mode detected — disk install UI enabled");
    }

    // Populate disk list for the UI
    if std::path::Path::new("/opt/yantrik/.installer-mode").exists() {
        let ui_weak_disks = ui.as_weak();
        std::thread::spawn(move || {
            let disks = detect_disks();
            let _ = slint::invoke_from_event_loop(move || {
                if let Some(ui) = ui_weak_disks.upgrade() {
                    if let Some(d) = disks.get(0) {
                        ui.set_onboard_disk_1_name(d.name.clone().into());
                        ui.set_onboard_disk_1_size(d.size.clone().into());
                        ui.set_onboard_disk_1_model(d.model.clone().into());
                        // Auto-select first disk
                        ui.set_onboard_selected_disk(d.name.clone().into());
                    }
                    if let Some(d) = disks.get(1) {
                        ui.set_onboard_disk_2_name(d.name.clone().into());
                        ui.set_onboard_disk_2_size(d.size.clone().into());
                        ui.set_onboard_disk_2_model(d.model.clone().into());
                    }
                    if let Some(d) = disks.get(2) {
                        ui.set_onboard_disk_3_name(d.name.clone().into());
                        ui.set_onboard_disk_3_size(d.size.clone().into());
                        ui.set_onboard_disk_3_model(d.model.clone().into());
                    }
                }
            });
        });
    }

    // Handle install-to-disk callback from onboarding UI
    let ui_weak = ui.as_weak();
    ui.on_onboard_install_to_disk(move |username, password, full_name, hostname, companion_name, target_disk| {
        let username = username.to_string();
        let password = password.to_string();
        let full_name = full_name.to_string();
        let hostname = hostname.to_string();
        let companion_name = companion_name.to_string();
        let target_disk = target_disk.to_string();
        let weak = ui_weak.clone();

        std::thread::spawn(move || {
            tracing::info!(
                username = %username,
                target_disk = %target_disk,
                hostname = %hostname,
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

            let state = InstallerState {
                username: username.clone(),
                password,
                full_name,
                hostname,
                language: String::new(),
                locale: String::new(),
                target_disk: target_disk.clone(),
                partition_scheme: "auto".into(),
                ai_provider: String::new(),
                ai_api_key: String::new(),
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
                            // Phase 12 (complete) is shown automatically by the UI
                            // when install-progress reaches 100
                        }
                        Err(e) => {
                            tracing::error!(error = %e, "Installation failed");
                            ui.set_onboard_install_status(format!("Installation failed: {e}").into());
                            ui.set_onboard_install_error(format!("Installation failed: {e}").into());
                            // Back to the summary, where the error is displayed and the Install
                            // button lives. The progress screen has no way off it, so failing
                            // there left the machine staring at a stalled bar.
                            ui.set_onboard_phase(10);
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

/// Shared installer state collected across onboarding callbacks.
#[derive(Debug, Clone, Default)]
pub struct InstallerState {
    pub language: String,
    pub locale: String,
    pub username: String,
    pub full_name: String,
    pub password: String,
    pub hostname: String,
    pub target_disk: String,      // e.g. "sda"
    pub partition_scheme: String,  // "auto" or "manual"
    pub ai_provider: String,
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

    // ── Step 1: Partition disk ────────────────────────────────────
    progress(2, "Partitioning disk...");
    run_cmd("parted", &["-s", &disk, "mklabel", "gpt"])?;

    let (efi_part, root_part) = if is_efi {
        // GPT + EFI: partition 1 = EFI (512M), partition 2 = root
        run_cmd("parted", &["-s", &disk, "mkpart", "EFI", "fat32", "1MiB", "513MiB"])?;
        run_cmd("parted", &["-s", &disk, "set", "1", "esp", "on"])?;
        run_cmd("parted", &["-s", &disk, "mkpart", "root", "ext4", "513MiB", "100%"])?;
        partition_names(&disk, 1, 2)
    } else {
        // GPT + BIOS: partition 1 = BIOS boot (1M), partition 2 = root (no EFI)
        run_cmd("parted", &["-s", &disk, "mkpart", "biosboot", "", "1MiB", "2MiB"])?;
        run_cmd("parted", &["-s", &disk, "set", "1", "bios_grub", "on"])?;
        run_cmd("parted", &["-s", &disk, "mkpart", "root", "ext4", "2MiB", "100%"])?;
        // No EFI partition in BIOS mode
        (String::new(), partition_name(&disk, 2))
    };
    progress(8, "Disk partitioned");

    // Wait for partition devices to appear
    let _ = run_cmd("partprobe", &[&disk]);
    std::thread::sleep(std::time::Duration::from_secs(2));

    if !std::path::Path::new(&root_part).exists() {
        let _ = run_cmd("udevadm", &["settle", "--timeout=5"]);
        std::thread::sleep(std::time::Duration::from_secs(1));
    }

    // ── Step 2: Format filesystems ──────────────────────────────
    if is_efi && !efi_part.is_empty() {
        progress(10, "Formatting EFI partition (FAT32)...");
        run_cmd("mkfs.fat", &["-F32", &efi_part])?;
    }

    progress(12, "Formatting root partition (ext4)...");
    run_cmd("mkfs.ext4", &["-q", "-L", "YANTRIK", &root_part])?;
    progress(15, "Filesystems formatted");

    // ── Step 3: Mount target ────────────────────────────────────
    let mount_dir = "/mnt/yantrik-install";
    progress(16, "Mounting target filesystem...");
    run_cmd("mkdir", &["-p", mount_dir])?;
    run_cmd("mount", &[&root_part, mount_dir])?;

    if is_efi && !efi_part.is_empty() {
        let efi_mount = format!("{mount_dir}/boot/efi");
        run_cmd("mkdir", &["-p", &efi_mount])?;
        run_cmd("mount", &[&efi_part, &efi_mount])?;
    }
    progress(18, "Target mounted");

    // From here, ensure we clean up on failure
    let result = install_to_target(state, &disk, &efi_part, is_efi, mount_dir, &progress);

    // ── Cleanup: unmount everything (lazy to avoid "device busy") ──
    // Kill any processes still using the mount
    let _ = run_cmd("fuser", &["-km", mount_dir]);
    std::thread::sleep(std::time::Duration::from_secs(1));

    // Unmount in reverse order, lazy flag to avoid EBUSY
    let _ = run_cmd("umount", &["-Rl", &format!("{mount_dir}/sys")]);
    let _ = run_cmd("umount", &["-Rl", &format!("{mount_dir}/proc")]);
    let _ = run_cmd("umount", &["-Rl", &format!("{mount_dir}/dev")]);
    let _ = run_cmd("umount", &["-l", &format!("{mount_dir}/boot/efi")]);
    let _ = run_cmd("umount", &["-l", mount_dir]);

    // Sync to flush writes
    let _ = run_cmd("sync", &[]);

    result
}

/// Core installation steps (after mount, before unmount).
fn install_to_target(
    state: &InstallerState,
    disk: &str,
    efi_part: &str,
    is_efi: bool,
    mount_dir: &str,
    progress: &ProgressFn,
) -> Result<(), String> {
    // ── Step 4: Copy live system via rsync ──────────────────────
    copy_system(mount_dir, progress)?;
    progress(55, "System files copied");

    // ── Step 5: Generate fstab ──────────────────────────────────
    progress(58, "Configuring filesystem table...");
    let mut fstab = format!("LABEL=YANTRIK  /           ext4  defaults,noatime  0  1\n");
    if is_efi && !efi_part.is_empty() {
        fstab.push_str(&format!("{efi_part}     /boot/efi   vfat  defaults          0  2\n"));
    }
    sudo_write(&format!("{mount_dir}/etc/fstab"), &fstab)?;

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

    // Configure GRUB defaults
    let _ = sudo_write(
        &format!("{mount_dir}/etc/default/grub"),
        "GRUB_DEFAULT=0\nGRUB_TIMEOUT=3\nGRUB_DISTRIBUTOR=\"Yantrik OS\"\nGRUB_CMDLINE_LINUX_DEFAULT=\"quiet splash\"\nGRUB_CMDLINE_LINUX=\"console=tty1 console=ttyS0,115200\"\nGRUB_TERMINAL=\"console serial\"\nGRUB_SERIAL_COMMAND=\"serial --speed=115200\"\n",
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

    // Ensure log directory exists and is writable
    let _ = run_cmd("mkdir", &["-p", &format!("{mount_dir}/opt/yantrik/logs")]);
    let _ = run_cmd("chmod", &["777", &format!("{mount_dir}/opt/yantrik/logs")]);

    // Regenerate initramfs without live-boot hooks
    progress(93, "Rebuilding initramfs...");
    let _ = chroot_cmd(mount_dir, &["update-initramfs", "-u"]);

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
    sudo_write(&format!("{dst_home}/.bash_profile"), INSTALLED_BASH_PROFILE)?;

    // labwc environment. No renderer in it: yantrik-session decides GPU or software per login.
    let labwc_dir = format!("{dst_home}/.config/labwc");
    let _ = run_cmd("mkdir", &["-p", &labwc_dir]);
    sudo_write(&format!("{labwc_dir}/environment"), INSTALLED_LABWC_ENVIRONMENT)?;

    // No autostart or rc.xml here. `yantrik-session` installs the shipped ones from
    // /opt/yantrik/share at every login. This installer used to write its own, and they drifted:
    // an installed machine drew the shell inside a window with a title bar and minimise, maximise
    // and close buttons, with none of the key bindings the real config carries.

    // The person has just answered onboarding, in this installer. Without the marker the
    // installed desktop opened on the same questions again, starting with their name.
    let _ = run_cmd("mkdir", &["-p", &format!("{dst_home}/.yantrik")]);
    sudo_write(&format!("{dst_home}/.yantrik/.onboarding_complete"), "done")?;

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

    // Passwordless sudo (needed for labwc/system operations)
    let sudoers_file = format!("{mount_dir}/etc/sudoers.d/{username}");
    let _ = sudo_write(&sudoers_file, &format!("{username} ALL=(ALL) NOPASSWD:ALL\n"));
    let _ = run_cmd("chmod", &["0440", &sudoers_file]);

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

/// The installed person's `.bash_profile`: start the session on tty1, and do not loop if it
/// cannot start.
const INSTALLED_BASH_PROFILE: &str = r#"# Auto-start Yantrik desktop on tty1
if [ "$(tty)" = "/dev/tty1" ] && [ -z "$WAYLAND_DISPLAY" ]; then
    export XDG_RUNTIME_DIR="/run/user/$(id -u)"
    mkdir -p "$XDG_RUNTIME_DIR"

    # Nothing about graphics here. yantrik-session decides GPU or software at every login, and
    # labwc reads ~/.config/labwc/environment itself, before it chooses a renderer. This profile
    # used to source that file and, without it, export WLR_RENDERER=pixman and
    # LIBGL_ALWAYS_SOFTWARE=1 -- every installed machine drew on the CPU, GPU or not.

    # Crash guard
    CRASH_FILE="/tmp/.yantrik-labwc-crash"
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
"#;

/// The installed person's `~/.config/labwc/environment`.
///
/// It used to carry `WLR_RENDERER=pixman` and `LIBGL_ALWAYS_SOFTWARE=1` for every machine, so an
/// installed laptop with an Intel, AMD or NVIDIA GPU drew its whole desktop on the CPU.
/// yantrik-session decides at every login now (deploy/yantrik-os/yantrik-session, "Graphics"),
/// and the first line is its mark: a file that carries it is one the session has taken charge
/// of, so a WLR_RENDERER a person adds later is theirs and is honoured. `YANTRIK_START_SCREEN=32`
/// boots to the graphical login screen.
const INSTALLED_LABWC_ENVIRONMENT: &str = "\
# yantrik-graphics: yantrik-session decides the renderer
# yantrik-session chooses the GPU or software at every login; `yantrik-session graphics` says
# what it would choose and why. To force one, add a line: YANTRIK_GRAPHICS=software or
# YANTRIK_GRAPHICS=gpu. WLR_RENDERER (labwc's renderer) and SLINT_BACKEND (the shell's) are
# honoured as written here.
XDG_SESSION_TYPE=wayland
QT_QPA_PLATFORM=wayland
MOZ_ENABLE_WAYLAND=1
YANTRIK_START_SCREEN=32
";

/// Write AI provider config and user_name into the installed system's config.yaml.
fn configure_ai(mount_dir: &str, state: &InstallerState) {
    let config_path = format!("{mount_dir}/opt/yantrik/config.yaml");
    let Ok(content) = std::fs::read_to_string(&config_path) else {
        return;
    };

    let mut new_content = content;

    // ── Write user_name ─────────────────────────────────────────
    let display_name = if !state.full_name.is_empty() {
        &state.full_name
    } else if !state.username.is_empty() {
        &state.username
    } else {
        "User"
    };

    if let Some(start) = new_content.find("user_name:") {
        // Replace existing user_name line
        if let Some(end) = new_content[start..].find('\n') {
            let line_end = start + end;
            new_content.replace_range(start..line_end, &format!("user_name: \"{}\"", display_name));
        }
    } else {
        // No user_name line exists — insert at the top of the file
        new_content.insert_str(0, &format!("user_name: \"{}\"\n", display_name));
    }

    // ── Write AI provider settings ──────────────────────────────
    if !state.ai_provider.is_empty() {
        // Resolve the API base URL for the provider
        let base_url = provider_base_url(&state.ai_provider);
        let model = provider_default_model(&state.ai_provider);

        // Replace api_base_url
        if let Some(start) = new_content.find("api_base_url:") {
            if let Some(end) = new_content[start..].find('\n') {
                let line_end = start + end;
                new_content.replace_range(start..line_end, &format!("api_base_url: \"{base_url}\""));
            }
        }

        // Replace api_model
        if let Some(start) = new_content.find("api_model:") {
            if let Some(end) = new_content[start..].find('\n') {
                let line_end = start + end;
                new_content.replace_range(start..line_end, &format!("api_model: \"{model}\""));
            }
        }

        // Write API key if provided
        if !state.ai_api_key.is_empty() {
            // Add api_key field after api_model line
            if let Some(pos) = new_content.find("api_model:") {
                if let Some(end) = new_content[pos..].find('\n') {
                    let insert_at = pos + end + 1;
                    new_content.insert_str(insert_at, &format!("  api_key: \"{}\"\n", state.ai_api_key));
                }
            }
        }
    }

    let _ = sudo_write(&config_path, &new_content);

    // ── Also write user_name to per-user settings.yaml ──────────
    let username = if state.username.is_empty() { "yantrik" } else { &state.username };
    let settings_dir = format!("{mount_dir}/home/{username}/.config/yantrik");
    let settings_path = format!("{settings_dir}/settings.yaml");
    let _ = run_cmd("mkdir", &["-p", &settings_dir]);

    let settings_content = if let Ok(existing) = std::fs::read_to_string(&settings_path) {
        let mut s = existing;
        if let Some(start) = s.find("user_name:") {
            if let Some(end) = s[start..].find('\n') {
                let line_end = start + end;
                s.replace_range(start..line_end, &format!("user_name: \"{}\"", display_name));
            }
        } else {
            s.insert_str(0, &format!("user_name: \"{}\"\n", display_name));
        }
        s
    } else {
        format!("user_name: \"{}\"\n", display_name)
    };

    let _ = sudo_write(&settings_path, &settings_content);
    // Fix ownership so the user can read/write their settings
    let _ = chroot_cmd(mount_dir, &["chown", "-R",
        &format!("{username}:{username}"),
        &format!("/home/{username}/.config/yantrik")]);
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
fn partition_name(disk: &str, num: u8) -> String {
    if disk.contains("nvme") || disk.contains("mmcblk") {
        format!("{disk}p{num}")
    } else {
        format!("{disk}{num}")
    }
}

/// Get two partition device names.
fn partition_names(disk: &str, n1: u8, n2: u8) -> (String, String) {
    (partition_name(disk, n1), partition_name(disk, n2))
}

/// Write a file via sudo tee (since direct fs::write lacks root perms).
fn sudo_write(path: &str, content: &str) -> Result<(), String> {
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

fn run_cmd(cmd: &str, args: &[&str]) -> Result<String, String> {
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
fn chroot_cmd(mount_dir: &str, args: &[&str]) -> Result<String, String> {
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

struct DiskInfo {
    name: String,
    size: String,
    model: String,
}

/// Detect available disks and return structured info.
fn detect_disks() -> Vec<DiskInfo> {
    // Use lsblk with JSON output for reliable parsing
    let output = Command::new("lsblk")
        .args(["-dn", "-o", "NAME,SIZE,MODEL,TYPE,RO,RM", "--json", "-e", "7,11"])
        .env("PATH", "/usr/local/sbin:/usr/local/bin:/usr/sbin:/usr/bin:/sbin:/bin")
        .output();

    let mut disks = Vec::new();

    let Ok(output) = output else {
        tracing::warn!("lsblk failed for disk detection");
        // Fallback: try non-JSON
        return detect_disks_fallback();
    };

    if !output.status.success() {
        return detect_disks_fallback();
    }

    let text = String::from_utf8_lossy(&output.stdout);
    // Parse JSON: {"blockdevices": [{"name":"sda","size":"80G","model":"VBOX HARDDISK","type":"disk","ro":false,"rm":false}, ...]}
    if let Ok(json) = serde_json::from_str::<serde_json::Value>(&text) {
        if let Some(devices) = json["blockdevices"].as_array() {
            for dev in devices {
                let dtype = dev["type"].as_str().unwrap_or("");
                if dtype != "disk" { continue; }

                let ro = dev["ro"].as_bool().unwrap_or(true);
                let rm = dev["rm"].as_bool().unwrap_or(true);
                if ro || rm { continue; }

                let name = dev["name"].as_str().unwrap_or("").to_string();
                if name.starts_with("loop") || name.starts_with("sr") || name.starts_with("fd") {
                    continue;
                }

                let size = dev["size"].as_str().unwrap_or("?").to_string();
                let model = dev["model"].as_str().unwrap_or("Unknown").trim().to_string();
                let model = if model.is_empty() { "Unknown".to_string() } else { model };

                tracing::info!(name = %name, size = %size, model = %model, "Detected disk");
                disks.push(DiskInfo { name, size, model });
            }
        }
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
        });
    }
    disks
}

#[cfg(test)]
mod graphics_tests {
    use super::*;

    /// The session's mark, read out of the script that checks for it, so the two cannot drift.
    fn session_mark() -> String {
        let script = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../deploy/yantrik-os/yantrik-session");
        let text = std::fs::read_to_string(&script).expect("the session script is in the tree");
        let line = text
            .lines()
            .find_map(|l| l.strip_prefix("GRAPHICS_ENV_MARK='"))
            .expect("yantrik-session defines GRAPHICS_ENV_MARK");
        line.trim_end_matches('\'').to_string()
    }

    #[test]
    fn an_installed_machine_is_not_told_to_draw_in_software() {
        for line in INSTALLED_LABWC_ENVIRONMENT.lines().chain(INSTALLED_BASH_PROFILE.lines()) {
            let line = line.trim();
            if line.starts_with('#') {
                continue;
            }
            for forced in ["WLR_RENDERER=", "LIBGL_ALWAYS_SOFTWARE=", "SLINT_BACKEND="] {
                assert!(!line.contains(forced), "the installer still forces a renderer: {line}");
            }
        }
    }

    #[test]
    fn the_installed_environment_carries_the_sessions_mark() {
        // Without it the session takes the file for an old image's and strips it, and a
        // WLR_RENDERER the person later adds would be read as the image's on the first start.
        assert_eq!(INSTALLED_LABWC_ENVIRONMENT.lines().next(), Some(session_mark().as_str()));
    }

    #[test]
    fn the_installed_profile_still_starts_the_session_and_guards_against_a_crash_loop() {
        assert!(INSTALLED_BASH_PROFILE.contains("/opt/yantrik/bin/yantrik-session"));
        assert!(INSTALLED_BASH_PROFILE.contains("CRASH_FILE"));
    }
}
