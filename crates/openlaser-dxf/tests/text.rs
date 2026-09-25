// SPDX-License-Identifier: GPL-3.0-or-later
//! DXF lettering retains nominal height, alignment, rotation and neighbouring open paths.
#![allow(clippy::unwrap_used, clippy::expect_used, reason = "known DXF fixtures")]

use openlaser_dxf::import;

fn file(entities: &str, units: u32) -> Vec<u8> {
    format!("0\nSECTION\n2\nHEADER\n9\n$INSUNITS\n70\n{units}\n0\nENDSEC\n0\nSECTION\n2\nENTITIES\n{entities}0\nENDSEC\n0\nEOF\n").into_bytes()
}

fn text(value: &str, fields: &str) -> String {
    format!("0\nTEXT\n8\nletters\n10\n20\n20\n30\n40\n10\n1\n{value}\n{fields}")
}

#[test]
fn nominal_text_height_units_rotation_and_width_are_preserved() {
    let a = import(&file(&text("H", ""), 4)).unwrap().drawing.bounds().unwrap();
    assert!((a.height() - 10.).abs() < 1e-4);
    assert!((a.min.y - 30.).abs() < 1e-4);
    let inches = import(&file(&text("H", ""), 1)).unwrap().drawing.bounds().unwrap();
    assert!((inches.height() - 254.).abs() < 1e-3);
    let b = import(&file(&text("H", "50\n90\n41\n2\n"), 4)).unwrap().drawing.bounds().unwrap();
    assert!((b.width() - a.height()).abs() < 1e-4);
    assert!((b.height() - 2. * a.width()).abs() < 1e-4);
    let right = import(&file(&text("H", "72\n2\n73\n3\n11\n50\n21\n60\n"), 4))
        .unwrap()
        .drawing
        .bounds()
        .unwrap();
    assert!((right.max.x - 50.).abs() < 1e-4);
    assert!((right.max.y - 60.).abs() < 1e-4);
}

#[test]
fn mtext_chunks_spaces_unicode_and_line_spacing_are_preserved() {
    let source = "0\nMTEXT\n10\n5\n20\n40\n40\n6\n71\n1\n44\n1.5\n3\nH \n1\nH\\PH\n";
    let drawing = import(&file(source, 4)).unwrap().drawing;
    assert_eq!(drawing.contours.len(), 3);
    assert!((drawing.bounds().unwrap().height() - 21.).abs() < 1e-4);
    let without_space = source.replace("H \n", "H\n");
    assert!(
        drawing.bounds().unwrap().width()
            > import(&file(&without_space, 4)).unwrap().drawing.bounds().unwrap().width()
    );
    let unicode = import(&file(&text("Caf\\U+00E9 %%d %%p %%c", ""), 4)).unwrap();
    assert!(unicode.drawing.contours.len() > 7);
    assert_eq!(unicode.warnings.len(), 1);
}

#[test]
fn text_holes_and_open_geometry_coexist() {
    let entities = format!(
        "{}0\nLINE\n8\nopen\n10\n0\n20\n0\n11\n20\n21\n0\n0\nLWPOLYLINE\n8\nopen\n90\n3\n70\n0\n10\n30\n20\n0\n10\n35\n20\n5\n10\n40\n20\n0\n",
        text("B", "")
    );
    let drawing = import(&file(&entities, 4)).unwrap().drawing;
    assert_eq!(drawing.contours.iter().filter(|c| c.is_closed()).count(), 3);
    assert_eq!(drawing.contours.iter().filter(|c| !c.is_closed()).count(), 2);
    assert!(drawing.contours.iter().filter(|c| c.is_closed()).all(|c| c.layer == "letters"));
}

#[test]
fn text_that_cannot_be_drawn_is_left_out_named_and_never_silent() {
    // Alone, it leaves nothing to cut: the import refuses, naming the text.
    for source in [text("H", "72\n5\n"), text("\\U+ZZZZ", ""), text("🚧", "")] {
        assert!(import(&file(&source, 4)).is_err(), "{source}");
    }
    // Beside a line, the drawing opens and says what was left out.
    let beside = format!("{}0\nLINE\n10\n0\n20\n0\n11\n5\n21\n0\n", text("🚧", ""));
    let opened = import(&file(&beside, 4)).unwrap();
    assert_eq!(opened.drawing.contours.len(), 1);
    assert!(
        opened.warnings.iter().any(|w| w.contains("the text was left out")),
        "{:?}",
        opened.warnings
    );
}

#[test]
fn mtext_wraps_to_its_box_and_drops_formatting_it_cannot_draw() {
    let one_row = import(&file("0\nMTEXT\n10\n0\n20\n0\n40\n10\n1\nHH HH HH\n", 4)).unwrap();
    let wrapped =
        import(&file("0\nMTEXT\n10\n0\n20\n0\n40\n10\n41\n25\n1\nHH HH HH\n", 4)).unwrap();
    let height = |i: &openlaser_dxf::Import| i.drawing.bounds().unwrap().height();
    assert!(height(&wrapped) > 2. * height(&one_row), "three rows");
    assert!(wrapped.drawing.bounds().unwrap().width() < 25.);
    let formatted =
        import(&file("0\nMTEXT\n10\n0\n20\n0\n40\n10\n1\n{\\H2x;\\fArial|b1;H}\\S1^2;\\NH\n", 4))
            .unwrap();
    assert!(
        formatted.warnings.iter().any(|w| w.contains("MTEXT fonts")),
        "{:?}",
        formatted.warnings
    );
    // TEXT has no formatting codes: a backslash is drawn.
    assert!(import(&file(&text("A\\B", ""), 4)).is_ok());
}

#[test]
fn older_files_are_read_in_their_code_page() {
    let mut western = file(&text("X", ""), 4);
    let at = western.windows(3).position(|w| w == b"\nX\n").unwrap() + 1;
    western[at] = 0xD8; // Ø in Windows-1252
    let opened = import(&western).unwrap();
    assert!(!opened.drawing.contours.is_empty());
}
