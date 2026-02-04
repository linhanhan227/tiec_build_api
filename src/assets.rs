#[derive(Clone, Copy)]
#[allow(dead_code)]
pub struct EmbeddedAsset {
    pub path: &'static str,
    pub data: &'static [u8],
    pub unix_mode: Option<u32>,
}

include!(concat!(env!("OUT_DIR"), "/assets_manifest.rs"));
