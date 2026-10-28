//! A ComfyUI server and an OpenAI-compatible one, both in this process, answering real HTTP on a
//! real socket to the same `ureq` client the app ships. What that tests is the protocol this app
//! speaks, rather than this app's idea of the protocol: the graph that goes out, the picture that
//! comes back and is filed, what the server says when it refuses, and that a key is used and never
//! repeated by anything the app says or writes.

use std::net::TcpListener;
use std::path::PathBuf;

use serde_json::{json, Value};

use super::server::{text_of_link, today, Manners, Server};
use super::started;
use crate::{backend, config, gallery, plan, words};

#[test]
fn a_comfyui_server_is_handed_a_graph_and_the_picture_it_renders_is_saved_with_its_record() {
    let server = Server::start(Manners::default());
    let f = started("wire-1", &format!(
            r#"{{"backend":{{"kind":"comfyui","base_url":"{}","model":"sdxl_test.safetensors"}}}}"#,
            server.base_url
        ));

    f.act("generate",
        json!({
            "prompt": "a lighthouse in fog",
            "negative": "blurry, text",
            "seed": 4242,
            // 1025 and 700: the ask is clamped onto the range but not onto the grid, so what proves
            // the graph was filled in is the latent coming out at 1024 and 704.
            "width": 1025,
            "height": 700,
            "steps": 20,
        }),
    )
    .unwrap();
    assert_eq!(f.settled(), "", "the render was reported as failed");

    // ── the graph that went out ──
    let posted = server.with(|recorded| recorded.prompts.clone());
    assert_eq!(posted.len(), 1, "one picture, one POST /prompt");
    assert_eq!(posted[0]["client_id"], json!("yantrik-studio"));
    let graph = server.last_graph();
    let nodes = graph.as_object().unwrap();
    assert!(nodes.keys().all(|id| !id.starts_with('_')), "the explanation was sent as a node: {graph}");
    assert_eq!(nodes.len(), 7, "the shipped graph is seven nodes: {graph}");

    let sampler = &nodes["3"];
    assert_eq!(sampler["class_type"], json!("KSampler"));
    assert_eq!(sampler["inputs"]["seed"].as_u64(), Some(4242));
    assert_eq!(sampler["inputs"]["steps"].as_u64(), Some(20));
    assert_eq!(sampler["inputs"]["cfg"].as_f64(), Some(plan::DEFAULT_CFG));
    assert_eq!(sampler["inputs"]["denoise"].as_f64(), Some(1.0));
    let latent = &nodes["5"];
    assert_eq!(latent["inputs"]["width"].as_u64(), Some(1024), "1025 was not snapped onto the grid");
    assert_eq!(latent["inputs"]["height"].as_u64(), Some(704), "700 was not snapped onto the grid");
    assert_eq!(latent["inputs"]["batch_size"].as_u64(), Some(1));
    assert_eq!(nodes["4"]["inputs"]["ckpt_name"], json!("sdxl_test.safetensors"));
    assert_eq!(
        nodes["9"]["inputs"]["filename_prefix"],
        json!(format!("Studio/{}", today())),
        "the server's own copy is filed under the same date as Studio's"
    );
    // Following the sampler's links, which is the part that cannot be checked by looking at one
    // node: swapped here, and every picture this app makes is the negative prompt.
    assert_eq!(text_of_link(&graph, &sampler["inputs"]["positive"]), "a lighthouse in fog");
    assert_eq!(text_of_link(&graph, &sampler["inputs"]["negative"]), "blurry, text");

    // ── the fetch ──
    let viewed = server.with(|recorded| recorded.viewed.clone());
    assert_eq!(viewed.len(), 1, "{viewed:?}");
    assert!(viewed[0].starts_with("/view?filename="), "{viewed:?}");
    assert!(viewed[0].contains("type=output"), "{viewed:?}");
    assert!(
        viewed[0].contains(&format!("subfolder=Studio%2F{}", today())),
        "the subfolder has to be percent-encoded, or the `&` in a filename would change what the next request asks for: {viewed:?}"
    );
    assert_eq!(server.with(|recorded| recorded.interrupts), 0, "nothing was cancelled");

    // ── and what landed ──
    let state = f.state();
    assert_eq!(state["backend"]["kind"], json!("comfyui"));
    assert_eq!(state["backend"]["generate_is_graded"], json!("standard"), "a server on this machine is local");
    assert_eq!(state["backend"]["prompt_leaves_this_machine"], json!(false));
    assert_eq!(state["gallery"]["count"], json!(1));
    let shot = &state["gallery"]["newest"][0];
    assert_eq!(shot["backend"], json!("comfyui"));
    assert_eq!(shot["model"], json!("sdxl_test.safetensors"));
    assert_eq!(shot["seed"], json!(4242));
    assert_eq!(shot["size"], json!("1024x704"));

    let path = PathBuf::from(shot["path"].as_str().unwrap());
    let sidecar = gallery::Sidecar::read(&path).unwrap();
    assert_eq!(sidecar.prompt, "a lighthouse in fog");
    assert_eq!(sidecar.negative, "blurry, text");
    assert_eq!(sidecar.seed, 4242);
    assert_eq!(sidecar.backend, "comfyui");
    assert_eq!(sidecar.sent, "1024x704", "the record names the size the graph was told to draw");
    assert_eq!(sidecar.steps, Some(20));
    // The bytes are what the server drew from the graph it was handed, computed here from the ask
    // rather than from the reply. This is the assertion that the whole round trip carried the
    // prompt, the seed and the size, and not merely that a file appeared.
    assert_eq!(
        std::fs::read(&path).unwrap(),
        backend::draw("a lighthouse in fog", 4242, 1024, 704),
        "the saved file is not the picture the server rendered from the graph"
    );
}


