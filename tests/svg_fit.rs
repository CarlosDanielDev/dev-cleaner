//! The banner SVGs must fit their own canvas.
//!
//! The first release shipped a hero whose tagline ran past the right edge and
//! was cut on GitHub, because the text was positioned from an assumed glyph
//! width. The generator now wraps the tagline and gives each line a
//! `textLength`; this test reads the committed files and fails if any such line
//! would end outside the image.

use std::fs;
use std::path::PathBuf;

fn read(name: &str) -> String {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("docs/img")
        .join(name);
    fs::read_to_string(&path).unwrap_or_else(|e| panic!("{}: {e}", path.display()))
}

/// The value of `attr="..."` on the first tag that has it.
fn attr(tag: &str, name: &str) -> Option<f64> {
    let key = format!(" {name}=\"");
    let start = tag.find(&key)? + key.len();
    tag[start..].split('"').next()?.parse().ok()
}

fn check(name: &str) {
    let svg = read(name);
    let root = svg.lines().next().expect("an svg");
    let width = attr(root, "width").expect("the canvas has a width");

    let lines: Vec<&str> = svg.lines().filter(|l| l.contains("textLength=")).collect();
    assert_eq!(lines.len(), 3, "{name}: the tagline is three lines");
    for line in lines {
        let x = attr(line, "x").expect("a text line has an x");
        let len = attr(line, "textLength").expect("a text line has a textLength");
        assert!(
            x + len <= width - 24.0,
            "{name}: a tagline line ends at {} on a canvas {width} wide:\n{line}",
            x + len
        );
    }
}

#[test]
fn the_hero_tagline_ends_inside_the_image() {
    check("hero.svg");
}

#[test]
fn the_social_preview_tagline_ends_inside_the_image() {
    check("social-preview.svg");
}
