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
    let _ = run_cmd("umount", &["-l", &format!("{mount_dir}/sys")]);
    let _ = run_cmd("umount", &["-l", &format!("{mount_dir}/proc")]);
    let _ = run_cmd("umount", &["-l", &format!("{mount_dir}/dev")]);
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
    progress(20, "Copying system files (this takes a few minutes)...");
    run_cmd(
        "rsync",
        &[
            "-aAXH",
            "--exclude=/proc/*",
            "--exclude=/sys/*",
            "--exclude=/dev/*",
            "--exclude=/run/*",
            "--exclude=/tmp/*",
            "--exclude=/mnt/*",
            "--exclude=/live/*",
            "--exclude=/cdrom/*",
            "/",
            &format!("{mount_dir}/"),
        ],
    )?;
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
    run_cmd("mount", &["--bind", "/dev", &format!("{mount_dir}/dev")])?;
    run_cmd("mount", &["--bind", "/proc", &format!("{mount_dir}/proc")])?;
    run_cmd("mount", &["--bind", "/sys", &format!("{mount_dir}/sys")])?;

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
        tracing::info!("Installer: installing GRUB for EFI");
        chroot_cmd(
            mount_dir,
            &[
                "grub-install",
                "--target=x86_64-efi",
                "--efi-directory=/boot/efi",
                "--bootloader-id=yantrik",
                "--no-nvram",  // Don't try to update NVRAM (may fail in live env)
            ],
        )?;
    } else {
        tracing::info!("Installer: installing GRUB for BIOS on {disk}");
        chroot_cmd(
            mount_dir,
            &["grub-install", "--target=i386-pc", disk],
        )?;
    }

    // Brand the installed system as Yantrik OS (so GRUB says "Yantrik OS" not "Debian")
    let _ = sudo_write(
        &format!("{mount_dir}/etc/os-release"),
        "PRETTY_NAME=\"Yantrik OS\"\nNAME=\"Yantrik OS\"\nID=yantrik\nID_LIKE=debian\nVERSION_ID=\"0.3.0\"\nHOME_URL=\"https://yantrikos.com\"\n",
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
    labwc 2>/opt/yantrik/logs/labwc.log
    EXIT_TIME=$(date +%s)

    if [ $((EXIT_TIME - START_TIME)) -lt 5 ]; then
        echo "$EXIT_TIME" > "$CRASH_FILE"
    else
        rm -f "$CRASH_FILE"
    fi
fi
"#,
    )?;

    // labwc environment — software rendering for VBox/headless
    let labwc_dir = format!("{dst_home}/.config/labwc");
    let _ = run_cmd("mkdir", &["-p", &labwc_dir]);

    // YANTRIK_START_SCREEN=32 boots to the graphical login screen
    sudo_write(
        &format!("{labwc_dir}/environment"),
        "WLR_RENDERER=pixman\nWLR_RENDERER_ALLOW_SOFTWARE=1\nXDG_SESSION_TYPE=wayland\nQT_QPA_PLATFORM=wayland\nMOZ_ENABLE_WAYLAND=1\nSLINT_BACKEND=winit\nLIBGL_ALWAYS_SOFTWARE=1\nYANTRIK_START_SCREEN=32\n",
    )?;

    // labwc autostart — launch yantrik-ui
    sudo_write(
        &format!("{labwc_dir}/autostart"),
        "#!/bin/sh\n/opt/yantrik/bin/yantrik-ui /opt/yantrik/config.yaml >> /opt/yantrik/logs/yantrik-os.log 2>&1 &\n",
    )?;
    let _ = run_cmd("chmod", &["+x", &format!("{labwc_dir}/autostart")]);

    // labwc rc.xml — fullscreen, no decorations
    sudo_write(
        &format!("{labwc_dir}/rc.xml"),
        r#"<?xml version="1.0" encoding="UTF-8"?>
<labwc_config>
  <core><gap>0</gap></core>
  <theme><titlebar><height>0</height></titlebar></theme>
  <keyboard>
    <keybind key="A-Tab"><action name="NextWindow" /></keybind>
    <keybind key="A-F4"><action name="Close" /></keybind>
    <keybind key="W-t">
      <action name="Execute"><command>foot</command></action>
    </keybind>
  </keyboard>
  <windowRules>
    <windowRule identifier="*" title="*">
      <action name="Maximize" />
    </windowRule>
  </windowRules>
</labwc_config>
"#,
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
