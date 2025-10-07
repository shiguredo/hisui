use shiguredo_mp4::{
    Uint,
    boxes::{Av01Box, Av1cBox, SampleEntry},
};

use crate::{types::EvenUsize, video};

pub fn av1_sample_entry(width: EvenUsize, height: EvenUsize, config_obus: &[u8]) -> SampleEntry {
    SampleEntry::Av01(Av01Box {
        visual: video::sample_entry_visual_fields(width.get(), height.get()),
        av1c_box: Av1cBox {
            seq_profile: Uint::new(0),            // Main profile
            seq_level_idx_0: Uint::new(0),        // Default level (unrestricted)
            seq_tier_0: Uint::new(0),             // Main tier
            high_bitdepth: Uint::new(0),          // false
            twelve_bit: Uint::new(0),             // false
            monochrome: Uint::new(0),             // false
            chroma_subsampling_x: Uint::new(1),   // 4:2:0 subsampling
            chroma_subsampling_y: Uint::new(1),   // 4:2:0 subsampling
            chroma_sample_position: Uint::new(0), // Colocated with luma (0, 0)
            initial_presentation_delay_minus_one: None,
            config_obus: config_obus.to_vec(),
        },
        unknown_boxes: Vec::new(),
    })
}
