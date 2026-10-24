use anyhow::{Result, ensure};
use base64::{Engine, engine::general_purpose::STANDARD};
use image::{
    DynamicImage, GenericImageView, ImageEncoder,
    codecs::png::{CompressionType, FilterType, PngEncoder},
    imageops,
};

pub fn prepare(image: DynamicImage, max_dimension: u32) -> Result<DynamicImage> {
    let gray = image.thumbnail(256, 256).to_luma8();
    let (mut low, mut high) = (u8::MAX, u8::MIN);
    for pixel in gray.pixels() {
        low = low.min(pixel[0]);
        high = high.max(pixel[0]);
    }
    ensure!(
        high.saturating_sub(low) >= 6,
        "框选区域几乎没有文字，请重新框选"
    );
    if image.width().max(image.height()) > max_dimension {
        let resized = image
            .resize(max_dimension, max_dimension, imageops::FilterType::Lanczos3)
            .to_rgb8();
        Ok(DynamicImage::ImageRgb8(imageops::unsharpen(
            &resized, 1.0, 2,
        )))
    } else {
        Ok(image)
    }
}

pub fn png(image: &DynamicImage) -> Result<Vec<u8>> {
    let rgb = image.to_rgb8();
    let mut bytes = Vec::new();
    PngEncoder::new_with_quality(&mut bytes, CompressionType::Fast, FilterType::Adaptive)
        .write_image(
            &rgb,
            image.width(),
            image.height(),
            image::ExtendedColorType::Rgb8,
        )?;
    Ok(bytes)
}

pub fn data_url(image: &DynamicImage) -> Result<String> {
    Ok(format!(
        "data:image/png;base64,{}",
        STANDARD.encode(png(image)?)
    ))
}

pub fn band_bounds(image: &DynamicImage) -> Vec<(u32, u32)> {
    let gray = image.to_luma8();
    let (width, height) = image.dimensions();
    let radius = 48.min(height / 12);
    let mut cuts = Vec::new();
    for ideal in [height / 3, height * 2 / 3] {
        let ink = |y: u32| {
            (0..width)
                .filter(|x| gray.get_pixel(*x, y)[0] < 192)
                .count()
        };
        let safe = (ideal - radius..=ideal + radius)
            .filter(|y| {
                [y - 1, *y, y + 1]
                    .into_iter()
                    .all(|row| ink(row) <= (width / 500).max(1) as usize)
            })
            .min_by_key(|y| y.abs_diff(ideal));
        cuts.push(safe.map(|y| (y, 0)).unwrap_or((ideal, 16.min(height / 30))));
    }
    let [(a, pa), (b, pb)] = cuts[..] else {
        unreachable!()
    };
    vec![(0, a + pa), (a - pa, b + pb), (b - pb, height)]
}
