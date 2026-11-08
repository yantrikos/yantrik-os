//! The syscall filter a sealed command runs under (#1124, M4).
//!
//! bubblewrap's `--seccomp FD` takes a compiled classic-BPF program. There is no `libseccomp` in
//! this tree and the program is small, so it is assembled here, byte for byte, and tested by an
//! interpreter of the instruction forms it uses.
//!
//! # What it denies
//!
//! A denylist, because a build runs anything: compilers, Node, Chromium, `ffmpeg`. Every entry is
//! a way to reach the kernel's larger attack surface or to undo the sandbox, and none is a call a
//! build makes:
//!
//! - `io_uring_*`, `bpf`, `perf_event_open`, `userfaultfd`, `keyctl`/`add_key`/`request_key`: the
//!   interfaces most unprivileged kernel escalations start from;
//! - `unshare`, `setns`, and `clone` with any `CLONE_NEW*` flag (`clone3` answers `ENOSYS`, which
//!   libc turns into the `clone` fallback, whose flags can be read): no nested namespaces, so no
//!   fresh capabilities in a user namespace of its own. `--disable-userns` says the same to the
//!   kernel; this holds where that flag is not;
//! - `mount`, the new mount API, `pivot_root`, `chroot`, module and kexec loading, `swapon`,
//!   `reboot`, `acct`, `quotactl`, `open_by_handle_at`, `process_vm_*`;
//! - `socket(AF_VSOCK)`: a virtual socket reaches the hypervisor and the guest's own listeners
//!   whatever the network namespace says;
//! - `removexattr` and its two variants, so a sealed command cannot strip the marker the shell
//!   puts on a workshop's files (`planted`).
//!
//! A call on the list answers `EPERM`, not a kill, so a tool that probes for it carries on. A call
//! from a foreign ABI (32-bit on x86-64, the x32 range) is refused the same way: its numbers mean
//! different calls and the list would not apply.
//!
//! Landlock is not added: the mount namespace already limits the filesystem to the workshop and
//! the read-only system, and a second layer for the same thing would be a second thing to keep in
//! step with the first. Little-endian only (x86-64 and aarch64, which this OS ships); any other
//! machine builds no filter, and so runs nothing sealed.

use std::os::fd::{AsRawFd, FromRawFd, OwnedFd, RawFd};

const AUDIT_ARCH_X86_64: u32 = 0xC000_003E;
const AUDIT_ARCH_AARCH64: u32 = 0xC000_00B7;
const RET_ALLOW: u32 = 0x7fff_0000;
const RET_ERRNO: u32 = 0x0005_0000;

const LD_ABS: u16 = 0x20;
const JEQ: u16 = 0x15;
const JGE: u16 = 0x35;
const JSET: u16 = 0x45;
const RET: u16 = 0x06;

/// `CLONE_NEWNS | NEWCGROUP | NEWUTS | NEWIPC | NEWUSER | NEWPID | NEWNET | NEWTIME`.
const CLONE_NEW_FLAGS: u32 =
    0x0002_0000 | 0x0200_0000 | 0x0400_0000 | 0x0800_0000 | 0x1000_0000 | 0x2000_0000 | 0x4000_0000 | 0x80;

/// Calls answered with an errno. The numbers are the libc crate's for the machine it is built for.
fn denied() -> Vec<(u32, u32)> {
    let eperm = libc::EPERM as u32;
    let mut list: Vec<(u32, u32)> = [
        libc::SYS_io_uring_setup,
        libc::SYS_io_uring_enter,
        libc::SYS_io_uring_register,
        libc::SYS_bpf,
        libc::SYS_perf_event_open,
        libc::SYS_userfaultfd,
        libc::SYS_keyctl,
        libc::SYS_add_key,
        libc::SYS_request_key,
        libc::SYS_unshare,
        libc::SYS_setns,
        libc::SYS_mount,
        libc::SYS_umount2,
        libc::SYS_pivot_root,
        libc::SYS_chroot,
        libc::SYS_fsopen,
        libc::SYS_fsconfig,
        libc::SYS_fsmount,
        libc::SYS_fspick,
        libc::SYS_move_mount,
        libc::SYS_open_tree,
        libc::SYS_mount_setattr,
        libc::SYS_kexec_load,
        libc::SYS_kexec_file_load,
        libc::SYS_init_module,
        libc::SYS_finit_module,
        libc::SYS_delete_module,
        libc::SYS_swapon,
        libc::SYS_swapoff,
        libc::SYS_reboot,
        libc::SYS_acct,
        libc::SYS_quotactl,
        libc::SYS_open_by_handle_at,
        libc::SYS_process_vm_readv,
        libc::SYS_process_vm_writev,
        libc::SYS_removexattr,
        libc::SYS_lremovexattr,
        libc::SYS_fremovexattr,
    ]
    .into_iter()
    .map(|n| (n as u32, eperm))
    .collect();
    // libc falls back from clone3 to clone on ENOSYS; clone's flags can be read, clone3's cannot.
    list.push((libc::SYS_clone3 as u32, libc::ENOSYS as u32));
    list
}

