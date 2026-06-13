#[cfg(test)]
mod pixel_format_tests {
    use crate::capture::types::PixelFormat;

    #[test]
    fn known_formats_parse_correctly() {
        assert_eq!(PixelFormat::from_raw(0x00000000), PixelFormat::Argb8888);
        assert_eq!(PixelFormat::from_raw(0x00000001), PixelFormat::Xrgb8888);
        assert_eq!(PixelFormat::from_raw(0x34324241), PixelFormat::Abgr8888);
        assert_eq!(PixelFormat::from_raw(0x34324258), PixelFormat::Xbgr8888);
    }

    #[test]
    fn unknown_format_preserved() {
        let raw = 0xDEADBEEFu32;
        assert_eq!(PixelFormat::from_raw(raw), PixelFormat::Unknown(raw));
    }

    #[test]
    fn known_formats_report_four_bytes_per_pixel() {
        let formats: [PixelFormat; 4] = [
            PixelFormat::Argb8888,
            PixelFormat::Xrgb8888,
            PixelFormat::Abgr8888,
            PixelFormat::Xbgr8888,
        ];
        for fmt in formats {
            assert_eq!(fmt.bytes_per_pixel(), Some(4), "{fmt:?} should be 4 bpp");
        }
    }

    #[test]
    fn unknown_format_has_no_bpp() {
        assert_eq!(PixelFormat::Unknown(0xFF).bytes_per_pixel(), None);
    }
}
