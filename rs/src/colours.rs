//! Dynamic colour schemes from an image, ported from `utils/material/` in the Python CLI: the
//! Material scheme for the image's best colour, plus terminal, Catppuccin-named and KDE colours
//! harmonised to it.

use crate::scheme::as_colours;
use crate::util::{Colours, atomic_write, dumps, hash_file};
use material_colors::blend::cam16_ucs;
use material_colors::color::Rgb;
use material_colors::dynamic_color::{DynamicScheme, Platform, Role, SpecVersion, Variant};
use material_colors::hct::Hct;
use material_colors::quantize::{Quantizer, QuantizerCelebi};
use serde_json::Value;
use std::fs;
use std::io;
use std::path::Path;

/// The cached scheme for `image`, generating (and caching) it on a miss.
pub fn for_image(c_cache: &Path, image: &Path, variant: &str, flavour: &str, mode: &str) -> io::Result<Colours> {
    let base = c_cache.join("schemes").join(hash_file(image)?);
    let cache = base.join(variant).join(flavour).join(mode).with_extension("json");
    if let Some(c) = fs::read_to_string(&cache).ok().and_then(|t| serde_json::from_str(&t).ok()).and_then(as_colours) {
        return Ok(c);
    }
    let score = base.join("score.json");
    let primary = match fs::read_to_string(&score).ok().and_then(|t| t.trim().parse::<u32>().ok()) {
        Some(argb) => Hct::new(Rgb::from_u32(argb)),
        None => {
            let pixels = image::open(image).map_err(io::Error::other)?.to_rgb8();
            let primary = best_colour(pixels.pixels().map(|p| Rgb::new(p[0], p[1], p[2])).collect());
            atomic_write(&score, Rgb::from(primary).as_argb_u32().to_string())?;
            primary
        }
    };
    let colours = scheme(primary, variant, flavour, mode == "light");
    atomic_write(&cache, dumps(&Value::Object(colours.clone())))?;
    Ok(colours)
}

/// Python's `round()`: halves go to the even neighbour.
fn round(x: f64) -> f64 {
    x.round_ties_even()
}

/// `score.Score.score` (filtering off): the most suitable source colour among the image's 128
/// quantised colours.
fn best_colour(pixels: Vec<Rgb>) -> Hct {
    const TARGET_CHROMA: f64 = 48.0;
    // Python iterates the quantiser's dict, which is sorted by colour; the stable sort keeps that for ties
    let mut counts: Vec<(Rgb, u32)> = QuantizerCelebi::quantize(&pixels, 128).color_to_count.into_iter().collect();
    counts.sort_by_key(|(rgb, _)| *rgb);

    let hcts: Vec<Hct> = counts.iter().map(|(rgb, _)| Hct::new(*rgb)).collect();
    let mut hue_population = [0u64; 360];
    for (hct, (_, count)) in hcts.iter().zip(&counts) {
        hue_population[hct.get_hue() as usize] += u64::from(*count);
    }
    let total: u64 = hue_population.iter().sum();
    let mut excited = [0f64; 360];
    for (hue, population) in hue_population.iter().enumerate() {
        let proportion = *population as f64 / total as f64;
        for i in hue as i64 - 14..hue as i64 + 16 {
            excited[i.rem_euclid(360) as usize] += proportion;
        }
    }

    let mut scored: Vec<(Hct, f64)> = hcts
        .into_iter()
        .map(|hct| {
            let proportion = excited[(round(hct.get_hue()) as i64).rem_euclid(360) as usize];
            let chroma = hct.get_chroma();
            let weight = if chroma < TARGET_CHROMA { 0.1 } else { 0.3 };
            (hct, proportion * 100.0 * 0.7 + (chroma - TARGET_CHROMA) * weight)
        })
        .collect();
    scored.sort_by(|a, b| b.1.total_cmp(&a.1));

    // Python recurses forever when nothing passes (an all-black image); take the top colour instead
    let primary = (0..=20)
        .rev()
        .find_map(|cutoff| {
            let c = f64::from(cutoff);
            scored.iter().find(|(h, _)| h.get_chroma() > c && h.get_tone() > c * 3.0).map(|(h, _)| *h)
        })
        .unwrap_or(scored[0].0);
    fix_if_disliked(primary)
}

