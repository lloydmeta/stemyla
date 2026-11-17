//! `xtask brand`: the committed icons and share image match what it draws,
//! and the head tags of `app/index.html` agree with each other and with
//! the files.

use xtask::brand::{
    self, ASSETS, FAVICON_ICO, FAVICON_SIZE, ICON_SVG, Image, MIX, OG_HEIGHT, OG_PNG, OG_WIDTH,
    STEMS, TILE, TOUCH_ICON, TOUCH_ICON_SIZE, decode_png, encode_png, head_problems, ico, ico_png,
    same_image,
};
use xtask::workspace_root;

fn index_html() -> String {
    std::fs::read_to_string(workspace_root().join("app/index.html")).unwrap()
}

#[test]
fn the_committed_brand_files_match_the_generator() {
    let problems = brand::stale(&workspace_root().join(ASSETS)).unwrap();
    assert!(
        problems.is_empty(),
        "{}\nFix: run `cargo run -p xtask -- brand`, look at the files, and commit them.",
        problems.join("\n")
    );
}

#[test]
fn the_drawn_files_have_their_sizes() {
    let files = brand::files().unwrap();
    let names: Vec<&str> = files.iter().map(|f| f.name).collect();
    assert_eq!(names, brand::FILES);
    let png = |name: &str| {
        let bytes = &files.iter().find(|f| f.name == name).unwrap().bytes;
        if name.ends_with(".ico") {
            decode_png(ico_png(bytes).unwrap()).unwrap()
        } else {
            decode_png(bytes).unwrap()
        }
    };
    let size = |i: &Image| (i.width, i.height);
    assert_eq!(size(&png(FAVICON_ICO)), (FAVICON_SIZE, FAVICON_SIZE));
    assert_eq!(size(&png(TOUCH_ICON)), (TOUCH_ICON_SIZE, TOUCH_ICON_SIZE));
    assert_eq!(size(&png(OG_PNG)), (OG_WIDTH, OG_HEIGHT));

    let favicon = png(FAVICON_ICO);
    assert_eq!(favicon.rgba[3], 0, "the favicon's tile has round corners");
    let touch = png(TOUCH_ICON);
    assert_eq!(
        touch.rgba[..4],
        [TILE.0, TILE.1, TILE.2, 255],
        "the iPhone rounds the corners itself, so the touch icon is square and opaque"
    );
    assert!(
        png(OG_PNG).rgba.chunks(4).all(|p| p[3] == 255),
        "the share image is opaque"
    );

    let svg = std::str::from_utf8(&files.iter().find(|f| f.name == ICON_SVG).unwrap().bytes)
        .unwrap()
        .to_string();
    for (stem, colour) in STEMS {
        assert!(
            svg.contains(&format!("fill=\"{}\"", colour.hex())),
            "{stem}'s bar: {svg}"
        );
    }
}

#[test]
fn the_brand_colours_are_the_app_s() {
    let lib = std::fs::read_to_string(workspace_root().join("app/src/lib.rs")).unwrap();
    let body = lib
        .split("pub fn stem_color(")
        .nth(1)
        .and_then(|rest| rest.split("\n}\n").next())
        .expect("app/src/lib.rs has stem_color");
    for (stem, colour) in STEMS {
        let arm = body
            .lines()
            .find(|line| line.contains(&format!("\"{stem}\"")) && line.contains("=>"))
            .unwrap_or_else(|| panic!("stem_color has no arm for {stem}"));
        let expected = format!(
            "Color32::from_rgb({}, {}, {})",
            colour.0, colour.1, colour.2
        );
        assert!(arm.contains(&expected), "{stem}: {arm} is not {expected}");
    }
    let mix = format!("color: Color32::from_rgb({}, {}, {})", MIX.0, MIX.1, MIX.2);
    assert!(
        lib.contains(&format!(
            "name: \"mix\".into(),\n            stem: None,\n            {mix}"
        )),
        "the app's mix lane is not {mix}"
    );
    assert!(
        index_html().contains(&format!("background: {};", TILE.hex())),
        "the page's background is not the tile's {}",
        TILE.hex()
    );
}

#[test]
fn the_head_tags_agree() {
    let og = std::fs::read(workspace_root().join(ASSETS).join(OG_PNG)).unwrap();
    let og = decode_png(&og).unwrap();
    let problems = head_problems(&index_html(), (og.width, og.height));
    assert!(problems.is_empty(), "{}", problems.join("\n"));
}

