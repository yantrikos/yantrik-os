//! A rounded element must not have a linear-gradient fill (#172).
//!
//! Slint's software renderer, the one every GPU-less machine and VM 520 draws with, ignores
//! `border-radius` on a gradient `background`: the fill paints a square inside the rounded border
//! and the corners poke out. A `clip: true` parent does not help either, because that renderer
//! does not round what it clips. So a rounded element's fill is solid, and a gradient bar that
//! needs round ends draws them as solid caps (see the weather forecast's range bar).
//!
//! Radial gradients are left alone: every one in the shell fades to transparent before the
//! corners, which is what makes a square fill invisible there.

use std::path::{Path, PathBuf};

fn slint_files(dir: &Path, out: &mut Vec<PathBuf>) {
    for entry in std::fs::read_dir(dir).unwrap_or_else(|e| panic!("{}: {e}", dir.display())) {
        let path = entry.unwrap().path();
        if path.is_dir() {
            slint_files(&path, out);
        } else if path.extension().is_some_and(|e| e == "slint") {
            out.push(path);
        }
    }
}

/// Whether a radius expression can be anything but zero. `0`, `0px` and `0.0px` are the only
/// zeros written in the shell; anything else, a conditional included, can round.
fn rounds(value: &str) -> bool {
    !matches!(value.trim().trim_end_matches(';').trim(), "0" | "0px" | "0.0px")
}

/// Every element, by the line it opens on, that has both a nonzero radius and a linear-gradient
/// `background`, looking only at that element's own properties and not its children's.
fn rounded_gradients(source: &str) -> Vec<usize> {
    struct Element {
        line: usize,
        radius: bool,
        gradient: bool,
    }
    let mut stack = vec![Element { line: 0, radius: false, gradient: false }];
    let mut found = Vec::new();
    // A background whose value runs on past this line (a conditional, a long gradient): the
    // lines up to its `;` are still its value.
    let mut background_continues = false;
    for (n, line) in source.lines().enumerate() {
        let code = line.split("//").next().unwrap_or("");
        let property = code.trim();
        if background_continues {
            if property.contains("@linear-gradient") {
                stack.last_mut().unwrap().gradient = true;
            }
            background_continues = !property.contains(';');
            continue;
        }
        if let Some((name, value)) = property.split_once(':') {
            let top = stack.last_mut().unwrap();
            let name = name.trim();
            if name.ends_with("radius") && name.starts_with("border") && rounds(value) {
                top.radius = true;
            }
            if name == "background" {
                top.gradient |= value.contains("@linear-gradient");
                background_continues = !value.contains(';');
            }
        }
        for c in code.chars() {
            match c {
                '{' => stack.push(Element { line: n + 1, radius: false, gradient: false }),
                '}' if stack.len() > 1 => {
                    let element = stack.pop().unwrap();
                    if element.radius && element.gradient {
                        found.push(element.line);
                    }
                }
                _ => {}
            }
        }
    }
    found
}

#[test]
fn no_rounded_element_has_a_linear_gradient_fill() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    let mut files = Vec::new();
    slint_files(&root.join("ui"), &mut files);
    slint_files(&root.join("../yantrik-ui-kit/slint"), &mut files);
    assert!(files.len() > 20, "found only {} .slint files; is the path right?", files.len());

    let mut problems = Vec::new();
    for file in &files {
        let source = std::fs::read_to_string(file).unwrap();
        for line in rounded_gradients(&source) {
            problems.push(format!("{}:{line}", file.strip_prefix(root).unwrap_or(file).display()));
        }
    }
    assert!(
        problems.is_empty(),
        "rounded elements with a linear-gradient fill draw square corners under the software \
         renderer (#172); make the fill solid:\n  {}",
        problems.join("\n  ")
    );
}

#[test]
fn the_scanner_sees_a_rounded_gradient_and_nothing_else() {
    let rounded = "Rectangle {\n    border-radius: 12px;\n    background: @linear-gradient(90deg, red 0%, blue 100%);\n}\n";
    assert_eq!(rounded_gradients(rounded), [1]);

    let conditional = "Rectangle {\n    border-radius: 14px;\n    background: root.on\n        ? @linear-gradient(135deg, red 0%, blue 100%)\n        : red;\n}\n";
    assert_eq!(rounded_gradients(conditional), [1]);

    let one_corner = "A := Rectangle {\n    background: @linear-gradient(90deg,\n        red 0%, blue 100%);\n    border-top-left-radius: root.flat ? 0px : 12px;\n}\n";
    assert_eq!(rounded_gradients(one_corner), [1]);

    // A square gradient, a rounded solid fill, and a gradient child inside a rounded parent are
    // each fine on their own element.
    let square = "Rectangle {\n    border-radius: 0px;\n    background: @linear-gradient(90deg, red 0%, blue 100%);\n}\n";
    assert!(rounded_gradients(square).is_empty());
    let solid = "Rectangle {\n    border-radius: 8px;\n    background: red;\n}\n";
    assert!(rounded_gradients(solid).is_empty());
    let nested = "Rectangle {\n    border-radius: 8px;\n    background: red;\n    Rectangle {\n        background: @linear-gradient(90deg, red 0%, blue 100%);\n    }\n}\n";
    assert!(rounded_gradients(nested).is_empty());
    let radial = "Rectangle {\n    border-radius: 40px;\n    background: @radial-gradient(circle, red 0%, transparent 60%);\n}\n";
    assert!(rounded_gradients(radial).is_empty());
}
