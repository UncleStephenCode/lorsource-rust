use std::io::Cursor;

/// Decode untrusted image bytes with strict dimension limits applied by the
/// decoder before it allocates the output pixel buffer. The upload handlers
/// still perform their feature-specific minimum/aspect checks afterwards.
pub fn stDecodeWithLimits(
    arrData: &[u8],
    enFormat: image::ImageFormat,
    iMaxWidth: u32,
    iMaxHeight: u32,
    iMaxAllocation: u64,
) -> image::ImageResult<image::DynamicImage> {
    let mut stLimits = image::Limits::default();
    stLimits.max_image_width = Some(iMaxWidth);
    stLimits.max_image_height = Some(iMaxHeight);
    stLimits.max_alloc = Some(iMaxAllocation);

    let mut stReader = image::ImageReader::with_format(Cursor::new(arrData), enFormat);
    stReader.limits(stLimits);
    stReader.decode()
}

#[cfg(test)]
mod tests {
    use super::stDecodeWithLimits;

    #[test]
    fn rejects_dimensions_in_the_decoder() {
        let stImage = image::DynamicImage::ImageRgb8(image::RgbImage::new(301, 1));
        let mut vecPng = Vec::new();
        stImage
            .write_to(
                &mut std::io::Cursor::new(&mut vecPng),
                image::ImageFormat::Png,
            )
            .expect("encode fixture");

        let stError =
            stDecodeWithLimits(&vecPng, image::ImageFormat::Png, 300, 300, 8 * 1024 * 1024)
                .expect_err("oversized dimensions must fail before image processing");
        assert!(matches!(stError, image::ImageError::Limits(_)));
    }
}