#[test]
fn a_server_that_refuses_the_graph_passes_its_own_explanation_back() {
    let server = Server::start(Manners {
        refuse: Some((
            400,
            r#"{"error":{"message":"Value not in list: ckpt_name: 'sdxl_test.safetensors'","type":"invalid_prompt"}}"#.to_string(),
        )),
        never_finish: false,
        ..Manners::default()
    });
    let f = started("wire-2", &server.config("comfyui"));

    f.act("generate", json!({ "prompt": "a lighthouse in fog", "width": 64, "height": 64 })).unwrap();
    let notice = f.settled();
    assert!(notice.contains("comfyui could not make the picture"), "{notice}");
    assert!(notice.contains("refused the graph (HTTP 400)"), "{notice}");
    // The server's own sentence is the useful part, so it is passed through rather than summarised
    // away: "Value not in list: ckpt_name" tells a person which file to name.
    assert!(notice.contains("Value not in list: ckpt_name"), "{notice}");
    assert!(!notice.contains("invalid_prompt"), "the whole envelope is noise next to its sentence: {notice}");
    assert_eq!(f.app.model(|m| m.gallery.len()), 0, "a refused render left a file behind");
    // The refusal reaches a caller too, and not only the notice bar.
    let rendered = f.describe();
    assert!(rendered["summary"].as_str().unwrap().contains("refused the graph"), "{}", rendered["summary"]);
    assert_eq!(rendered["state"]["gallery"]["count"], json!(0));
}