/// `DislikeAnalyzer.fix_if_disliked`, with Python's rounding.
fn fix_if_disliked(hct: Hct) -> Hct {
    let (h, c, t) = (round(hct.get_hue()), round(hct.get_chroma()), round(hct.get_tone()));
    if (90.0..=111.0).contains(&h) && c > 16.0 && t < 65.0 {
        Hct::from(hct.get_hue(), hct.get_chroma(), 70.0)
    } else {
        hct
    }
}

/// The Material colours, in the order Python's `MaterialDynamicColors.all_colors` lists them.
const MATERIAL: [(&str, Role); 59] = {
    use Role::*;
    [
        ("background", Background),
        ("onBackground", OnBackground),
        ("surface", Surface),
        ("surfaceDim", SurfaceDim),
        ("surfaceBright", SurfaceBright),
        ("surfaceContainerLowest", SurfaceContainerLowest),
        ("surfaceContainerLow", SurfaceContainerLow),
        ("surfaceContainer", SurfaceContainer),
        ("surfaceContainerHigh", SurfaceContainerHigh),
        ("surfaceContainerHighest", SurfaceContainerHighest),
        ("onSurface", OnSurface),
        ("surfaceVariant", SurfaceVariant),
        ("onSurfaceVariant", OnSurfaceVariant),
        ("outline", Outline),
        ("outlineVariant", OutlineVariant),
        ("inverseSurface", InverseSurface),
        ("inverseOnSurface", InverseOnSurface),
        ("shadow", Shadow),
        ("scrim", Scrim),
        ("surfaceTint", SurfaceTint),
        ("primary", Primary),
        ("primaryDim", PrimaryDim),
        ("onPrimary", OnPrimary),
        ("primaryContainer", PrimaryContainer),
        ("onPrimaryContainer", OnPrimaryContainer),
        ("inversePrimary", InversePrimary),
        ("primaryFixed", PrimaryFixed),
        ("primaryFixedDim", PrimaryFixedDim),
        ("onPrimaryFixed", OnPrimaryFixed),
        ("onPrimaryFixedVariant", OnPrimaryFixedVariant),
        ("secondary", Secondary),
        ("secondaryDim", SecondaryDim),
        ("onSecondary", OnSecondary),
        ("secondaryContainer", SecondaryContainer),
        ("onSecondaryContainer", OnSecondaryContainer),
        ("secondaryFixed", SecondaryFixed),
        ("secondaryFixedDim", SecondaryFixedDim),
        ("onSecondaryFixed", OnSecondaryFixed),
        ("onSecondaryFixedVariant", OnSecondaryFixedVariant),
        ("tertiary", Tertiary),
        ("tertiaryDim", TertiaryDim),
        ("onTertiary", OnTertiary),
        ("tertiaryContainer", TertiaryContainer),
        ("onTertiaryContainer", OnTertiaryContainer),
        ("tertiaryFixed", TertiaryFixed),
        ("tertiaryFixedDim", TertiaryFixedDim),
        ("onTertiaryFixed", OnTertiaryFixed),
        ("onTertiaryFixedVariant", OnTertiaryFixedVariant),
        ("error", Error),
        ("errorDim", ErrorDim),
        ("onError", OnError),
        ("errorContainer", ErrorContainer),
        ("onErrorContainer", OnErrorContainer),
        ("primaryPaletteKeyColor", PrimaryPaletteKeyColor),
        ("secondaryPaletteKeyColor", SecondaryPaletteKeyColor),
        ("tertiaryPaletteKeyColor", TertiaryPaletteKeyColor),
        ("neutralPaletteKeyColor", NeutralPaletteKeyColor),
        ("neutralVariantPaletteKeyColor", NeutralVariantPaletteKeyColor),
        ("errorPaletteKeyColor", ErrorPaletteKeyColor),
    ]
};

/// Old names for the palette key colours. In Python they are the same objects as the new names.
const ALIASES: [(&str, &str); 5] = [
    ("primary_paletteKeyColor", "primaryPaletteKeyColor"),
    ("secondary_paletteKeyColor", "secondaryPaletteKeyColor"),
    ("tertiary_paletteKeyColor", "tertiaryPaletteKeyColor"),
    ("neutral_paletteKeyColor", "neutralPaletteKeyColor"),
    ("neutral_variant_paletteKeyColor", "neutralVariantPaletteKeyColor"),
];

