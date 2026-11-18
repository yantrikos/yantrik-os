//! Turning "this picture, this size, this seed" into a ComfyUI graph.
//!
//! The graph is filled by following the sampler's own links rather than by assuming node
//! numbers, so a workflow a person exported themselves works as well as the shipped one. What
//! could not be filled is collected and reported rather than dropped: a graph that silently
//! ignores the prompt is the failure that would otherwise look like a model that does not
//! listen.
//!
//! Pure — a string in, a value out — so the templating can be tested without a server.

use std::path::{Path, PathBuf};

use serde_json::{json, Map, Value};

use crate::backend::Request;

/// The graph that ships with the app. Read at compile time so an installed binary does not
/// depend on a file that may not have been installed beside it.
pub const BUILT_IN: &str = include_str!("../workflows/sdxl_txt2img.json");

/// The latent grid SDXL works on. A width of 1023 is not an error to ComfyUI, it is a picture
/// that comes back a different shape than was asked for, so the request is snapped first and the
/// sidecar records what was actually sent.
pub const GRID: u32 = 8;

pub fn snap_to_grid(value: u32) -> u32 {
    (((value + GRID / 2) / GRID) * GRID).max(GRID)
}

/// What came out of filling a graph in: the graph to send, the size it was told to draw, and an
/// honest account of what took and what did not.
#[derive(Clone, Debug, Default)]
pub struct Filled {
    pub graph: Value,
    /// The latent size the graph was given, if the graph has a latent to give one to.
    pub size: Option<(u32, u32)>,
    /// Short phrases naming each thing that was set. Kept for the log, not for `describe`.
    pub applied: Vec<String>,
    /// Whole sentences naming each thing that could not be set. The first of these is what a
    /// person sees, because "the picture is not what you asked for" is otherwise unexplainable.
    pub missed: Vec<String>,
}

/// The graph to fill: a person's own file if they named one and it can be read, otherwise the
/// shipped SDXL pass.
pub fn graph(override_path: &str) -> Result<Value, String> {
    let path = override_path.trim();
    if path.is_empty() {
        return api_format(BUILT_IN)
            .map_err(|e| format!("the graph Studio ships with is not usable: {e}"));
    }
    let expanded = expand_home(path);
    let text = std::fs::read_to_string(&expanded).map_err(|e| {
        format!(
            "the workflow {} named in the configuration could not be read ({e}); check the path in the studio.json config",
            expanded.display()
        )
    })?;
    api_format(&text).map_err(|e| format!("{} is not a usable ComfyUI graph: {e}", expanded.display()))
}

fn expand_home(path: &str) -> PathBuf {
    match path.strip_prefix("~/") {
        Some(rest) => match std::env::var_os("HOME") {
            Some(home) => Path::new(&home).join(rest),
            None => Path::new(path).to_path_buf(),
        },
        None => Path::new(path).to_path_buf(),
    }
}

/// Read and check a graph before anything is filled into it.
///
/// The check exists because the most common way this fails is a person exporting the wrong thing
/// from the ComfyUI editor: the editor's own "Save" writes a document for humans, and posting it
/// to `/prompt` gets back a 400 that does not say which of the two you have. Answering that here,
/// in a sentence, saves a round trip through a server's log.
pub fn api_format(text: &str) -> Result<Value, String> {
    let value: Value = serde_json::from_str(text).map_err(|e| format!("it is not valid JSON ({e})"))?;
    // Somebody who saved the whole request body rather than the graph gets the graph out of it.
    let root = match value.get("prompt") {
        Some(inner) if inner.is_object() => inner.clone(),
        _ => value,
    };
    let Some(nodes) = root.as_object() else {
        return Err("its top level is not a map of nodes".to_string());
    };
    if nodes.contains_key("nodes") || nodes.contains_key("links") {
        return Err(
            "it looks like a workflow saved from the ComfyUI editor, not the API format Studio sends. In the editor, turn on Dev mode in its settings and use 'Save (API Format)'."
                .to_string(),
        );
    }
    let mut saw_a_node = false;
    for (id, node) in nodes {
        // Keys starting with an underscore are notes for a person reading the file, and are
        // stripped before the graph is sent; the shipped one carries its own explanation there.
        if id.starts_with('_') {
            continue;
        }
        let Some(node) = node.as_object() else {
            return Err(format!("node {id} is not an object"));
        };
        match node.get("class_type").and_then(|c| c.as_str()) {
            Some(class) if !class.is_empty() => saw_a_node = true,
            _ => return Err(format!("node {id} names no class_type, so ComfyUI cannot run it")),
        }
        if !node.contains_key("inputs") {
            return Err(format!("node {id} ({}) has no inputs", node["class_type"]));
        }
    }
    if !saw_a_node {
        return Err("it holds no nodes".to_string());
    }
    Ok(root)
}

