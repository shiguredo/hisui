/// イベントの subscription flag 付きテキスト
#[derive(Clone)]
pub struct TaggedEvent {
    pub text: nojson::RawJsonOwned,
    pub subscription_flag: u32,
}