const LIGHT_TERM: [u32; 16] = [
    0xFDF9F3, 0xFF6188, 0xA9DC76, 0xFC9867, 0xFFD866, 0xF47FD4, 0x78DCE8, 0x333034, 0x121212, 0xFF6188, 0xA9DC76,
    0xFC9867, 0xFFD866, 0xF47FD4, 0x78DCE8, 0x333034,
];
const DARK_TERM: [u32; 16] = [
    0x282828, 0xCC241D, 0x98971A, 0xD79921, 0x458588, 0xB16286, 0x689D6A, 0xA89984, 0x928374, 0xFB4934, 0xB8BB26,
    0xFABD2F, 0x83A598, 0xD3869B, 0x8EC07C, 0xEBDBB2,
];
const NAMED: [&str; 14] = [
    "rosewater",
    "flamingo",
    "pink",
    "mauve",
    "red",
    "maroon",
    "peach",
    "yellow",
    "green",
    "teal",
    "sky",
    "sapphire",
    "blue",
    "lavender",
];
const LIGHT_NAMED: [u32; 14] = [
    0xdc8a78, 0xdd7878, 0xea76cb, 0x8839ef, 0xd20f39, 0xe64553, 0xfe640b, 0xdf8e1d, 0x40a02b, 0x179299, 0x04a5e5,
    0x209fb5, 0x1e66f5, 0x7287fd,
];
const DARK_NAMED: [u32; 14] = [
    0xf5e0dc, 0xf2cdcd, 0xf5c2e7, 0xcba6f7, 0xf38ba8, 0xeba0ac, 0xfab387, 0xf9e2af, 0xa6e3a1, 0x94e2d5, 0x89dceb,
    0x74c7ec, 0x89b4fa, 0xb4befe,
];
const KDE: [(&str, u32); 5] = [
    ("klink", 0x2980b9),
    ("kvisited", 0x9b59b6),
    ("knegative", 0xda4453),
    ("kneutral", 0xf67400),
    ("kpositive", 0x27ae60),
];

fn hct(rgb: u32) -> Hct {
    Hct::new(Rgb::from_u32(rgb))
}

fn lighten(c: Hct, amount: f64) -> Hct {
    let diff = (100.0 - c.get_tone()) * amount;
    Hct::from(c.get_hue(), c.get_chroma() + diff / 5.0, c.get_tone() + diff)
}

fn darken(c: Hct, amount: f64) -> Hct {
    let diff = c.get_tone() * amount;
    Hct::from(c.get_hue(), c.get_chroma() - diff / 5.0, c.get_tone() - diff)
}

fn grayscale(c: Hct, light: bool) -> Hct {
    let mut c = if light { darken(c, 0.35) } else { lighten(c, 0.65) };
    c.set_chroma(0.0);
    c
}

fn mix(a: Hct, b: Hct, w: f64) -> Hct {
    Hct::new(cam16_ucs(a.into(), b.into(), w))
}

/// `Python sanitize_degrees_double`: `x % 360.0`, which is never negative.
fn sanitize(x: f64) -> f64 {
    let r = x % 360.0;
    if r < 0.0 { r + 360.0 } else { r }
}

/// Rotate `from`'s hue up to 80% of the way to `to`'s (at most 100 degrees), scaling its tone.
fn harmonize(from: Hct, to: Hct, tone_boost: f64) -> Hct {
    let (a, b) = (from.get_hue(), to.get_hue());
    let difference = 180.0 - ((a - b).abs() - 180.0).abs();
    let rotation = (difference * 0.8).min(100.0);
    let direction = if sanitize(b - a) <= 180.0 { 1.0 } else { -1.0 };
    Hct::from(sanitize(a + rotation * direction), from.get_chroma(), from.get_tone() * (1.0 + tone_boost))
}

/// An insertion-ordered name -> colour list, like the Python dict.
struct Named(Vec<(String, Hct)>);

impl Named {
    fn get(&self, name: &str) -> Hct {
        self.0.iter().find(|(n, _)| n == name).expect("known colour").1
    }

    fn set(&mut self, name: &str, c: Hct) {
        match self.0.iter_mut().find(|(n, _)| n == name) {
            Some(e) => e.1 = c,
            None => self.0.push((name.to_owned(), c)),
        }
    }

    fn update(&mut self, name: &str, f: impl FnOnce(Hct) -> Hct) {
        let c = f(self.get(name));
        self.set(name, c);
    }
}