/// `AF_VSOCK`: a socket to the hypervisor or to a listener in the guest, past the network namespace.
const AF_VSOCK: u32 = 40;

fn insn(code: u16, jt: u8, jf: u8, k: u32) -> [u8; 8] {
    let mut b = [0u8; 8];
    b[0..2].copy_from_slice(&code.to_le_bytes());
    b[2] = jt;
    b[3] = jf;
    b[4..8].copy_from_slice(&k.to_le_bytes());
    b
}

/// The compiled program: an array of `struct sock_filter`.
pub(crate) fn program() -> Result<Vec<u8>, String> {
    #[cfg(not(target_endian = "little"))]
    return Err("sealed mode has no syscall filter for a big-endian machine; nothing was run.".into());
    #[cfg(target_endian = "little")]
    {
        let arch = if cfg!(target_arch = "x86_64") {
            AUDIT_ARCH_X86_64
        } else if cfg!(target_arch = "aarch64") {
            AUDIT_ARCH_AARCH64
        } else {
            return Err("sealed mode has no syscall filter for this architecture; nothing was run.".into());
        };
        let errno = |e: u32| RET_ERRNO | e;
        let eperm = errno(libc::EPERM as u32);
        let mut p: Vec<[u8; 8]> = vec![
            insn(LD_ABS, 0, 0, 4),
            insn(JEQ, 1, 0, arch),
            insn(RET, 0, 0, eperm),
            insn(LD_ABS, 0, 0, 0),
        ];
        if cfg!(target_arch = "x86_64") {
            // The x32 ABI: numbers with bit 30 set are other calls.
            p.push(insn(JGE, 0, 1, 0x4000_0000));
            p.push(insn(RET, 0, 0, eperm));
        }
        for (nr, e) in denied() {
            p.push(insn(JEQ, 0, 1, nr));
            p.push(insn(RET, 0, 0, errno(e)));
        }
        // clone: refused when it asks for any new namespace. A holds the flags afterwards, so the
        // number is loaded again for whatever follows.
        p.push(insn(JEQ, 0, 4, libc::SYS_clone as u32));
        p.push(insn(LD_ABS, 0, 0, 16));
        p.push(insn(JSET, 0, 1, CLONE_NEW_FLAGS));
        p.push(insn(RET, 0, 0, eperm));
        p.push(insn(LD_ABS, 0, 0, 0));
        // socket: refused for AF_VSOCK, which `--unshare-net` does not isolate (#1124, N6). The
        // same shape as clone: arg0 is the domain; any other domain, and any other call, lands on
        // the final allow with the number still in A.
        p.push(insn(JEQ, 0, 4, libc::SYS_socket as u32));
        p.push(insn(LD_ABS, 0, 0, 16));
        p.push(insn(JEQ, 0, 1, AF_VSOCK));
        p.push(insn(RET, 0, 0, eperm));
        p.push(insn(LD_ABS, 0, 0, 0));
        p.push(insn(RET, 0, 0, RET_ALLOW));
        if p.len() > 4096 {
            return Err("the syscall filter is too long".into());
        }
        Ok(p.concat())
    }
}

/// The program on the read end of a pipe, for `bwrap --seccomp`. The writer is closed, so the
/// reader sees the program and then the end.
pub(crate) fn as_pipe() -> Result<OwnedFd, String> {
    let bytes = program()?;
    let mut fds = [0 as RawFd; 2];
    if unsafe { libc::pipe2(fds.as_mut_ptr(), libc::O_CLOEXEC) } != 0 {
        return Err(format!("no pipe for the syscall filter: {}", std::io::Error::last_os_error()));
    }
    let (read, write) = unsafe { (OwnedFd::from_raw_fd(fds[0]), OwnedFd::from_raw_fd(fds[1])) };
    let wrote = unsafe { libc::write(write.as_raw_fd(), bytes.as_ptr().cast(), bytes.len()) };
    if wrote < 0 || wrote as usize != bytes.len() {
        return Err("the syscall filter could not be written".into());
    }
    drop(write);
    Ok(read)
}

#[cfg(all(test, target_endian = "little"))]
mod tests {
    use super::*;