#[test]
fn a_server_nobody_is_listening_on_is_reported_as_unreachable() {
    // A port nothing is bound to: the listener is opened and dropped, so the number was real and is
    // now free.
    let port = TcpListener::bind("127.0.0.1:0").unwrap().local_addr().unwrap().port();
    let f = started("wire-3", &format!(r#"{{"backend":{{"kind":"comfyui","base_url":"http://127.0.0.1:{port}"}}}}"#));
    f.act("generate", json!({ "prompt": "a lighthouse in fog", "width": 64, "height": 64 })).unwrap();
    let notice = f.settled();
    assert!(notice.contains("comfyui could not make the picture"), "{notice}");
    assert!(notice.contains(&format!("http://127.0.0.1:{port} could not be reached")), "{notice}");
    // The question a person asks next is in the answer, because "could not be reached" on its own
    // sends them to the logs.
    assert!(notice.contains("Is ComfyUI running there"), "{notice}");
    assert!(notice.contains("listening on something other than 127.0.0.1"), "{notice}");
    assert_eq!(f.app.model(|m| m.gallery.len()), 0);
}

/// Review of #856: the backend's address is said (the notice, the state's `where`, the summary
/// line, the approval sentence) only as `place::address_of` draws it. ureq's own error text
/// begins with the whole URL, so it is not passed on either.
#[test]
fn a_backend_address_is_never_said_with_its_sign_in_details() {
    const SECRETS: &[&str] = &["user", "pw", "p/w", "api-key", "u@", "u:p", "%40", "#f"];
    let clean = |input: &str, shown: &str| {
        for secret in SECRETS {
            assert!(!shown.contains(secret), "{input} says {secret:?}: {shown}");
        }
    };
    let port = TcpListener::bind("127.0.0.1:0").unwrap().local_addr().unwrap().port();
    let base = format!("http://user:pw@127.0.0.1:{port}/x?api-key=k#f");
    let f = started("wire-4", &format!(r#"{{"backend":{{"kind":"comfyui","base_url":"{base}"}}}}"#));
    f.act("generate", json!({ "prompt": "a lighthouse in fog", "width": 64, "height": 64 })).unwrap();
    let notice = f.settled();
    assert!(notice.contains(&format!("http://127.0.0.1:{port}/x could not be reached")), "{notice}");
    clean(&base, &notice);
    clean(&base, f.state()["backend"]["where"].as_str().unwrap());
    clean(&base, &f.summary());

    for (base_url, said) in [
        ("https://user:p/w@gpu.example/v1", "a ComfyUI server away from your network at an address not shown, as it may hold sign-in details"),
        ("https://user:pw@gpu.example/v1?api-key=x#f", "a ComfyUI server away from your network at https://gpu.example/v1"),
        ("http://u:p@[::1]:8188", "a ComfyUI server you reach at http://[::1]:8188"),
    ] {
        let config = config::Config::parse(&json!({ "backend": { "kind": "comfyui", "base_url": base_url } }).to_string()).unwrap();
        assert_eq!(config.facts(true).place, said, "{base_url}");
    }

    // The approval sentence names where the call goes, and nothing of a URL whose host cannot be
    // told from its sign-in details (`https://user:1/w@evil.example` parses to the host "user").
    let (set_backend, _) = crate::intents::actions().into_iter().find(|(a, _)| a.name == "set_backend").unwrap();
    let explains = |args: Value| set_backend.explainer.as_ref().unwrap().sentence(&args);
    assert_eq!(
        explains(json!({ "kind": "openai-images", "base_url": "https://user:pw@images.example/v1?api-key=x" })),
        "After this, prompts and the value of the environment variable you named go to images.example."
    );
    for odd in ["https://user:1/w@evil.example/v1", "https://user:p/w@evil.example/v1"] {
        assert_eq!(explains(json!({ "kind": "openai-images", "base_url": odd })), "", "{odd}");
        assert_eq!(explains(json!({ "kind": "comfyui", "base_url": odd })), "", "{odd}");
    }
}

/// A key that is not a key. Named for this test alone, and read by nothing else in the binary, so
/// moving it cannot change another test's answer — which is the bar the rest of this crate's tests
/// hold by being handed their `Places` instead.
const KEY_VARIABLE: &str = "STUDIO_TEST_HOSTED_KEY";
const KEY_VALUE: &str = "sk-studio-test-not-a-real-key-9f3c2b";

#[test]
fn a_key_is_used_and_never_repeated_by_anything_the_app_says_or_writes() {
    std::env::set_var(KEY_VARIABLE, KEY_VALUE);
    let server = Server::start(Manners::default());
    let f = started("wire-5", &format!(
            r#"{{"backend":{{"kind":"openai-images","base_url":"{}","model":"gpt-image-1","api_key_env":"{KEY_VARIABLE}"}}}}"#,
            server.base_url
        ));
    assert_eq!(f.app.model(|m| m.grade()), "sensitive", "a hosted service is `sensitive` however trustworthy it is");

    // A hosted service accepts three sizes; 2000×700 is not one of them.
    f.act("generate", json!({ "prompt": "a wide harbour", "width": 2000, "height": 700, "seed": 5 }))
        .unwrap();
    assert_eq!(f.settled(), "");

    // The key was read from the environment at the moment of the call — that is the only place it
    // comes from — and it reached the service.
    let sent = server.with(|recorded| recorded.authorizations.clone());
    assert_eq!(sent, vec![format!("Bearer {KEY_VALUE}")]);
    let asked = server.with(|recorded| recorded.image_requests.clone());
    assert_eq!(asked.len(), 1);
    assert_eq!(asked[0]["model"], json!("gpt-image-1"));
    assert_eq!(asked[0]["prompt"], json!("a wide harbour"));
    assert_eq!(asked[0]["size"], json!("1536x1024"), "the nearest shape was not chosen");
    assert!(asked[0].get("response_format").is_none(), "gpt-image-1 rejects that field");

    // And it is nowhere else.
    let state = f.state();
    assert_eq!(state["backend"]["api_key_env"], json!(KEY_VARIABLE), "the variable's name is the person's to know");
    assert_eq!(state["backend"]["api_key_is_set"], json!(true));
    assert_eq!(state["backend"]["generate_is_graded"], json!("sensitive"));
    assert_eq!(state["backend"]["prompt_leaves_this_machine"], json!(true));
    let rendered = f.describe().to_string();
    assert!(!rendered.contains(KEY_VALUE), "describe repeated the key");
    assert!(rendered.contains(KEY_VARIABLE));

    let (headline, facts, status) =
        f.app.model(|m| (words::headline(m), words::facts_block(m), words::status_line(m)));
    for text in [headline, facts.clone(), status, f.summary(), f.state().to_string()] {
        assert!(!text.contains(KEY_VALUE), "the window repeated the key: {text}");
    }
    // What the window does say is which variable to export, and whether it worked.
    assert!(facts.contains(KEY_VARIABLE), "{facts}");
    assert!(facts.contains("set, so prompts can be sent"), "{facts}");
    assert!(facts.contains("graded `sensitive`"), "{facts}");
    assert!(facts.contains("sent off this machine"), "{facts}");

    let shot = &state["gallery"]["newest"][0];
    let path = PathBuf::from(shot["path"].as_str().unwrap());
    assert_eq!(shot["backend"], json!("openai-images"));
    assert_eq!(shot["model"], json!("gpt-image-1"));
    assert_eq!(shot["size"], json!("1536x1024"));
    let sidecar_text = std::fs::read_to_string(gallery::Sidecar::path_for(&path)).unwrap();
    assert!(!sidecar_text.contains(KEY_VALUE), "the record beside the picture holds the key");
    let sidecar = gallery::Sidecar::read(&path).unwrap();
    assert_eq!(sidecar.prompt, "a wide harbour");
    assert_eq!(sidecar.seed, 5);
    assert_eq!(sidecar.sent, "1536x1024");
    assert_eq!(std::fs::read(&path).unwrap(), backend::draw("a wide harbour", 1, 1536, 1024));

    // A backend that cannot send anything says so without saying why in terms of the key.
    std::env::remove_var(KEY_VARIABLE);
    let answer = f.act("set_backend",
        json!({ "kind": "openai-images", "model": "gpt-image-1", "api_key_env": KEY_VARIABLE }),
    )
    .unwrap();
    assert!(answer["note"].as_str().unwrap().contains("is not set"), "{answer}");
    assert!(!answer.to_string().contains(KEY_VALUE));

    // The settings file — written by that `set_backend`, since opening the app from text never
    // touches the disk — holds the variable's name and not its contents, which is the whole
    // reason `api_key_env` is a name.
    let written = std::fs::read_to_string(f.world.places.settings.as_ref().unwrap()).unwrap();
    assert!(!written.contains(KEY_VALUE), "the configuration file holds the key");
    assert!(written.contains(KEY_VARIABLE));
    assert!(written.contains("openai-images"));
}