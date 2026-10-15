use std::{env, fs, path::PathBuf};

use resvg::{tiny_skia, usvg};

fn render(svg: &str, size: u32) -> tiny_skia::Pixmap {
    let tree = usvg::Tree::from_str(svg, &usvg::Options::default()).expect("valid branding SVG");
    let mut pixels = tiny_skia::Pixmap::new(size, size).expect("icon pixmap");
    resvg::render(
        &tree,
        tiny_skia::Transform::from_scale(
            size as f32 / tree.size().width(),
            size as f32 / tree.size().height(),
        ),
        &mut pixels.as_mut(),
    );
    pixels
}

fn main() {
    let output = PathBuf::from(env::var_os("OUT_DIR").expect("build output directory"));
    let logos =
        PathBuf::from(env::var_os("CARGO_MANIFEST_DIR").unwrap()).join("../../assets/logos");
    let light = fs::read_to_string(logos.join("moiren-2c-mark-light.svg")).unwrap();
    let dark = fs::read_to_string(logos.join("moiren-2c-mark-dark.svg")).unwrap();
    for name in ["moiren-2c-mark-light.svg", "moiren-2c-mark-dark.svg"] {
        println!("cargo:rerun-if-changed={}", logos.join(name).display());
    }
    let app = light.replacen(
        '>',
        "><rect x=\"8\" y=\"8\" width=\"240\" height=\"240\" rx=\"48\" fill=\"#F5F3EE\"/>",
        1,
    );
    let sizes = [16u32, 20, 24, 32, 40, 48, 64, 128, 256];
    let mut icon_images = Vec::new();
    for size in sizes {
        for (name, source) in [("tray-light", &light), ("tray-dark", &dark), ("app", &app)] {
            let pixels = render(source, size);
            let rgba: Vec<u8> = pixels
                .pixels()
                .iter()
                .flat_map(|pixel| {
                    let straight = pixel.demultiply();
                    [
                        straight.red(),
                        straight.green(),
                        straight.blue(),
                        straight.alpha(),
                    ]
                })
                .collect();
            fs::write(output.join(format!("{name}-{size}.rgba")), rgba).unwrap();
            let png = pixels.encode_png().unwrap();
            fs::write(output.join(format!("{name}-{size}.png")), &png).unwrap();
            if name == "app" {
                icon_images.push((size, png));
            }
        }
    }
    let mut ico = Vec::new();
    ico.extend_from_slice(&[0, 0, 1, 0]);
    ico.extend_from_slice(&(icon_images.len() as u16).to_le_bytes());
    let mut offset = 6 + 16 * icon_images.len() as u32;
    for (size, png) in &icon_images {
        ico.extend_from_slice(&[*size as u8, *size as u8, 0, 0]);
        ico.extend_from_slice(&1u16.to_le_bytes());
        ico.extend_from_slice(&32u16.to_le_bytes());
        ico.extend_from_slice(&(png.len() as u32).to_le_bytes());
        ico.extend_from_slice(&offset.to_le_bytes());
        offset += png.len() as u32;
    }
    for (_, png) in icon_images {
        ico.extend_from_slice(&png);
    }
    let icon_path = output.join("moiren.ico");
    fs::write(&icon_path, ico).unwrap();
    if env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("windows") {
        let resource = output.join("moiren.rc");
        fs::write(
            &resource,
            format!(
                "1 ICON \"{}\"\n",
                icon_path.display().to_string().replace('\\', "/")
            ),
        )
        .unwrap();
        embed_resource::compile_for_everything(&resource, embed_resource::NONE)
            .manifest_optional()
            .unwrap();
    }
}