/// `generator.gen_scheme`.
fn scheme(primary: Hct, variant: &str, flavour: &str, light: bool) -> Colours {
    let mono = variant == "monochrome";
    let v = match variant {
        "content" => Variant::Content,
        "expressive" => Variant::Expressive,
        "fidelity" => Variant::Fidelity,
        "fruitsalad" => Variant::FruitSalad,
        "monochrome" => Variant::Monochrome,
        "neutral" => Variant::Neutral,
        "rainbow" => Variant::Rainbow,
        "tonalspot" => Variant::TonalSpot,
        _ => Variant::Vibrant,
    };
    // As in Python: spec 2025, which only Expressive, Vibrant, TonalSpot and Neutral use; the rest
    // fall back to 2021
    let s = DynamicScheme::from_spec(primary, v, !light, Some(0.0), Platform::Phone, SpecVersion::Spec2025);
    let mut c = Named(MATERIAL.iter().map(|(n, role)| ((*n).to_owned(), role.color().get_hct(&s))).collect());
    for (old, new) in ALIASES {
        c.set(old, c.get(new));
    }

    let key = c.get("primary_paletteKeyColor");
    let fit = |hct: Hct, boost: f64| if mono { grayscale(hct, light) } else { harmonize(hct, key, boost) };
    for (i, rgb) in (if light { LIGHT_TERM } else { DARK_TERM }).into_iter().enumerate() {
        let boost = if i < 8 { 0.35 } else { 0.2 };
        c.set(&format!("term{i}"), fit(hct(rgb), if light { -boost } else { boost }));
    }
    for (name, rgb) in NAMED.iter().zip(if light { LIGHT_NAMED } else { DARK_NAMED }) {
        c.set(name, fit(hct(rgb), if light { -0.2 } else { 0.05 }));
    }
    let (primary, selection) = (c.get("primary"), c.get("onPrimaryFixedVariant"));
    for (name, rgb) in KDE {
        let gray = |h: Hct| if mono { grayscale(h, light) } else { h };
        c.set(name, gray(harmonize(hct(rgb), primary, 0.1)));
        c.set(&format!("{name}Selection"), gray(harmonize(hct(rgb), selection, 0.1)));
    }

    if variant == "neutral" {
        let less = |mut h: Hct| {
            h.set_chroma(h.get_chroma() - 15.0);
            h
        };
        for i in 0..c.0.len() {
            // An alias shares its object with the new name, so Python takes 15 off that object twice
            match ALIASES.iter().find(|(old, _)| *old == c.0[i].0) {
                Some((_, new)) => {
                    c.update(new, less);
                    c.0[i].1 = c.get(new);
                }
                None => c.0[i].1 = less(c.0[i].1),
            }
        }
    }

    let hard = flavour == "hard";
    let soften = |h: Hct, dark: f64| if light { lighten(h, 0.4) } else { darken(h, dark) };
    if hard {
        let surfaces: Vec<String> =
            c.0.iter().map(|(n, _)| n.clone()).filter(|n| n == "background" || n.starts_with("surface")).collect();
        for name in surfaces {
            c.update(&name, |h| soften(h, 0.8));
        }
        c.update("term0", |h| soften(h, 0.9));
    }

    // Deprecated names
    c.set("text", c.get("onBackground"));
    c.set("subtext1", c.get("onSurfaceVariant"));
    c.set("subtext0", c.get("outline"));
    let (surface, outline) = (c.get("surface"), c.get("outline"));
    for (name, w) in [
        ("overlay2", 0.86),
        ("overlay1", 0.71),
        ("overlay0", 0.57),
        ("surface2", 0.43),
        ("surface1", 0.29),
        ("surface0", 0.14),
    ] {
        c.set(name, mix(surface, outline, w));
    }
    c.set("base", surface);
    c.set("mantle", darken(surface, 0.03));
    c.set("crust", darken(surface, 0.05));
    if hard {
        for name in ["base", "mantle", "crust"] {
            c.update(name, |h| soften(h, 0.9));
        }
        for i in 0..3 {
            c.update(&format!("overlay{i}"), |h| soften(h, 0.8));
            c.update(&format!("surface{i}"), |h| soften(h, 0.8));
        }
    }

    let mut out: Colours = c.0.into_iter().map(|(n, h)| (n, format!("{:06x}", Rgb::from(h).as_u32()).into())).collect();
    let success: [(&str, &str); 4] = if light {
        [
            ("success", "4F6354"),
            ("onSuccess", "FFFFFF"),
            ("successContainer", "D1E8D5"),
            ("onSuccessContainer", "0C1F13"),
        ]
    } else {
        [
            ("success", "B5CCBA"),
            ("onSuccess", "213528"),
            ("successContainer", "374B3E"),
            ("onSuccessContainer", "D1E9D6"),
        ]
    };
    for (n, v) in success {
        out.insert(n.into(), v.into());
    }
    out
}
