//! Which desktop entries are LibreOffice, for the rule that keeps the person's LibreOffice closed to
//! agents (#942, #953).
//!
//! LibreOffice evaluates what it opens (links, macros, fields) with the rights of the process that
//! opens it, and the person's has all of theirs. An agent's work goes through the adapter's own
//! actions instead, which run a LibreOffice of their own in a sandbox on a copy of the file
//! (adapters/libreoffice). This is the rule for the shell's own routes that would open a file, or
//! the app, in the person's LibreOffice. It decides by the handler an entry resolves to, not by the
//! file's extension, so a renamed file or a changed default cannot get round it.

use crate::apps::{exec_argv, DesktopEntry};

/// The refusal an agent or the Mind is given, naming the route it can take instead.
pub const CLOSED_TO_AGENTS: &str = "An agent does not open files in the person's LibreOffice, which runs \
     with all of the person's rights. While LibreOffice is open, its own actions (`yos act \
     libreoffice open`) work on a copy in a sandbox instead (#953). Nothing was opened.";

/// Programs and ids that are LibreOffice, whichever way an entry spells them.
const NAMES: &[&str] = &[
    "soffice", "libreoffice", "lowriter", "localc", "loimpress", "lodraw", "lomath", "lobase", "oowriter",
    "oocalc", "ooimpress", "oodraw",
];

/// Whether `entry` starts LibreOffice: by its id, its declared surface, or any word of its `Exec`
/// line (so `env FOO=1 soffice --writer %U` and `/usr/lib/libreoffice/program/soffice.bin` count).
pub fn is_libreoffice(entry: &DesktopEntry) -> bool {
    let named = |s: &str| {
        let s = s.to_lowercase();
        NAMES.iter().any(|n| s.contains(n))
    };
    named(&entry.app_id)
        || entry.surface.as_deref().is_some_and(named)
        || entry.aliases.iter().any(|a| named(a))
        || exec_argv(&entry.exec, None).iter().any(|w| named(w.rsplit('/').next().unwrap_or(w)))
}

/// Whether a file name or MIME type is one of the office families LibreOffice is the default for.
pub fn is_office_document(name: &str, mime: Option<&str>) -> bool {
    const EXT: &[&str] = &[
        "odt", "ods", "odp", "odg", "doc", "docx", "xls", "xlsx", "ppt", "pptx", "rtf", "csv",
    ];
    let ext = name.rsplit_once('.').map(|(_, e)| e.to_lowercase()).unwrap_or_default();
    let mime = mime.unwrap_or_default().to_lowercase();
    EXT.contains(&ext.as_str())
        || mime.contains("opendocument")
        || mime.contains("officedocument")
        || mime.contains("msword")
        || mime.contains("ms-excel")
        || mime.contains("ms-powerpoint")
        || mime == "application/rtf"
        || mime == "text/rtf"
        || mime == "text/csv"
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entry(app_id: &str, exec: &str) -> DesktopEntry {
        DesktopEntry { app_id: app_id.into(), exec: exec.into(), ..Default::default() }
    }

    #[test]
    fn libreoffice_is_known_by_id_surface_or_exec() {
        assert!(is_libreoffice(&entry("yantrik-libreoffice", "/usr/bin/yantrik-libreoffice %U")));
        assert!(is_libreoffice(&entry("writer", "env A=1 /usr/lib/libreoffice/program/soffice.bin --writer %U")));
        assert!(is_libreoffice(&entry("office", "localc %F")));
        let mut other = entry("sheets", "/opt/x/run");
        other.surface = Some("libreoffice".into());
        assert!(is_libreoffice(&other));
        assert!(!is_libreoffice(&entry("yantrik-text-editor", "yantrik-text-editor %f")));
    }

    #[test]
    fn the_office_families_are_known_by_extension_and_mime() {
        for n in ["a.odt", "a.ODS", "a.odp", "a.odg", "a.doc", "a.docx", "a.xls", "a.xlsx", "a.ppt", "a.pptx", "a.rtf", "a.csv"] {
            assert!(is_office_document(n, None), "{n}");
        }
        assert!(is_office_document("noext", Some("application/vnd.oasis.opendocument.text")));
        assert!(is_office_document("noext", Some("application/vnd.openxmlformats-officedocument.wordprocessingml.document")));
        assert!(!is_office_document("a.txt", Some("text/plain")));
    }
}
