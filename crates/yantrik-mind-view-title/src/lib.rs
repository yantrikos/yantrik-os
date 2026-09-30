//! Mind View's window title (#239).
//!
//! Mind View is a nested labwc: on the wlroots Wayland backend it is one window on the person's
//! desktop. labwc names that window itself, `wlr_wl_output_set_title(output, "labwc - WL-1")`, with
//! no setting to change it, so the person's title bar read "labwc - WL-1" over the mind's desktop.
//!
//! This library is preloaded (`LD_PRELOAD`) into that one labwc and nothing else. It answers the
//! call before wlroots does and passes the real function the title the shell chose
//! (`YANTRIK_MIND_VIEW_TITLE`, "Mind View" if unset). Every other symbol resolves as before.

use std::ffi::{c_char, c_void, CStr, CString};
use std::sync::OnceLock;

type SetTitle = unsafe extern "C" fn(*mut c_void, *const c_char);

/// wlroots' own `wlr_wl_output_set_title`, the next one after this library.
fn real_set_title() -> Option<SetTitle> {
    static REAL: OnceLock<Option<usize>> = OnceLock::new();
    let addr = *REAL.get_or_init(|| {
        // SAFETY: dlsym with RTLD_NEXT and a NUL-terminated name; a null result is handled.
        let p = unsafe { libc::dlsym(libc::RTLD_NEXT, c"wlr_wl_output_set_title".as_ptr()) };
        (!p.is_null()).then_some(p as usize)
    });
    // SAFETY: the address is wlroots' function of exactly this signature.
    addr.map(|a| unsafe { std::mem::transmute::<usize, SetTitle>(a) })
}

/// The title the shell asked for.
fn chosen_title() -> &'static CStr {
    static TITLE: OnceLock<CString> = OnceLock::new();
    TITLE.get_or_init(|| {
        let t = std::env::var("YANTRIK_MIND_VIEW_TITLE").unwrap_or_default();
        let t = if t.trim().is_empty() { "Mind View".to_string() } else { t };
        CString::new(t.replace('\0', "")).unwrap_or_else(|_| c"Mind View".to_owned())
    })
}

/// # Safety
/// Called by labwc with wlroots' own arguments; `output` is passed through untouched.
#[no_mangle]
pub unsafe extern "C" fn wlr_wl_output_set_title(output: *mut c_void, _title: *const c_char) {
    if let Some(real) = real_set_title() {
        // SAFETY: the real function, with the output it was given and a live C string.
        unsafe { real(output, chosen_title().as_ptr()) }
    }
}
