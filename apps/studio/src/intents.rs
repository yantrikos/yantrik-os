//! What the Mind may ask Studio by name: the same nine actions as before, with the same names,
//! descriptions, parameters, grades and order.
//!
//! # Grades
//!
//! `generate` and `variations` are declared at `standard` and follow the backend: they are brought
//! to `sensitive` the moment the configuration names a service that is not on a network the person
//! owns (`update/backend.rs`, which regrades them at start and on every `set_backend`), because the
//! sentence is sent to a machine the person does not own and the service may charge for it.
//! `set_backend` is `sensitive` because it decides where every later prompt goes. `refresh` is
//! `safe`; everything else is `standard`.
//!
//! # Settling
//!
//! `generate`, `variations` and `upscale` declare that they only start the work, as they always
//! did: the answer carries the job's id and `describe` lists the job under `queue` until the
//! pictures land. Every action is answered when its own command has landed, in the command that
//! carries its result (`answers.rs`).

use std::time::Duration;

use yantrik_app::control::{Action, Param};
use yantrik_app::intents::{Agents, Args, Intent, Intents, Settle};

use crate::answers;
use crate::args;
use crate::command::{Command, Which};
use crate::config;
use crate::Studio;

/// How long an action may take to land before the Mind is told it did not. Every one of them is a
/// file read, a file move or a queue entry; none waits for a picture.
pub const WITHIN: Duration = Duration::from_secs(10);

/// What every agent caller of `set_backend` is told. Where Studio sends prompts, and which
/// credential goes with them, is the person's choice: the variable `api_key_env` names is sent as
/// a bearer token to the `base_url` named, so an agent that could set both could have any secret
/// in the environment sent to a host it picked. The framework refuses an agent's call to an
/// intent declared `Agents::Refused` before the grade, the mode or an approval is looked at, so no
/// card is ever shown for it.
pub const AGENT_REFUSED: &str =
    "Changing where Studio sends prompts is the person's choice; ask them to edit ~/.config/yantrik/studio.json.";

pub fn table(table: &mut Intents<Studio>) {
    for (action, parse) in actions() {
        let persons_alone = action.name == "set_backend";
        let mut intent = Intent::new(action, move |a| Ok(parse(a)))
            .settles(Settle::Explicit { within: WITHIN })
            .reply(answers::reply);
        if persons_alone {
            intent = intent.agents(Agents::Refused(AGENT_REFUSED));
        }
        table.add(intent);
    }
}

