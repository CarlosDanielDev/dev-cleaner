//! The table screens share one set of primitives, and none of them carries its
//! own column arithmetic any more. The three call sites are Projects,
//! Candidates and the plan (Review); the primitives are in `src/tui/kit`.

use std::fs;

fn source(file: &str) -> String {
    fs::read_to_string(format!("{}/src/tui/{file}", env!("CARGO_MANIFEST_DIR")))
        .unwrap_or_else(|e| panic!("{file}: {e}"))
}

const TABLE_SCREENS: [&str; 3] = ["projects.rs", "candidates.rs", "review.rs"];

#[test]
fn every_table_screen_declares_its_columns_on_the_shared_table() {
    for file in TABLE_SCREENS {
        let src = source(file);
        assert!(
            src.contains("Table::new("),
            "{file} does not build a kit::Table"
        );
        assert!(
            src.contains("kit::") || src.contains("use super::kit"),
            "{file}"
        );
    }
}

#[test]
fn no_table_screen_has_its_own_column_width_arithmetic() {
    // What a screen used to carry: a width function per screen, a way to
    // right-align a figure by hand, and the old shared row layout.
    let own = [
        "fn layout(",
        "fn fit(",
        "fn aligned(",
        "row::columns",
        "row::widest",
        "columns(left",
        "path_w",
        "desc_w",
        "format!(\"{text:>width$}\")",
    ];
    for file in TABLE_SCREENS {
        let src = source(file);
        for needle in own {
            assert!(!src.contains(needle), "{file} still has {needle:?}");
        }
    }
}

#[test]
fn the_kit_is_where_the_pieces_live() {
    let table = source("kit/table.rs");
    for piece in ["fn fit(", "fn aligned(", "Align::Right"] {
        assert!(table.contains(piece), "kit/table.rs lacks {piece:?}");
    }
    let lines = source("kit/lines.rs");
    for piece in [
        "fn section(",
        "fn view_bar(",
        "fn detail(",
        "fn empty_body(",
        "fn band(",
    ] {
        assert!(lines.contains(piece), "kit/lines.rs lacks {piece:?}");
    }
}
