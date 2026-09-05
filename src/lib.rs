#![forbid(unsafe_code)]

use zed_extension_api as zed;

struct MoonBitLabExtension;

impl zed::Extension for MoonBitLabExtension {
    fn new() -> Self {
        Self
    }
}

zed::register_extension!(MoonBitLabExtension);
