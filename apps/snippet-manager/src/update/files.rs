//! Files in and out: a file kept as a snippet (or an export read back), from the Mind's `import`
//! or a file on the command line, and every snippet exported to one file. The file is read or
//! written in a job; what it held comes back here.

use std::path::{Path, PathBuf};

use yantrik_app::Refused;
use yantrik_foundation::place;
use yantrik_ipc_contracts::home_paths;

use super::changes::commit;
use super::{fail, say, written, Fx};
use crate::answers;
use crate::command::{After, Command, From, Read, Work};
use crate::model::{Model, PART_DETAIL, PART_LIST, PART_RAISE};
use crate::store::{file, text};

pub(super) fn update(m: &mut Model, command: Command, fx: &mut Fx) -> Result<(), Refused> {
    match command {
        Command::Import { path, from } => read(m, fx, path, from),
        // The command line names one file, as it always did; a second launch hands its file over
        // the same way.
        Command::Launch(files) => {
            if let Some(path) = files.into_iter().next() {
                read(m, fx, path, From::Launch);
            }
        }
        Command::Read(read) => landed(m, *read, fx),
        Command::ExportAll => export(m, fx),
        Command::Exported(Ok((path, count))) => {
            say(m, fx, &format!("Exported {count} snippet(s) to {}", path.display()));
        }
        Command::Exported(Err(why)) => fail(m, fx, &why),
        _ => {}
    }
    Ok(())
}

/// Read `path` in a job, to take it in when it lands.
fn read(m: &mut Model, fx: &mut Fx, path: PathBuf, from: From) {
    m.work += 1;
    let collection = m.collection;
    let agent = super::browsing::agent(fx);
    fx.spawn_or(Command::WorkFailed(Box::new(Work::Import)), move |_| {
        // An agent names only a file in the person's home, outside its protected places (#443, as
        // Images' `open`), asked before the path is looked at all; the person names any file.
        if agent {
            if let Err(why) = home_paths::may_read_file(&path.to_string_lossy(), &place::home()) {
                let result = Err(format!("an agent may import only a file in the person's home, outside its protected places: {why}"));
                return Command::Read(Box::new(Read { path, from, result, missing: true, collection }));
            }
        }
        // The Mind is told plainly there is no such file, before anything is decided; a file on
        // the command line that is not there is said on the notice line like any other failure.
        let missing = from == From::Surface && !path.is_file();
        let result = if missing {
            Err(format!("no file at {}", path.display()))
        } else {
            file::read_import(&path)
        };
        Command::Read(Box::new(Read { path, from, result, missing, collection }))
    });
}

fn landed(m: &mut Model, read: Read, fx: &mut Fx) {
    let incoming = match read.result {
        Ok(incoming) => incoming,
        Err(why) if read.missing => {
            fx.settle(Err(Refused::new("no_file", why)));
            return;
        }
        Err(why) => {
            fail(m, fx, &why);
            return;
        }
    };
    let restore = m.restore_point();
    let committed = match commit(m) {
        Ok(committed) => committed,
        Err(why) => {
            fail(m, fx, &why);
            return;
        }
    };
    match m.library.import(incoming, read.collection) {
        Ok(imported) => {
            // What came in is what the window shows, so the file named is visibly kept.
            if let Some(id) = imported.ids.first() {
                m.select(*id);
            }
            if read.from == From::Surface {
                fx.changed(PART_RAISE);
            }
            let answer = answers::imported(m, &imported.ids, imported.kind);
            written::write(m, fx, After::Answer(answer), restore);
        }
        Err(why) => {
            fail(m, fx, &why);
            if committed {
                written::write(m, fx, After::Say(why), restore);
            }
        }
    }
    fx.changed(PART_LIST);
    fx.changed(PART_DETAIL);
}

/// Every snippet to one file, in the app's own directory, named by the moment: this screen has
/// no file dialog, so the only honest alternative would be no export at all. The notice names
/// the full path.
fn export(m: &mut Model, fx: &mut Fx) {
    let bytes = match file::encode_bundle(&m.library) {
        Ok(bytes) => bytes,
        Err(why) => {
            fail(m, fx, &why);
            return;
        }
    };
    let dir = m.lane.path().parent().map(Path::to_path_buf).unwrap_or_default();
    let path = dir.join(format!("export-{}.json", text::now()));
    let count = m.library.snippets().len();
    let disk = m.lane.disk();
    m.work += 1;
    fx.spawn_or(Command::WorkFailed(Box::new(Work::Export)), move |_| {
        Command::Exported(disk.write(&path, &bytes).map(|()| (path, count)))
    });
}