type Parse = fn(&Args<'_>) -> Command;

/// The declarations, kept apart from the commands so a test can read exactly what a mind is shown.
pub fn actions() -> Vec<(Action, Parse)> {
    vec![
        (
            act(
                "generate",
                "Make pictures from a sentence and save them as files in the output folder, each with a \
                 small JSON record beside it naming the prompt, the seed, the model, the backend and \
                 how many seconds it took. This starts the work rather than finishing it: the answer \
                 carries the job's id, and `describe` lists that job under `queue.running` until the \
                 pictures land in `gallery.newest`. Graded `sensitive` while a hosted backend is \
                 configured, because the sentence is sent to a machine you do not own and the service \
                 may charge for each picture; graded `standard` on a ComfyUI server on your own network \
                 and on `fake`, the placeholder backend, where the prompt never leaves the building.",
            )
            .defers()
            .arg(arg(
                "prompt",
                "The sentence the picture is made from. Up to 4000 characters, and the only required \
                 argument.",
            ))
            .arg(
                arg("negative", "What to keep out of the picture. Left out, Studio sends its own \
                 default negative prompt; sent as an empty string, no negative prompt is sent at all.")
                    .optional(),
            )
            .arg(num("width", "Pixels wide. Rounded onto the 8-pixel grid the models want and kept \
                 between 64 and 4096. Left out means 1024. A hosted service may only accept a few \
                 sizes, in which case the nearest one is used and the record beside the picture names \
                 the size that was actually sent.").optional())
            .arg(num("height", "Pixels high. Same rounding and same limits as `width`. Left out means \
                 1024.").optional())
            .arg(num("steps", "Sampling steps: more is slower and usually sharper. Between 1 and 150, \
                 left out means 30. A hosted service may ignore this entirely.").optional())
            .arg(num("seed", "The seed to start from. Left out, one is chosen at random and written \
                 into the record beside the picture, so any picture can be asked for again. Asking for \
                 several pictures with one seed runs the seeds on: 7 makes 7, 8 and 9.").optional())
            .arg(num("count", "How many pictures to make, 1 to 4. A hosted backend charges for each \
                 one, which is why this stops at four.").optional()),
            |a| Command::Generate(Some(Box::new(args::plan_of(a.raw())))),
        ),

        (
            act(
                "variations",
                "Make more pictures of one that already exists, taken from the record beside it: the \
                 same sentence, the same size, the same step count, new seeds. `of` is the picture's \
                 path, or just its filename if it is in the gallery. Starts the work and answers with \
                 the job's id. Graded the same way as `generate`, because it sends the same prompt to \
                 the same backend.",
            )
            .defers()
            .arg(arg(
                "of",
                "The picture to vary: its path, or its bare filename if it is in Studio's gallery.",
            ))
            .arg(num("count", "How many variations, 1 to 4. Left out means 1.").optional()),
            |a| Command::Variations { of: args::given(a.raw(), "of"), count: args::count_or(a.raw(), 1), fill: false },
        ),

        (
            act(
                "upscale",
                "Make a bigger copy of a picture by resampling it on this machine, with no model and no \
                 network. It is a resize and not an upscaler: it interpolates the pixels that are \
                 already there and cannot add detail that was never generated. The copy is saved into \
                 the gallery with a record naming the picture it came from. Nothing is sent anywhere, \
                 whatever backend is configured, and no backend is asked.",
            )
            .defers()
            .arg(arg("path", "The picture to enlarge: a path, or a bare filename from the gallery."))
            .arg(
                num("factor", "How many times bigger on each edge, 2 to 8. Left out means 2, so a \
                 1024×1024 picture becomes 2048×2048.")
                    .optional(),
            ),
            |a| Command::Upscale { of: args::given(a.raw(), "path"), factor: args::factor_of(a.raw()) },
        ),

        (
            act(
                "open",
                "Open a picture — or a folder, if a folder is named — in whatever this machine opens it \
                 with. The file is not changed and nothing leaves the machine.",
            )
            .arg(arg("path", "What to open: a picture's path, a folder's path, or a bare filename from \
                 the gallery.")),
            |a| Command::Open { of: args::given(a.raw(), "path") },
        ),

        (
            act(
                "delete",
                "Move a picture, and the record beside it, to the Trash. It is recoverable: the files \
                 are moved into the OS's own trash folder, not destroyed, and Files can put them back. \
                 Only files inside Studio's output folder can be deleted this way — this is not a way \
                 to remove an arbitrary file.",
            )
            .arg(arg("path", "The picture to move to the Trash: a path, or a bare filename from the \
                 gallery.")),
            |a| Command::Delete { of: args::given(a.raw(), "path") },
        ),

        (
            act(
                "set_backend",
                "The person's alone: an agent that calls this is refused and should ask the person to \
                 edit ~/.config/yantrik/studio.json. Choose where pictures are made from now on, and \
                 write that choice down in the configuration file. Graded `sensitive` because it decides where every later prompt \
                 goes: naming a hosted service means the sentences typed into this app will leave this \
                 machine and may cost money. `generate` and `variations` are regraded the moment this \
                 lands, so a caller cannot point Studio at a service and generate in the same breath \
                 under the old, local grade. No key is taken here — only the NAME of an environment \
                 variable that holds one, which is read at call time and never stored, logged or \
                 shown.",
            )
            .risk("sensitive")
            .arg(arg(
                "kind",
                "Which backend: `comfyui` for a ComfyUI server (your own GPU), `openai-images` for any \
                 OpenAI-compatible /v1/images/generations endpoint (a hosted service), or `fake` for \
                 the placeholder that draws from the prompt's hash on this machine.",
            ))
            .arg(
                arg("base_url", "Where the backend is. Left out, a kind takes its own default: \
                 http://127.0.0.1:8188 for ComfyUI, https://api.openai.com/v1 for a hosted service.")
                    .optional(),
            )
            .arg(arg("model", "Which model to ask for: a ComfyUI checkpoint filename, or a hosted \
                 service's image model. Required for `openai-images`, because a guess here spends \
                 money on the wrong thing.").optional())
            .arg(arg("api_key_env", "The NAME of the environment variable holding the API key for a \
                 hosted service — for example OPENAI_API_KEY. Only the name is written down; the value \
                 is read from the environment each time and never stored.").optional())
            .arg(arg("workflow", "A path to a ComfyUI workflow in API format, to use instead of the \
                 built-in SDXL text-to-image graph. A file saved from ComfyUI's editor rather than its \
                 'Save (API Format)' is refused with an explanation, not silently misread.").optional())
            // The paragraph above can only state the condition ("naming a hosted service means the
            // sentences typed into this app will leave this machine"); whether THIS call meets it
            // turns on the `kind` it carries, which `describe` never sees (#137). So the card gets
            // the unconditional half from here: what the arguments establish about where prompts go
            // after this call, and nothing guessed beyond that. A kind this closure does not know
            // gets an honest nothing rather than a guess — the card then reads as it did before.
            .explain(|args| {
                // Read the kind the way `set_backend` itself will (`Kind::parse`), not by its one
                // canonical spelling: "openai" and "OpenAI_Images" pick the same backend, and a
                // spelling this closure did not know used to leave the card unexplained — which is
                // also the card that still offers a standing yes (#137).
                let Some(kind) = config::Kind::parse(&args::given(args, "kind")) else {
                    return String::new();
                };
                // Where the call goes: the URL it names, else the one the backend falls back to.
                let named = args::given(args, "base_url");
                let url = match (named.trim(), kind) {
                    ("", config::Kind::OpenAiImages) => config::DEFAULT_OPENAI_URL,
                    ("", config::Kind::ComfyUi) => config::DEFAULT_COMFY_URL,
                    (url, _) => url,
                };
                // Name the PARSED HOST, never the raw argument: `base_url` is caller-supplied text,
                // and "https://api.openai.com@evil.example/v1" would read as naming api.openai.com
                // while every prompt and the key go to evil.example. A URL that parses to no host
                // gets an honest nothing rather than a guess. So does one whose sign-in details cannot
                // be told from its host (`https://user:1/w@evil.example` parses to the host "user"):
                // the one rule for drawing an address says nothing of it (`place::address_of`, #856).
                let drawable = !yantrik_app::place::address_of(url).is_empty();
                let host = || {
                    url::Url::parse(url).ok().filter(|_| drawable).and_then(|u| u.host_str().map(str::to_owned))
                };
                match kind {
                    config::Kind::Fake => "After this, prompts stay on this machine.".to_string(),
                    // A hosted service is sent a key with the prompt: the card says which variable's
                    // value goes, by name and never by value.
                    config::Kind::OpenAiImages => {
                        let named = args::given(args, "api_key_env");
                        let name = match named.trim() {
                            "" => "OPENAI_API_KEY",
                            name => name,
                        };
                        let usable = config::is_variable_name(name);
                        // The name only when it is a variable that is set here (`shown_variable`):
                        // a key pasted where a name goes is valid as a name and must not be said.
                        match (host(), usable, config::shown_variable(name)) {
                            (Some(host), true, Some(name)) => {
                                format!("After this, prompts and the value of ${name} go to {host}.")
                            }
                            (Some(host), true, None) => format!(
                                "After this, prompts and the value of the environment variable you named go to {host}."
                            ),
                            _ => String::new(),
                        }
                    }
                    config::Kind::ComfyUi => match host().as_deref() {
                        Some("127.0.0.1" | "localhost" | "[::1]") => {
                            "After this, prompts go to the ComfyUI server on this machine.".to_string()
                        }
                        Some(host) => format!("After this, prompts go to the ComfyUI server at {host}."),
                        None => String::new(),
                    },
                }
            }),
            |a| Command::SetBackend(Box::new(args::backend_ask(a.raw()))),
        ),

        (
            act(
                "cancel",
                "Stop one job. A picture already saved stays saved, and there is no half-written file \
                 to clean up: Studio writes nothing until a whole picture has arrived. To stop \
                 everything in the queue, ask for `cancel_all`.",
            )
            .arg(num("job", "The job id from `generate`, `variations` or `upscale`, as `describe` lists \
                 under `queue`: a whole number of 1 or more.")),
            |a| Command::Cancel(args::which(a.raw())),
        ),

        (
            act(
                "cancel_all",
                "Stop every job in the queue, which is what the window's Cancel button asks for. A \
                 picture already saved stays saved, as with `cancel`: Studio writes nothing until a \
                 whole picture has arrived.",
            ),
            |_| Command::Cancel(Which::All),
        ),

        (
            act(
                "refresh",
                "Read the output folder again. Pictures this app made are noticed on their own within a \
                 few seconds; this is for one copied in by hand, or for a folder that changed while \
                 Studio was not looking. Read-only: it changes nothing and sends nothing.",
            )
            .risk("safe"),
            |_| Command::Refresh,
        ),
    ]
}

/// One action, with the sentence a reader who cannot see the screen needs.
///
/// The guard is the point: a description that is only the action's own name stops the app before
/// it serves, and stops the tests, which build this same list. `Action::new` takes any `&str` and
/// cannot be made to care, and the framework is shared by every app, so the check lives here.
fn act(name: &'static str, sentence: &'static str) -> Action {
    assert!(
        sentence.len() >= 40 && !sentence.starts_with("Studio:"),
        "studio action `{name}` was given a placeholder description: {sentence:?}"
    );
    Action::new(name, sentence)
}

/// One text argument. Required is the default, which is the opposite of what a loop that marked
/// everything optional would do: a call naming nothing must not be accepted. An argument that may
/// be left out says `.optional()` at its call site, where a reader of the list can see it.
fn arg(name: &'static str, sentence: &'static str) -> Param {
    checked_argument(Param::text(name), name, sentence)
}

/// One number argument.
fn num(name: &'static str, sentence: &'static str) -> Param {
    checked_argument(Param::number(name), name, sentence)
}

fn checked_argument(param: Param, name: &'static str, sentence: &'static str) -> Param {
    assert!(sentence.len() >= 15, "studio argument `{name}` was given no usable description: {sentence:?}");
    param.describe(sentence)
}
