#[path = "../../crates/yantrik-ui/src/config_store.rs"]
mod config_store;
// config_store publishes through the shell's one private-file writer.
#[path = "../../crates/yantrik-ui/src/private_file.rs"]
mod private_file;

#[cfg(test)]
mod tests {
    use super::config_store::*;
    use std::{fs, path::PathBuf, os::unix::fs::{PermissionsExt, symlink}, sync::atomic::{AtomicUsize, Ordering}};
    static NEXT:AtomicUsize=AtomicUsize::new(0);
    struct Fixture(PathBuf);
    impl Fixture {
        fn new()->Self {let p=std::env::temp_dir().join(format!("yantrik-settings-test-{}-{}",std::process::id(),NEXT.fetch_add(1,Ordering::Relaxed)));fs::create_dir(&p).unwrap();Self(p)}
        fn file(&self)->PathBuf{self.0.join("settings.yaml")}
    }
    impl Drop for Fixture {fn drop(&mut self){let _=fs::remove_dir_all(&self.0);}}
    #[test] fn saves_private_yaml_and_preserves_unknown_fields(){
        let f=Fixture::new();let p=f.file();fs::write(&p,"dark_mode: true\nfuture_option:\n  enabled: true\n").unwrap();
        let mut store=PreferenceFile::open(p.clone()).unwrap();store.save("dark_mode: false\naccent_color: purple\n").unwrap();
        let parsed:serde_yaml::Value=serde_yaml::from_str(&fs::read_to_string(&p).unwrap()).unwrap();
        assert_eq!(parsed["future_option"]["enabled"].as_bool(),Some(true));assert_eq!(parsed["dark_mode"].as_bool(),Some(false));
        assert_eq!(fs::metadata(&p).unwrap().permissions().mode()&0o777,0o600);
        store.save("dark_mode: true\n").unwrap();assert_eq!(fs::read_dir(&f.0).unwrap().count(),1);
    }
    #[test] fn creates_once_and_detects_external_changes(){
        let f=Fixture::new();let p=f.file();let mut a=PreferenceFile::open(p.clone()).unwrap();let mut b=PreferenceFile::open(p.clone()).unwrap();
        a.save("theme: dark\n").unwrap();assert!(b.save("theme: light\n").is_err());
        fs::write(&p,"theme: external\n").unwrap();assert!(a.save("theme: ours\n").is_err());assert_eq!(fs::read_to_string(&p).unwrap(),"theme: external\n");
    }
    #[test] fn preserves_corrupt_yaml_and_rejects_non_mappings(){
        for content in ["broken: [", "- a\n- b\n", "null\n"] {let f=Fixture::new();let p=f.file();fs::write(&p,content).unwrap();let mut s=PreferenceFile::open(p.clone()).unwrap();assert!(s.save("theme: light\n").is_err());assert_eq!(fs::read_to_string(p).unwrap(),content);}
    }
    #[test] fn rejects_links_and_read_only_but_allows_retry_after_repair(){
        let f=Fixture::new();let p=f.file();let target=f.0.join("target");fs::write(&target,"theme: dark\n").unwrap();symlink(&target,&p).unwrap();assert!(PreferenceFile::open(p.clone()).is_err());fs::remove_file(&p).unwrap();
        fs::hard_link(&target,&p).unwrap();let mut s=PreferenceFile::open(p.clone()).unwrap();assert!(s.save("theme: light\n").is_err());fs::remove_file(&target).unwrap();
        fs::set_permissions(&p,fs::Permissions::from_mode(0o400)).unwrap();assert!(s.save("theme: light\n").is_err());fs::set_permissions(&p,fs::Permissions::from_mode(0o600)).unwrap();s.save("theme: light\n").unwrap();
    }
    #[test] fn bounds_reads_writes_and_rejects_special_files(){
        let f=Fixture::new();let p=f.file();fs::write(&p,vec![b'x';256*1024+1]).unwrap();assert!(PreferenceFile::open(p.clone()).is_err());fs::write(&p,[0xff]).unwrap();assert!(PreferenceFile::open(p.clone()).is_err());fs::remove_file(&p).unwrap();
        let mut s=PreferenceFile::open(p.clone()).unwrap();assert!(s.save(&"x".repeat(256*1024+1)).is_err());assert!(!p.exists());
        let name=std::ffi::CString::new(p.to_str().unwrap()).unwrap();assert_eq!(unsafe{libc::mkfifo(name.as_ptr(),0o600)},0);assert!(PreferenceFile::open(p).is_err());
    }
    // Files planted at names a temporary file could take — the old `.preferences-<pid>-<n>.tmp`
    // and private_file's `.<name>-<pid>-<n>.tmp` — are somebody else's. A save creates its own
    // exclusively and removes only the one it created, so every planted file survives it.
    #[test] fn never_removes_a_preexisting_temporary_file(){
        let f=Fixture::new();
        let pid=std::process::id();
        let fixtures:Vec<_>=(0..256).flat_map(|i|[f.0.join(format!(".preferences-{pid}-{i}.tmp")),f.0.join(format!(".settings.yaml-{pid}-{i}.tmp"))]).collect();
        for p in &fixtures{fs::write(p,"owned by somebody else").unwrap();}
        let mut s=PreferenceFile::open(f.file()).unwrap();
        let saved=s.save("theme: dark\n");
        for p in &fixtures{assert_eq!(fs::read_to_string(p).unwrap(),"owned by somebody else","{}",p.display());}
        // Either it found a free name and saved, or every name it tried was taken and it said so.
        match saved{Ok(())=>assert!(fs::read_to_string(f.file()).unwrap().contains("theme: dark")),Err(_)=>assert!(!f.file().exists())}
    }
    #[test] fn searches_control_keywords_and_handles_empty_or_unknown_queries(){
        assert_eq!(search_categories(" wallpaper "),vec![0]);assert_eq!(search_categories("Wi-Fi"),vec![3]);assert_eq!(search_categories("idle lock"),vec![5]);assert_eq!(search_categories(" API model "),vec![1]);assert_eq!(search_categories(""),(0..9).collect::<Vec<_>>());assert!(search_categories("no-such-control").is_empty());
    }
}
