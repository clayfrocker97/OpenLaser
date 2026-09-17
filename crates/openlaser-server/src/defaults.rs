// SPDX-License-Identifier: GPL-3.0-or-later

//! Bundled metal recipes with original XML bytes and OpenLaser material artwork.

/// The supplied clean metal library; nozzle variants remain separate.
pub const METALS: &[(&str, &[u8])] = &[
    (
        "Aluminum - 1mm - AIR - 2.0S.xml",
        include_bytes!("../defaults/metal/Aluminum - 1mm - AIR - 2.0S.xml"),
    ),
    (
        "Aluminum - 1mm - N2 - 2.0S.xml",
        include_bytes!("../defaults/metal/Aluminum - 1mm - N2 - 2.0S.xml"),
    ),
    (
        "Aluminum - 2mm - AIR - 2.0S.xml",
        include_bytes!("../defaults/metal/Aluminum - 2mm - AIR - 2.0S.xml"),
    ),
    (
        "Aluminum - 2mm - N2 - 2.0S.xml",
        include_bytes!("../defaults/metal/Aluminum - 2mm - N2 - 2.0S.xml"),
    ),
    (
        "Aluminum - 3mm - AIR - 2.0S.xml",
        include_bytes!("../defaults/metal/Aluminum - 3mm - AIR - 2.0S.xml"),
    ),
    (
        "Aluminum - 3mm - N2 - 2.0S.xml",
        include_bytes!("../defaults/metal/Aluminum - 3mm - N2 - 2.0S.xml"),
    ),
    (
        "Aluminum - 4mm - AIR - 2.0S.xml",
        include_bytes!("../defaults/metal/Aluminum - 4mm - AIR - 2.0S.xml"),
    ),
    (
        "Aluminum - 4mm - N2 - 2.0S.xml",
        include_bytes!("../defaults/metal/Aluminum - 4mm - N2 - 2.0S.xml"),
    ),
    (
        "Aluminum - 5mm - AIR - 2.0S.xml",
        include_bytes!("../defaults/metal/Aluminum - 5mm - AIR - 2.0S.xml"),
    ),
    (
        "Aluminum - 6mm - AIR - 2.0S.xml",
        include_bytes!("../defaults/metal/Aluminum - 6mm - AIR - 2.0S.xml"),
    ),
    (
        "Brass - 1mm - AIR - 2.0S.xml",
        include_bytes!("../defaults/metal/Brass - 1mm - AIR - 2.0S.xml"),
    ),
    (
        "Brass - 1mm - N2 - 2.0S.xml",
        include_bytes!("../defaults/metal/Brass - 1mm - N2 - 2.0S.xml"),
    ),
    (
        "Brass - 2mm - AIR - 2.0S.xml",
        include_bytes!("../defaults/metal/Brass - 2mm - AIR - 2.0S.xml"),
    ),
    (
        "Brass - 2mm - N2 - 2.0S.xml",
        include_bytes!("../defaults/metal/Brass - 2mm - N2 - 2.0S.xml"),
    ),
    (
        "Brass - 3mm - AIR - 2.0S.xml",
        include_bytes!("../defaults/metal/Brass - 3mm - AIR - 2.0S.xml"),
    ),
    (
        "Brass - 3mm - N2 - 2.0S.xml",
        include_bytes!("../defaults/metal/Brass - 3mm - N2 - 2.0S.xml"),
    ),
    (
        "Carbon Steel - 10mm - O2 - 4.0D.xml",
        include_bytes!("../defaults/metal/Carbon Steel - 10mm - O2 - 4.0D.xml"),
    ),
    (
        "Carbon Steel - 1mm - AIR - 2.0S.xml",
        include_bytes!("../defaults/metal/Carbon Steel - 1mm - AIR - 2.0S.xml"),
    ),
    (
        "Carbon Steel - 2mm - AIR - 2.0S.xml",
        include_bytes!("../defaults/metal/Carbon Steel - 2mm - AIR - 2.0S.xml"),
    ),
    (
        "Carbon Steel - 3mm - AIR - 2.0S.xml",
        include_bytes!("../defaults/metal/Carbon Steel - 3mm - AIR - 2.0S.xml"),
    ),
    (
        "Carbon Steel - 3mm - O2 - 1.2D.xml",
        include_bytes!("../defaults/metal/Carbon Steel - 3mm - O2 - 1.2D.xml"),
    ),
    (
        "Carbon Steel - 4mm - AIR - 2.0S.xml",
        include_bytes!("../defaults/metal/Carbon Steel - 4mm - AIR - 2.0S.xml"),
    ),
    (
        "Carbon Steel - 4mm - O2 - 1.5D.xml",
        include_bytes!("../defaults/metal/Carbon Steel - 4mm - O2 - 1.5D.xml"),
    ),
    (
        "Carbon Steel - 5mm - AIR - 2.0S.xml",
        include_bytes!("../defaults/metal/Carbon Steel - 5mm - AIR - 2.0S.xml"),
    ),
    (
        "Carbon Steel - 5mm - O2 - 1.5D.xml",
        include_bytes!("../defaults/metal/Carbon Steel - 5mm - O2 - 1.5D.xml"),
    ),
    (
        "Carbon Steel - 6mm - AIR - 2.0S.xml",
        include_bytes!("../defaults/metal/Carbon Steel - 6mm - AIR - 2.0S.xml"),
    ),
    (
        "Carbon Steel - 6mm - O2 - 1.5D.xml",
        include_bytes!("../defaults/metal/Carbon Steel - 6mm - O2 - 1.5D.xml"),
    ),
    (
        "Carbon Steel - 7mm - O2 - 2.0D.xml",
        include_bytes!("../defaults/metal/Carbon Steel - 7mm - O2 - 2.0D.xml"),
    ),
    (
        "Carbon Steel - 8mm - O2 - 2.0D.xml",
        include_bytes!("../defaults/metal/Carbon Steel - 8mm - O2 - 2.0D.xml"),
    ),
    (
        "Galvanized Steel - 1mm - AIR - 2.0S.xml",
        include_bytes!("../defaults/metal/Galvanized Steel - 1mm - AIR - 2.0S.xml"),
    ),
    (
        "Galvanized Steel - 2mm - AIR - 2.0S.xml",
        include_bytes!("../defaults/metal/Galvanized Steel - 2mm - AIR - 2.0S.xml"),
    ),
    (
        "Galvanized Steel - 3mm - AIR - 2.0S.xml",
        include_bytes!("../defaults/metal/Galvanized Steel - 3mm - AIR - 2.0S.xml"),
    ),
    (
        "Galvanized Steel - 4mm - AIR - 2.0S.xml",
        include_bytes!("../defaults/metal/Galvanized Steel - 4mm - AIR - 2.0S.xml"),
    ),
    (
        "Galvanized Steel - 5mm - AIR - 2.0S.xml",
        include_bytes!("../defaults/metal/Galvanized Steel - 5mm - AIR - 2.0S.xml"),
    ),
    (
        "Stainless Steel - 1mm - AIR - 2.0S.xml",
        include_bytes!("../defaults/metal/Stainless Steel - 1mm - AIR - 2.0S.xml"),
    ),
    (
        "Stainless Steel - 1mm - N2 - 1.5S.xml",
        include_bytes!("../defaults/metal/Stainless Steel - 1mm - N2 - 1.5S.xml"),
    ),
    (
        "Stainless Steel - 1mm - N2 - 2.0S.xml",
        include_bytes!("../defaults/metal/Stainless Steel - 1mm - N2 - 2.0S.xml"),
    ),
    (
        "Stainless Steel - 2mm - AIR - 2.0S.xml",
        include_bytes!("../defaults/metal/Stainless Steel - 2mm - AIR - 2.0S.xml"),
    ),
    (
        "Stainless Steel - 2mm - N2 - 1.5S.xml",
        include_bytes!("../defaults/metal/Stainless Steel - 2mm - N2 - 1.5S.xml"),
    ),
    (
        "Stainless Steel - 2mm - N2 - 2.0S.xml",
        include_bytes!("../defaults/metal/Stainless Steel - 2mm - N2 - 2.0S.xml"),
    ),
    (
        "Stainless Steel - 3mm - AIR - 2.0S.xml",
        include_bytes!("../defaults/metal/Stainless Steel - 3mm - AIR - 2.0S.xml"),
    ),
    (
        "Stainless Steel - 3mm - N2 - 1.5S.xml",
        include_bytes!("../defaults/metal/Stainless Steel - 3mm - N2 - 1.5S.xml"),
    ),
    (
        "Stainless Steel - 3mm - N2 - 2.0S.xml",
        include_bytes!("../defaults/metal/Stainless Steel - 3mm - N2 - 2.0S.xml"),
    ),
    (
        "Stainless Steel - 4mm - AIR - 2.0S.xml",
        include_bytes!("../defaults/metal/Stainless Steel - 4mm - AIR - 2.0S.xml"),
    ),
    (
        "Stainless Steel - 4mm - N2 - 1.5S.xml",
        include_bytes!("../defaults/metal/Stainless Steel - 4mm - N2 - 1.5S.xml"),
    ),
    (
        "Stainless Steel - 4mm - N2 - 2.0S.xml",
        include_bytes!("../defaults/metal/Stainless Steel - 4mm - N2 - 2.0S.xml"),
    ),
    (
        "Stainless Steel - 5mm - AIR - 2.0S.xml",
        include_bytes!("../defaults/metal/Stainless Steel - 5mm - AIR - 2.0S.xml"),
    ),
    (
        "Stainless Steel - 5mm - N2 - 1.5S.xml",
        include_bytes!("../defaults/metal/Stainless Steel - 5mm - N2 - 1.5S.xml"),
    ),
    (
        "Stainless Steel - 5mm - N2 - 2.0D.xml",
        include_bytes!("../defaults/metal/Stainless Steel - 5mm - N2 - 2.0D.xml"),
    ),
    (
        "Stainless Steel - 6mm - AIR - 2.0S.xml",
        include_bytes!("../defaults/metal/Stainless Steel - 6mm - AIR - 2.0S.xml"),
    ),
];

/// Existing CO2 starter recipe.
pub const CO2: (&str, &[u8]) =
    ("Basswood-3MM_CO2 CUTTING.xml", include_bytes!("../defaults/Basswood-3MM_CO2 CUTTING.xml"));