    /// Run the program the way the kernel would, for one call.
    fn run(prog: &[u8], arch: u32, nr: u32, arg0: u32) -> u32 {
        let words: Vec<(u16, u8, u8, u32)> = prog
            .chunks(8)
            .map(|c| (u16::from_le_bytes([c[0], c[1]]), c[2], c[3], u32::from_le_bytes([c[4], c[5], c[6], c[7]])))
            .collect();
        let (mut a, mut pc) = (0u32, 0usize);
        loop {
            let (code, jt, jf, k) = words[pc];
            pc += 1;
            match code {
                LD_ABS => {
                    a = match k {
                        0 => nr,
                        4 => arch,
                        16 => arg0,
                        other => panic!("offset {other}"),
                    }
                }
                JEQ | JGE | JSET => {
                    let hit = match code {
                        JEQ => a == k,
                        JGE => a >= k,
                        _ => a & k != 0,
                    };
                    pc += usize::from(if hit { jt } else { jf });
                }
                RET => return k,
                other => panic!("opcode {other:#x}"),
            }
        }
    }

    fn native() -> u32 {
        if cfg!(target_arch = "x86_64") {
            AUDIT_ARCH_X86_64
        } else {
            AUDIT_ARCH_AARCH64
        }
    }

    #[test]
    fn the_listed_calls_are_refused_and_ordinary_ones_pass() {
        let prog = program().unwrap();
        assert_eq!(prog.len() % 8, 0);
        let eperm = RET_ERRNO | libc::EPERM as u32;
        for nr in [
            libc::SYS_io_uring_setup,
            libc::SYS_bpf,
            libc::SYS_unshare,
            libc::SYS_setns,
            libc::SYS_mount,
            libc::SYS_keyctl,
            libc::SYS_userfaultfd,
            libc::SYS_removexattr,
            libc::SYS_fremovexattr,
        ] {
            assert_eq!(run(&prog, native(), nr as u32, 0), eperm, "{nr}");
        }
        assert_eq!(run(&prog, native(), libc::SYS_clone3 as u32, 0), RET_ERRNO | libc::ENOSYS as u32);
        for nr in [libc::SYS_read, libc::SYS_write, libc::SYS_openat, libc::SYS_execve, libc::SYS_mmap, libc::SYS_futex, libc::SYS_setxattr] {
            assert_eq!(run(&prog, native(), nr as u32, 0), RET_ALLOW, "{nr}");
        }
    }

    #[test]
    fn clone_may_not_ask_for_a_namespace() {
        let prog = program().unwrap();
        let clone = libc::SYS_clone as u32;
        // A thread and a plain fork pass.
        assert_eq!(run(&prog, native(), clone, 0x0000_0100 | 0x0000_0200 | 0x0001_0000), RET_ALLOW);
        assert_eq!(run(&prog, native(), clone, 17), RET_ALLOW);
        for flag in [0x1000_0000u32, 0x2000_0000, 0x4000_0000, 0x0002_0000, 0x0800_0000, 0x0400_0000, 0x0200_0000, 0x80] {
            assert_eq!(run(&prog, native(), clone, 17 | flag), RET_ERRNO | libc::EPERM as u32, "{flag:#x}");
        }
    }

    #[test]
    fn a_vsock_socket_is_refused_and_an_ordinary_one_is_not() {
        let prog = program().unwrap();
        let socket = libc::SYS_socket as u32;
        assert_eq!(run(&prog, native(), socket, 40), RET_ERRNO | libc::EPERM as u32);
        for domain in [libc::AF_UNIX as u32, libc::AF_INET as u32, libc::AF_INET6 as u32, libc::AF_NETLINK as u32] {
            assert_eq!(run(&prog, native(), socket, domain), RET_ALLOW, "{domain}");
        }
        // A call after the block is still judged by its own number.
        assert_eq!(run(&prog, native(), libc::SYS_mount as u32, 0), RET_ERRNO | libc::EPERM as u32);
    }

    #[test]
    fn another_abi_is_refused_whole() {
        let prog = program().unwrap();
        let eperm = RET_ERRNO | libc::EPERM as u32;
        assert_eq!(run(&prog, 0x4000_0003, libc::SYS_read as u32, 0), eperm, "32-bit");
        #[cfg(target_arch = "x86_64")]
        assert_eq!(run(&prog, native(), 0x4000_0000 | libc::SYS_read as u32, 0), eperm, "x32");
    }

    #[test]
    fn the_pipe_carries_the_whole_program_and_ends() {
        use std::io::Read;
        let mut file = std::fs::File::from(as_pipe().unwrap());
        let mut got = Vec::new();
        file.read_to_end(&mut got).unwrap();
        assert_eq!(got, program().unwrap());
    }
}