#[test]
fn head_problems_finds_each_fault() {
    let html = index_html();
    let size = (OG_WIDTH, OG_HEIGHT);
    assert_eq!(head_problems(&html, size), Vec::<String>::new());
    let description = "Split any song into vocals, drums, bass, guitar, piano and the rest, in your browser with WebGPU. Free, private, no install: it all stems from your own GPU.";
    let long = "x".repeat(brand::DESCRIPTION_LIMIT + 1);
    let faults: [(&str, String, &str); 13] = [
        (
            "property=\"og:title\" content=\"Stemyla",
            "property=\"og:title\" content=\"Other".into(),
            "og:title",
        ),
        (
            "property=\"og:description\" content=\"Split",
            "property=\"og:description\" content=\"Unmix".into(),
            "og:description",
        ),
        (description, long, "characters"),
        (
            "<link rel=\"canonical\" href=\"https://",
            "<link rel=\"canonical\" href=\"http://".into(),
            "canonical URL",
        ),
        (
            "property=\"og:url\" content=\"https://beachape.com/stemyla/\"",
            "property=\"og:url\" content=\"https://acme.com/\"".into(),
            "og:url",
        ),
        ("/stemyla/og.png", "/stemyla/v/og.png".into(), "og:image"),
        (
            "property=\"og:image:width\" content=\"1200\"",
            "property=\"og:image:width\" content=\"1201\"".into(),
            "1201 x 630",
        ),
        (
            "property=\"og:image:alt\"",
            "property=\"og:image:none\"".into(),
            "no og:image:alt",
        ),
        (
            "content=\"summary_large_image\"",
            "content=\"summary\"".into(),
            "twitter:card",
        ),
        (
            "name=\"theme-color\" content=\"#1b1b1b\"",
            "name=\"theme-color\" content=\"#ffffff\"".into(),
            "theme-color",
        ),
        (
            "<link data-trunk rel=\"copy-file\" href=\"assets/og.png\"",
            "<link data-trunk rel=\"copy-file\" href=\"og.png\"".into(),
            "assets/og.png",
        ),
        (
            "<link rel=\"apple-touch-icon\" href=\"apple-touch-icon.png\"",
            "<link rel=\"apple-touch-icon\" href=\"touch.png\"".into(),
            "apple-touch-icon",
        ),
        ("<title>", "<titel>".into(), "no <title>"),
    ];
    for (from, to, says) in faults {
        assert!(html.contains(from), "the page has no {from:?}");
        let problems = head_problems(&html.replacen(from, &to, 1), size);
        assert!(
            problems.iter().any(|p| p.contains(says)),
            "{from:?} -> {to:?} should give a problem with {says:?}: {problems:?}"
        );
    }
    let wrong_size = head_problems(&html, (OG_WIDTH, OG_HEIGHT + 1));
    assert!(
        wrong_size.iter().any(|p| p.contains("1200 x 631")),
        "{wrong_size:?}"
    );
}

#[test]
fn an_ico_holds_its_png_and_a_changed_pixel_counts() {
    let image = Image {
        width: 2,
        height: 1,
        rgba: vec![1, 2, 3, 255, 4, 5, 6, 0],
    };
    let png = encode_png(&image).unwrap();
    assert_eq!(decode_png(&png).unwrap(), image);
    let icon = ico(&png, 2).unwrap();
    assert_eq!(ico_png(&icon).unwrap(), png.as_slice());
    assert!(ico(&png, 256).is_err(), "an ICO entry is at most 255 px");
    assert!(ico_png(&icon[..10]).is_err(), "a cut file");
    assert!(ico_png(&icon[..icon.len() - 1]).is_err(), "a cut image");

    let changed = Image {
        rgba: vec![1, 2, 3, 255, 4, 5, 7, 0],
        ..image.clone()
    };
    let changed_png = encode_png(&changed).unwrap();
    assert!(same_image("a.png", &png, &png).unwrap());
    assert!(!same_image("a.png", &png, &changed_png).unwrap());
    assert!(!same_image("a.ico", &icon, &ico(&changed_png, 2).unwrap()).unwrap());
    assert!(!same_image("a.svg", b"<svg/>", b"<svg />").unwrap());
}

#[test]
fn stale_names_a_changed_or_missing_file() {
    let dir = std::env::temp_dir().join(format!("stemyla-brand-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    for file in brand::files().unwrap() {
        std::fs::write(dir.join(file.name), &file.bytes).unwrap();
    }
    assert_eq!(brand::stale(&dir).unwrap(), Vec::<String>::new());
    std::fs::write(dir.join(ICON_SVG), "<svg/>").unwrap();
    std::fs::remove_file(dir.join(OG_PNG)).unwrap();
    let problems = brand::stale(&dir).unwrap();
    assert_eq!(problems.len(), 2, "{problems:?}");
    assert!(problems[0].contains(ICON_SVG) && problems[0].contains("differs"));
    assert!(problems[1].contains(OG_PNG));
    let _ = std::fs::remove_dir_all(&dir);
}