/// Fill a graph with one request. Returns the graph to post and what happened while filling it.
pub fn fill(
    template: Value,
    request: &Request,
    model: &str,
    filename_prefix: &str,
) -> Result<Filled, String> {
    let Some(mut nodes) = template.as_object().cloned() else {
        return Err("the graph is not a map of nodes".to_string());
    };
    let mut filled = Filled::default();

    // The explanation in the shipped file is for a person reading the repository. It is stripped
    // here rather than left in, because ComfyUI would try to run it as a node.
    nodes.retain(|id, _| !id.starts_with('_'));

    // Every node id this needs is taken as an owned string up front. Holding a reference into
    // `nodes` while also writing to it is the mistake this avoids, and it costs one clone of an
    // id per graph.
    let sampler = first_node(&nodes, |class| class.starts_with("KSampler"));
    let advanced = sampler
        .as_ref()
        .is_some_and(|id| class_of(&nodes[id]).starts_with("KSamplerAdvanced"));

    // ── the sampler: seed, steps, guidance ──
    if let Some(id) = &sampler {
        if advanced {
            set(&mut nodes, id, "noise_seed", json!(request.seed), &mut filled, "the seed");
            set(&mut nodes, id, "steps", json!(request.steps), &mut filled, "the step count");
            // An advanced sampler is a *range* of steps. Leaving the graph's own start would make
            // it resume halfway through a pass nobody started, and the seed would mean nothing.
            set(&mut nodes, id, "start_at_step", json!(0), &mut filled, "the first step");
            set(&mut nodes, id, "end_at_step", json!(request.steps), &mut filled, "the last step");
            if has_key(&nodes, id, "cfg") {
                set(&mut nodes, id, "cfg", json!(request.cfg), &mut filled, "the guidance");
            } else {
                filled.missed.push(format!(
                    "the graph's sampler is a KSamplerAdvanced, which takes its guidance elsewhere, so cfg {} was not applied",
                    request.cfg
                ));
            }
        } else {
            set(&mut nodes, id, "seed", json!(request.seed), &mut filled, "the seed");
            set(&mut nodes, id, "steps", json!(request.steps), &mut filled, "the step count");
            set(&mut nodes, id, "cfg", json!(request.cfg), &mut filled, "the guidance");
            set(&mut nodes, id, "denoise", json!(1.0), &mut filled, "a full denoise");
        }
    } else {
        filled.missed.push(
            "the graph has no KSampler, so the seed, the step count and the guidance were not applied"
                .to_string(),
        );
    }

    // ── the two prompts, found by following the sampler's own links ──
    for (input, text, what) in [
        ("positive", &request.prompt, "the prompt"),
        ("negative", &request.negative, "the negative prompt"),
    ] {
        match linked_node(&nodes, sampler.as_deref(), input) {
            Some(id) if has_key(&nodes, &id, "text") => {
                set(&mut nodes, &id, "text", json!(text), &mut filled, what);
            }
            Some(id) => filled.missed.push(format!(
                "node {id} feeds the sampler's {input} input but takes no text, so {what} was not applied"
            )),
            None if input == "positive" => match first_node(&nodes, |class| class == "CLIPTextEncode") {
                Some(id) => {
                    set(&mut nodes, &id, "text", json!(text), &mut filled, what);
                    filled.missed.push(
                        "nothing is wired to the sampler's positive input, so the prompt went to the first text encoder in the graph instead".to_string(),
                    );
                }
                None => filled
                    .missed
                    .push("the graph has no text encoder, so the prompt was not applied".to_string()),
            },
            None => filled.missed.push(format!(
                "nothing is wired to the sampler's {input} input, so {what} was not applied"
            )),
        }
    }

    // ── the size, snapped to the latent grid ──
    let width = snap_to_grid(request.width);
    let height = snap_to_grid(request.height);
    let latent = linked_node(&nodes, sampler.as_deref(), "latent_image").or_else(|| {
        first_node(&nodes, |class| {
            class == "EmptyLatentImage"
                || class == "EmptySD3LatentImage"
                || class == "EmptyHunyuanLatent"
        })
    });
    if let Some(id) = &latent {
        set(&mut nodes, id, "width", json!(width), &mut filled, "the width");
        set(&mut nodes, id, "height", json!(height), &mut filled, "the height");
        set(&mut nodes, id, "batch_size", json!(1), &mut filled, "one image per pass");
        filled.size = Some((width, height));
    } else {
        filled.missed.push(format!(
            "the graph has no empty latent image, so {width}x{height} was not applied and the server drew at its own size"
        ));
    }

    // ── the checkpoint ──
    let loader = linked_node(&nodes, sampler.as_deref(), "model")
        .or_else(|| first_node(&nodes, |class| class.starts_with("CheckpointLoader")));
    match loader {
        Some(id) if !model.trim().is_empty() => {
            // `ckpt_name` is the usual spelling, but a LoRA-stacked graph may load its model some
            // other way. Setting a key the node does not have makes ComfyUI refuse the whole
            // graph, which is worse than saying this one part did not take.
            if has_key(&nodes, &id, "ckpt_name") {
                set(&mut nodes, &id, "ckpt_name", json!(model), &mut filled, &format!("the checkpoint {model}"));
            } else {
                filled.missed.push(format!(
                    "node {id} loads a model but has no ckpt_name, so {model} was not applied and the graph's own checkpoint was used"
                ));
            }
        }
        // No model named means "whatever the graph says", which is a reasonable thing to want
        // from a graph a person exported themselves.
        Some(_) => {}
        None if !model.trim().is_empty() => {
            filled.missed.push(format!("the graph loads no checkpoint, so {model} was not applied"))
        }
        None => {}
    }

    // ── where the server writes its own copy ──
    // The bytes are fetched back and saved under Studio's folder whatever this says; naming the
    // date here means a copy left on the server can be matched to the one in the gallery.
    if let Some(id) = first_node(&nodes, |class| class == "SaveImage") {
        set(&mut nodes, &id, "filename_prefix", json!(filename_prefix), &mut filled, "the server-side filename");
    }

    filled.graph = Value::Object(nodes);
    Ok(filled)
}

