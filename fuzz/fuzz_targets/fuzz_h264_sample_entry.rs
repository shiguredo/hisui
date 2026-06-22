#![no_main]

use libfuzzer_sys::fuzz_target;

// 最小 PPS NAL (固定、src/video/h264.rs::tests::PPS_NAL と同一バイト列)
const PPS_NAL: &[u8] = &[0x68, 0xce, 0x06, 0xe2];

fuzz_target!(|data: &[u8]| {
    // SPS 先頭バイトを 0x67 (NAL タイプ=7) に強制差し替えて NAL タイプ検査を通過させ、
    // parse_sps 本体のビット読み出しパスを fuzz 対象にする
    if data.is_empty() {
        return;
    }
    let mut sps = data.to_vec();
    sps[0] = 0x67;
    let _ = hisui::video::h264::h264_sample_entry_from_sps_pps_lists(
        vec![sps],
        vec![PPS_NAL.to_vec()],
    );
});
