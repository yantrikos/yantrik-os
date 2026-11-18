use super::*;
use chrono::TimeZone;

fn folder() -> PathBuf {
    let dir = std::env::temp_dir().join(format!(
        "studio-gallery-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

fn sidecar(prompt: &str) -> Sidecar {
    Sidecar {
        prompt: prompt.into(),
        negative: "blurry".into(),
        seed: 1234,
        backend: "fake".into(),
        model: "fake".into(),
        seconds: 0.4,
        width: 64,
        height: 48,
        sent: "64x48".into(),
        created: "2026-09-22T10:00:00+05:30".into(),
        made_from: String::new(),
        steps: Some(30),
        cfg: Some(7.0),
        made_by: default_made_by(),
    }
}

#[test]
fn the_pictures_go_in_a_dated_folder_under_pictures() {
    // The base is read from this machine rather than pinned by setting HOME here. The
    // environment is process-wide and the tests in one binary run at the same time, so a test
    // that moved HOME to check something would be changing what every other test saw —
    // including the ones asserting that a path is written with `~/…` in it.
    let base = match std::env::var_os("XDG_PICTURES_DIR") {
        Some(value) if !value.is_empty() => PathBuf::from(value),
        _ => home().join("Pictures"),
    };
    assert_eq!(output_root(""), base.join("Studio"));
    // A configured folder is used exactly as given, so a person can point Studio at a disk.
    assert_eq!(output_root("/srv/pictures"), PathBuf::from("/srv/pictures"));
    assert_eq!(output_root("~/art"), home().join("art"));
    let when = Local.with_ymd_and_hms(2026, 9, 22, 10, 0, 0).unwrap();
    assert_eq!(day_folder(&base.join("Studio"), when), base.join("Studio/2026-09-22"));
}

#[test]
fn a_sidecar_written_beside_a_picture_reads_back_as_the_same_thing() {
    let dir = folder();
    let image = dir.join("2026-09-22").join("101500-01234.png");
    std::fs::create_dir_all(image.parent().unwrap()).unwrap();
    std::fs::write(&image, crate::backend::draw("a lighthouse", 1234, 64, 48)).unwrap();

    let written = sidecar("a lighthouse in fog");
    let path = written.write(&image).unwrap();
    assert_eq!(path, dir.join("2026-09-22").join("101500-01234.json"));
    assert!(!path.with_extension("json.new").exists(), "the temporary file was left behind");

    let read = Sidecar::read(&image).unwrap();
    assert_eq!(read.prompt, "a lighthouse in fog");
    assert_eq!(read.negative, "blurry");
    assert_eq!(read.seed, 1234);
    assert_eq!(read.backend, "fake");
    assert_eq!(read.seconds, 0.4);
    assert_eq!(read.sent, "64x48");
    assert_eq!(read.steps, Some(30));
    assert_eq!(read.made_by, "yantrik-studio");
    std::fs::remove_dir_all(&dir).ok();
}

#[test]
fn a_seed_no_signed_integer_holds_survives_the_sidecar() {
    // The whole promise of a seed is that the picture can be made again from it.
    let dir = folder();
    let image = dir.join("big.png");
    std::fs::write(&image, crate::backend::draw("x", 1, 8, 8)).unwrap();
    let mut written = sidecar("x");
    written.seed = u64::MAX - 1;
    written.write(&image).unwrap();
    let text = std::fs::read_to_string(Sidecar::path_for(&image)).unwrap();
    assert!(text.contains("18446744073709551614"), "{text}");
    assert_eq!(Sidecar::read(&image).unwrap().seed, u64::MAX - 1);
    std::fs::remove_dir_all(&dir).ok();
}

#[test]
fn a_sidecar_from_an_older_version_still_reads() {
    // Fields added later must not make pictures already on disk unreadable, and fields a
    // hand-written sidecar omits must not make it a parse failure.
    let dir = folder();
    let image = dir.join("old.png");
    std::fs::write(&image, crate::backend::draw("x", 1, 8, 8)).unwrap();
    std::fs::write(
        Sidecar::path_for(&image),
        r#"{"prompt":"an older picture","seed":7,"backend":"comfyui","seconds":41.2,
            "width":1024,"height":1024,"created":"2026-01-01T09:00:00+05:30"}"#
            .replace('\n', ""),
    )
    .unwrap();
    let read = Sidecar::read(&image).unwrap();
    assert_eq!(read.prompt, "an older picture");
    assert_eq!(read.negative, "");
    assert_eq!(read.steps, None);
    assert_eq!(read.made_by, "yantrik-studio");
    std::fs::remove_dir_all(&dir).ok();
}

#[test]
fn a_picture_with_no_sidecar_is_still_listed_and_says_it_has_no_record() {
    let dir = folder();
    let day = dir.join("2026-09-22");
    std::fs::create_dir_all(&day).unwrap();
    std::fs::write(day.join("101500-01234.png"), crate::backend::draw("a lighthouse", 1234, 64, 48)).unwrap();
    std::fs::write(day.join("copied-in.png"), crate::backend::draw("elsewhere", 9, 32, 32)).unwrap();
    sidecar("a lighthouse in fog").write(&day.join("101500-01234.png")).unwrap();

    let rows = listing(&dir, LISTING_CAP, false);
    assert_eq!(rows.len(), 2, "{rows:?}");
    let with = rows.iter().find(|row| row.has_sidecar).unwrap();
    assert_eq!(with.prompt, "a lighthouse in fog");
    let without = rows.iter().find(|row| !row.has_sidecar).unwrap();
    assert_eq!(without.name, "copied-in.png");
    // The size still comes from the file, so the row is not empty; the seed and backend are
    // not invented for it.
    assert_eq!((without.width, without.height), (32, 32));
    assert_eq!(without.seed, 0);
    assert_eq!(without.json()["sidecar"], serde_json::json!("missing"));
    assert!(without.json().get("seed").is_none());
    assert!(with.json().get("sidecar").is_none());
    std::fs::remove_dir_all(&dir).ok();
}

#[test]
fn the_gallery_is_newest_first_and_stops_at_the_limit() {
    let dir = folder();
    for (day, hour) in [("2026-09-20", 9), ("2026-09-22", 10), ("2026-09-21", 11)] {
        let folder = dir.join(day);
        std::fs::create_dir_all(&folder).unwrap();
        let image = folder.join(format!("{hour}0000-00001.png"));
        std::fs::write(&image, crate::backend::draw(day, 1, 16, 16)).unwrap();
        let mut record = sidecar(day);
        record.created = format!("{day}T{hour:02}:00:00+05:30");
        record.write(&image).unwrap();
    }
    let rows = listing(&dir, LISTING_CAP, false);
    let order: Vec<&str> = rows.iter().map(|row| row.prompt.as_str()).collect();
    assert_eq!(order, ["2026-09-22", "2026-09-21", "2026-09-20"], "{order:?}");

    let two = listing(&dir, 2, false);
    assert_eq!(two.len(), 2);
    assert_eq!(two[0].prompt, "2026-09-22");

    assert!(listing(&dir.join("nothing-here"), LISTING_CAP, false).is_empty());
    std::fs::remove_dir_all(&dir).ok();
}

#[test]
fn only_pictures_are_listed_not_the_sidecars_beside_them() {
    let dir = folder();
    let day = dir.join("2026-09-22");
    std::fs::create_dir_all(&day).unwrap();
    std::fs::write(day.join("a.png"), crate::backend::draw("a", 1, 16, 16)).unwrap();
    sidecar("a").write(&day.join("a.png")).unwrap();
    std::fs::write(day.join("notes.txt"), "not a picture").unwrap();
    std::fs::write(day.join("a.json.new"), "{}").unwrap();
    let rows = listing(&dir, LISTING_CAP, false);
    assert_eq!(rows.len(), 1, "{rows:?}");
    assert_eq!(rows[0].name, "a.png");
    std::fs::remove_dir_all(&dir).ok();
}

#[test]
fn a_row_carries_what_a_person_would_ask_about_the_picture() {
    let dir = folder();
    let day = dir.join("2026-09-22");
    std::fs::create_dir_all(&day).unwrap();
    let image = day.join("101500-01234.png");
    std::fs::write(&image, crate::backend::draw("a lighthouse", 1234, 64, 48)).unwrap();
    let mut written = sidecar("a lighthouse in fog");
    written.backend = "comfyui".into();
    written.model = "dreamshaperXL_v21.safetensors".into();
    written.seconds = 41.23;
    written.width = 1024;
    written.height = 768;
    written.made_from = "variation of 091200-00007.png".into();
    written.write(&image).unwrap();

    let row = listing(&dir, 1, false).remove(0);
    let value = row.json();
    assert_eq!(value["prompt"], serde_json::json!("a lighthouse in fog"));
    assert_eq!(value["seed"], serde_json::json!(1234));
    assert_eq!(value["backend"], serde_json::json!("comfyui"));
    assert_eq!(value["model"], serde_json::json!("dreamshaperXL_v21.safetensors"));
    // Rounded to a tenth: the seconds are worth knowing and the microseconds are not.
    assert_eq!(value["seconds"], serde_json::json!(41.2));
    assert_eq!(value["size"], serde_json::json!("1024x768"));
    assert_eq!(value["made_from"], serde_json::json!("variation of 091200-00007.png"));
    assert!(value["path"].as_str().unwrap().ends_with("101500-01234.png"));
    std::fs::remove_dir_all(&dir).ok();
}

#[test]
fn a_thumbnail_is_decoded_and_shrunk_to_something_a_grid_can_show() {
    let dir = folder();
    let image = dir.join("big.png");
    std::fs::write(&image, crate::backend::draw("a lighthouse", 1, 640, 480)).unwrap();
    let thumbnail = read_thumbnail(&image).unwrap();
    assert!(thumbnail.width <= THUMBNAIL_EDGE && thumbnail.height <= THUMBNAIL_EDGE);
    assert_eq!(thumbnail.rgb.len(), (thumbnail.width * thumbnail.height * 3) as usize);
    assert_eq!((thumbnail.width, thumbnail.height), (160, 120));
    assert!(read_thumbnail(&dir.join("not-there.png")).is_none());
    std::fs::remove_dir_all(&dir).ok();
}

#[test]
fn resampling_doubles_a_picture_and_says_so_when_there_is_no_room_to() {
    let dir = folder();
    let image = dir.join("small.png");
    std::fs::write(&image, crate::backend::draw("a lighthouse", 1, 64, 48)).unwrap();
    let bytes = resample(&image, 2, LONGEST_EDGE).unwrap();
    assert_eq!(png_size(&bytes), (Some(128), Some(96)));

    // A ceiling the picture has already reached is reported rather than written back as a
    // copy of itself, which would look like an upscale that did nothing.
    let problem = resample(&image, 2, 64).unwrap_err();
    assert!(problem.contains("64x48"), "{problem}");
    assert!(problem.contains("as large as"), "{problem}");

    // A factor of one is the same picture, and saying so is better than writing a duplicate.
    assert!(resample(&image, 1, LONGEST_EDGE).is_err());
    assert!(resample(&dir.join("nothing.png"), 2, LONGEST_EDGE)
        .unwrap_err()
        .contains("could not be opened"));
    std::fs::remove_dir_all(&dir).ok();
}

#[test]
fn a_filename_that_already_exists_is_not_reused() {
    let dir = folder();
    std::fs::create_dir_all(&dir).unwrap();
    let when = Local.with_ymd_and_hms(2026, 9, 22, 10, 15, 0).unwrap();
    let first = unique_file(&dir, when, 1234);
    assert_eq!(first.file_name().unwrap(), "101500-01234.png");
    std::fs::write(&first, b"x").unwrap();
    let second = unique_file(&dir, when, 1234);
    assert_eq!(second.file_name().unwrap(), "101500-01234-2.png");
    std::fs::write(&second, b"x").unwrap();
    assert_eq!(unique_file(&dir, when, 1234).file_name().unwrap(), "101500-01234-3.png");
    // A sidecar left behind by a failed write is also enough to move the name on, so a
    // picture cannot end up wearing a record that belongs to another one.
    let third = unique_file(&dir, when, 99999);
    assert_eq!(third.file_name().unwrap(), "101500-99999.png");
    std::fs::write(Sidecar::path_for(&third), b"{}").unwrap();
    assert_eq!(unique_file(&dir, when, 99999).file_name().unwrap(), "101500-99999-2.png");
    std::fs::remove_dir_all(&dir).ok();
}

#[test]
fn only_studios_own_gallery_is_inside_studios_own_gallery() {
    let dir = folder();
    let day = dir.join("2026-09-22");
    std::fs::create_dir_all(&day).unwrap();
    let image = day.join("a.png");
    std::fs::write(&image, b"x").unwrap();
    assert!(is_inside(&image, &dir));
    assert!(is_inside(&day, &dir));
    assert!(!is_inside(&dir, &day), "the folder itself is not inside a day of it");
    assert!(!is_inside(Path::new("/etc/passwd"), &dir));
    assert!(!is_inside(&dir.join("nope.png"), &dir));
    // A symlink pointing out of the gallery is still outside it, which is what canonicalize
    // is doing here.
    #[cfg(unix)]
    {
        let link = dir.join("link");
        std::os::unix::fs::symlink("/etc", &link).unwrap();
        assert!(!is_inside(&link.join("passwd"), &dir));
    }
    std::fs::remove_dir_all(&dir).ok();
}