fn class_of(node: &Value) -> String {
    node.get("class_type").and_then(|c| c.as_str()).unwrap_or_default().to_string()
}

fn has_key(nodes: &Map<String, Value>, id: &str, key: &str) -> bool {
    nodes
        .get(id)
        .and_then(|node| node.get("inputs"))
        .and_then(|inputs| inputs.as_object())
        .is_some_and(|inputs| inputs.contains_key(key))
}

/// The id of the first node of a kind. Node ids are strings and a JSON map's order is
/// alphabetical, not the order the graph was drawn in, so numeric ids are ordered as numbers:
/// "the first KSampler" then means the first one a person would have meant in the editor.
fn first_node(nodes: &Map<String, Value>, wanted: impl Fn(&str) -> bool) -> Option<String> {
    let mut ids: Vec<&String> = nodes.keys().collect();
    ids.sort_by(|a, b| order_of(a).cmp(&order_of(b)));
    ids.iter()
        .map(|id| id.to_string())
        .find(|id| wanted(&class_of(&nodes[id])))
}

fn order_of(id: &str) -> (bool, i64, &str) {
    match id.parse::<i64>() {
        Ok(number) => (false, number, ""),
        Err(_) => (true, 0, id),
    }
}

/// The id of the node feeding one of the sampler's inputs, if that input is a link to a node
/// this graph actually holds.
fn linked_node(nodes: &Map<String, Value>, sampler: Option<&str>, input: &str) -> Option<String> {
    let link = nodes.get(sampler?)?.get("inputs")?.get(input)?;
    let id = link.as_array()?.first()?.as_str()?.to_string();
    nodes.contains_key(&id).then_some(id)
}

/// Write one value into one node's inputs, and say what happened either way.
fn set(nodes: &mut Map<String, Value>, id: &str, key: &str, value: Value, filled: &mut Filled, what: &str) {
    let Some(inputs) = nodes
        .get_mut(id)
        .and_then(|node| node.get_mut("inputs"))
        .and_then(|inputs| inputs.as_object_mut())
    else {
        // `api_format` checks every node has an inputs object, so this is only reachable for a
        // graph built in memory by a test. Reported rather than panicked: this runs on a worker.
        filled.missed.push(format!("node {id} has no inputs, so {what} was not applied"));
        return;
    };
    // Only a key the node already has. Adding one ComfyUI does not expect makes it refuse the
    // whole graph, which is a worse outcome than reporting that this part did not take.
    if inputs.contains_key(key) {
        inputs.insert(key.into(), value);
        filled.applied.push(what.to_string());
    } else {
        filled
            .missed
            .push(format!("node {id} has no `{key}` input, so {what} was not applied"));
    }
}

#[cfg(test)]
#[path = "workflow_tests.rs"]
mod tests;
