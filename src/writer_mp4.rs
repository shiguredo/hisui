use std::{
    fs::File,
    io::{BufWriter, Seek, SeekFrom, Write},
    num::NonZeroU32,
    path::Path,
    time::{Duration, SystemTime},
};

use orfail::OrFail;
use shiguredo_mp4::{
    BaseBox, BoxSize, BoxType, Either, Encode, FixedPointNumber, Mp4FileTime, Utf8String,
    boxes::{
        Brand, Co64Box, DinfBox, FreeBox, FtypBox, HdlrBox, MdatBox, MdhdBox, MdiaBox, MinfBox,
        MoovBox, MvhdBox, SampleEntry, SmhdBox, StblBox, StcoBox, StscBox, StscEntry, StsdBox,
        StssBox, StszBox, SttsBox, TkhdBox, TrakBox, UnknownBox, VmhdBox,
    },
};

use crate::{
    audio::{AudioData, AudioDataReceiver},
    layout::{Layout, Resolution},
    mixer_audio::MIXED_AUDIO_DATA_DURATION,
    stats::{Mp4WriterStats, Seconds},
    video::{VideoFrame, VideoFrameReceiver},
};

// Hisui では出力 MP4 のタイムスケールはマイクロ秒固定にする
const TIMESCALE: NonZeroU32 = NonZeroU32::MIN.saturating_add(1_000_000 - 1);

// 映像・音声混在時のチャンクの尺の最大値（映像か音声の片方だけの場合はチャンクは一つだけ）
const MAX_CHUNK_DURATION: Duration = Duration::from_secs(10);

/// 合成結果を含んだ MP4 ファイルを書き出すための構造体
#[derive(Debug)]
pub struct Mp4Writer {
    file: BufWriter<File>,
    file_size: u64,
    resolution: Resolution,
    moov_box_offset: u64,
    mdat_box_offset: u64,
    audio_chunks: Vec<Chunk>,
    video_chunks: Vec<Chunk>,
    audio_sample_entry: Option<SampleEntry>,
    video_sample_entry: Option<SampleEntry>,
    input_audio_rx: Option<AudioDataReceiver>,
    input_video_rx: Option<VideoFrameReceiver>,
    finalize_time: Mp4FileTime,
    appending_video_chunk: bool,
    stats: Mp4WriterStats,
}

impl Mp4Writer {
    /// [`Mp4Writer`] インスタンスを生成する
    pub fn new<P: AsRef<Path>>(
        path: P,
        layout: &Layout,
        input_audio_rx: AudioDataReceiver,
        input_video_rx: VideoFrameReceiver,
    ) -> orfail::Result<Self> {
        let file = std::fs::OpenOptions::new()
            .create(true)
            .truncate(true)
            .write(true)
            .open(path)
            .or_fail()?;
        let mut this = Self {
            file: BufWriter::new(file),
            file_size: 0,
            resolution: layout.resolution,
            moov_box_offset: 0,
            mdat_box_offset: 0,
            audio_chunks: Vec::new(),
            video_chunks: Vec::new(),
            audio_sample_entry: None,
            video_sample_entry: None,
            finalize_time: Mp4FileTime::from_unix_time(Duration::ZERO),
            input_audio_rx: layout.has_audio().then_some(input_audio_rx),
            input_video_rx: layout.has_video().then_some(input_video_rx),
            appending_video_chunk: true,
            stats: Mp4WriterStats::default(),
        };

        let (result, elapsed) = Seconds::elapsed(|| this.init(layout).or_fail());
        result?;
        this.stats.total_processing_seconds.set(elapsed);

        Ok(this)
    }

    /// 統計情報を返す
    pub fn stats(&self) -> &Mp4WriterStats {
        &self.stats
    }

    /// 新しい入力（合成後の映像と音声）を待機して、それの出力ファイルへの書き込みを行う
    ///
    /// 結果は現在の書き込み位置を示すタイムスタンプで、全ての書き込みが完了した場合には `Ok(None)` が返される。
    pub fn poll(&mut self) -> orfail::Result<Option<Duration>> {
        let (audio, video) = self.peek_input_audio_and_video();
        let audio_timestamp = audio.map(|x| x.timestamp);
        let video_timestamp = video.map(|x| x.timestamp);

        let (result, elapsed) = Seconds::elapsed(|| {
            self.handle_next_audio_and_video(audio_timestamp, video_timestamp)
                .or_fail()
        });
        self.stats.total_processing_seconds.add(elapsed);

        result
    }

    fn handle_next_audio_and_video(
        &mut self,
        audio_timestamp: Option<Duration>,
        video_timestamp: Option<Duration>,
    ) -> orfail::Result<Option<Duration>> {
        match (audio_timestamp,video_timestamp){
              (None, None) => {
                // 全部の入力の処理が完了した
                self.finalize().or_fail()?;
                return Ok(None);
            }
            (None, Some(_)) => {
                // 残りは映像のみ
                let new_chunk = self.video_chunks.len() == self.audio_chunks.len();
                self.append_video_frame(new_chunk).or_fail()?;
            }
            (Some(_), None) => {
                // 残りは音声のみ
                let new_chunk = self.audio_chunks.is_empty()
                    || self.video_chunks.len() > self.audio_chunks.len();
                self.append_audio_data(new_chunk).or_fail()?;
            }
            (Some(audio_timestamp), Some(video_timestamp))
                if
                // 音声が一定以上遅れている場合は映像に追従する
                (self.appending_video_chunk && video_timestamp.saturating_sub(audio_timestamp) > MAX_CHUNK_DURATION)
                ||
                // 一度音声追記モードに入った場合には、映像に追いつくまでは音声を追記し続ける
                (!self.appending_video_chunk && video_timestamp > audio_timestamp) =>
            {
                let new_chunk = self.video_chunks.len() > self.audio_chunks.len();
                self.append_audio_data(new_chunk).or_fail()?;
            }
            (Some(_), Some(_)) => {
                // 音声との差が一定以内の場合は、映像の処理を進める
                let new_chunk = self.video_chunks.len() == self.audio_chunks.len();
                self.append_video_frame(new_chunk).or_fail()?;
            }
        }

        // 進捗（現在のタイムスタンプ）を呼び出し元に返す
        Ok(Some(self.current_duration()))
    }

    fn current_duration(&self) -> Duration {
        self.stats
            .total_audio_track_seconds
            .get_duration()
            .max(self.stats.total_video_track_seconds.get_duration())
    }

    fn peek_input_audio_and_video(&mut self) -> (Option<&AudioData>, Option<&VideoFrame>) {
        let audio = self.input_audio_rx.as_mut().and_then(|rx| rx.peek());
        let video = self.input_video_rx.as_mut().and_then(|rx| rx.peek());
        (audio, video)
    }

    fn append_video_frame(&mut self, new_chunk: bool) -> orfail::Result<()> {
        // 次の入力を取り出す（これは常に成功する）
        let frame = self
            .input_video_rx
            .as_mut()
            .and_then(|rx| rx.recv())
            .or_fail()?;

        if self.stats.video_codec.get().is_none()
            && let Some(name) = frame.format.codec_name()
        {
            self.stats.video_codec.set(name);
        }

        // Hisui では途中でエンコード情報が変わることがないので、
        // サンプルエントリーは最初に一回だけ存在する
        if self.video_sample_entry.is_none() {
            frame.sample_entry.is_some().or_fail()?;
            self.video_sample_entry = frame.sample_entry;
        } else {
            frame.sample_entry.is_none().or_fail()?;
        }

        // 必要に応じて新しいチャンクを始める
        if new_chunk {
            self.video_chunks.push(Chunk {
                offset: self.file_size,
                samples: Vec::new(),
            });
            self.stats.total_video_chunk_count.add(1);
        }

        // 一番最後に moov ボックスを構築するためのメタデータを覚えておく
        let sample = Sample {
            keyframe: frame.keyframe,
            size: frame.data.len() as u32,
            duration: frame.duration.as_micros() as u32,
        };
        self.video_chunks.last_mut().or_fail()?.samples.push(sample);
        self.stats.total_video_sample_count.add(1);

        // mdat ボックスにデータを追記する
        self.file.write_all(&frame.data).or_fail()?;
        self.file_size += frame.data.len() as u64;
        self.stats
            .total_video_sample_data_byte_size
            .add(frame.data.len() as u64);

        self.stats
            .total_video_track_seconds
            .add(Seconds::new(frame.duration));
        self.appending_video_chunk = true;
        Ok(())
    }

    fn append_audio_data(&mut self, new_chunk: bool) -> orfail::Result<()> {
        // 次の入力を取り出す（これは常に成功する）
        let data = self
            .input_audio_rx
            .as_mut()
            .and_then(|rx| rx.recv())
            .or_fail()?;

        if self.stats.audio_codec.get().is_none()
            && let Some(name) = data.format.codec_name()
        {
            self.stats.audio_codec.set(name);
        }

        // Hisui では途中でエンコード情報が変わることがないので、
        // サンプルエントリーは最初に一回だけ存在する
        if self.audio_sample_entry.is_none() {
            data.sample_entry.is_some().or_fail()?;
            self.audio_sample_entry = data.sample_entry;
        } else {
            data.sample_entry.is_none().or_fail()?;
        }

        // 必要に応じて新しいチャンクを始める
        if new_chunk {
            self.audio_chunks.push(Chunk {
                offset: self.file_size,
                samples: Vec::new(),
            });
            self.stats.total_audio_chunk_count.add(1);
        }

        // 一番最後に moov ボックスを構築するためのメタデータを覚えておく
        let sample = Sample {
            keyframe: true,
            size: data.data.len() as u32,
            duration: data.duration.as_micros() as u32,
        };
        self.audio_chunks.last_mut().or_fail()?.samples.push(sample);
        self.stats.total_audio_sample_count.add(1);

        // mdat ボックスにデータを追記する
        self.file.write_all(&data.data).or_fail()?;
        self.file_size += data.data.len() as u64;
        self.stats
            .total_audio_sample_data_byte_size
            .add(data.data.len() as u64);

        self.stats
            .total_audio_track_seconds
            .add(Seconds::new(data.duration));
        self.appending_video_chunk = false;
        Ok(())
    }

    fn finalize(&mut self) -> orfail::Result<()> {
        self.finalize_time =
            Mp4FileTime::from_unix_time(SystemTime::UNIX_EPOCH.elapsed().or_fail()?);

        // 確定した moov ボックスの内容で事前に確保しておいた free ボックスの
        // 領域を上書きする
        let moov_box = self.build_moov_box().or_fail()?;

        let moov_box_size = moov_box.box_size().get();
        let free_box_min_size = 8;
        let reserved_size = self.mdat_box_offset - self.moov_box_offset;
        self.stats
            .actual_moov_box_size
            .set(moov_box_size + free_box_min_size);
        (moov_box_size + free_box_min_size < reserved_size).or_fail()?;

        self.file
            .seek(SeekFrom::Start(self.moov_box_offset))
            .or_fail()?;
        moov_box.encode(&mut self.file).or_fail()?;

        let free_box_payload_size =
            self.mdat_box_offset - (self.moov_box_offset + moov_box_size) - 8;
        let free_box = FreeBox {
            payload: vec![0; free_box_payload_size as usize],
        };
        free_box.encode(&mut self.file).or_fail()?;

        // [NOTE]
        // 特に支障はないはずなので mdat ボックスは可変長サイズ扱いのままにしておく
        // (もし問題があるようなら、ここで確定したサイズに上書きする)

        self.file.flush().or_fail()?;
        Ok(())
    }

    fn build_moov_box(&self) -> orfail::Result<MoovBox> {
        let mut trak_boxes = Vec::new();
        if !self.audio_chunks.is_empty() {
            let track_id = trak_boxes.len() as u32 + 1;
            trak_boxes.push(self.build_audio_trak_box(track_id).or_fail()?);
        }
        if !self.video_chunks.is_empty() {
            let track_id = trak_boxes.len() as u32 + 1;
            trak_boxes.push(self.build_video_trak_box(track_id).or_fail()?);
        }

        let mvhd_box = MvhdBox {
            creation_time: self.finalize_time,
            modification_time: self.finalize_time,
            timescale: TIMESCALE,
            duration: self.current_duration().as_micros() as u64,
            rate: MvhdBox::DEFAULT_RATE,
            volume: MvhdBox::DEFAULT_VOLUME,
            matrix: MvhdBox::DEFAULT_MATRIX,
            next_track_id: trak_boxes.len() as u32 + 1,
        };

        Ok(MoovBox {
            mvhd_box,
            trak_boxes,
            unknown_boxes: Vec::new(),
        })
    }

    fn build_audio_trak_box(&self, track_id: u32) -> orfail::Result<TrakBox> {
        let tkhd_box = TkhdBox {
            flag_track_enabled: true,
            flag_track_in_movie: true,
            flag_track_in_preview: false,
            flag_track_size_is_aspect_ratio: false,
            creation_time: self.finalize_time,
            modification_time: self.finalize_time,
            track_id,
            duration: self
                .stats
                .total_audio_track_seconds
                .get_duration()
                .as_micros() as u64,
            layer: TkhdBox::DEFAULT_LAYER,
            alternate_group: TkhdBox::DEFAULT_ALTERNATE_GROUP,
            volume: TkhdBox::DEFAULT_AUDIO_VOLUME,
            matrix: TkhdBox::DEFAULT_MATRIX,
            width: FixedPointNumber::default(),
            height: FixedPointNumber::default(),
        };
        Ok(TrakBox {
            tkhd_box,
            edts_box: None,
            mdia_box: self.build_audio_mdia_box().or_fail()?,
            unknown_boxes: Vec::new(),
        })
    }

    fn build_video_trak_box(&self, track_id: u32) -> orfail::Result<TrakBox> {
        let tkhd_box = TkhdBox {
            flag_track_enabled: true,
            flag_track_in_movie: true,
            flag_track_in_preview: false,
            flag_track_size_is_aspect_ratio: false,
            creation_time: self.finalize_time,
            modification_time: self.finalize_time,
            track_id,
            duration: self
                .stats
                .total_video_track_seconds
                .get_duration()
                .as_micros() as u64,
            layer: TkhdBox::DEFAULT_LAYER,
            alternate_group: TkhdBox::DEFAULT_ALTERNATE_GROUP,
            volume: TkhdBox::DEFAULT_VIDEO_VOLUME,
            matrix: TkhdBox::DEFAULT_MATRIX,
            width: FixedPointNumber::new(self.resolution.width().get() as i16, 0),
            height: FixedPointNumber::new(self.resolution.height().get() as i16, 0),
        };
        Ok(TrakBox {
            tkhd_box,
            edts_box: None,
            mdia_box: self.build_video_mdia_box().or_fail()?,
            unknown_boxes: Vec::new(),
        })
    }

    fn build_audio_mdia_box(&self) -> orfail::Result<MdiaBox> {
        let sample_entry = self.audio_sample_entry.as_ref().or_fail()?;
        let mdhd_box = MdhdBox {
            creation_time: self.finalize_time,
            modification_time: self.finalize_time,
            timescale: TIMESCALE,
            duration: self
                .stats
                .total_audio_track_seconds
                .get_duration()
                .as_micros() as u64,
            language: MdhdBox::LANGUAGE_UNDEFINED,
        };
        let hdlr_box = HdlrBox {
            handler_type: HdlrBox::HANDLER_TYPE_SOUN,
            name: Utf8String::EMPTY.into_null_terminated_bytes(),
        };
        let minf_box = MinfBox {
            smhd_or_vmhd_box: Either::A(SmhdBox::default()),
            dinf_box: DinfBox::LOCAL_FILE,
            stbl_box: self.build_stbl_box(sample_entry, &self.audio_chunks),
            unknown_boxes: Vec::new(),
        };
        Ok(MdiaBox {
            mdhd_box,
            hdlr_box,
            minf_box,
            unknown_boxes: Vec::new(),
        })
    }

    fn build_video_mdia_box(&self) -> orfail::Result<MdiaBox> {
        let sample_entry = self.video_sample_entry.as_ref().or_fail()?;
        let mdhd_box = MdhdBox {
            creation_time: self.finalize_time,
            modification_time: self.finalize_time,
            timescale: TIMESCALE,
            duration: self
                .stats
                .total_video_track_seconds
                .get_duration()
                .as_micros() as u64,
            language: MdhdBox::LANGUAGE_UNDEFINED,
        };
        let hdlr_box = HdlrBox {
            handler_type: HdlrBox::HANDLER_TYPE_VIDE,
            name: Utf8String::EMPTY.into_null_terminated_bytes(),
        };
        let minf_box = MinfBox {
            smhd_or_vmhd_box: Either::B(VmhdBox::default()),
            dinf_box: DinfBox::LOCAL_FILE,
            stbl_box: self.build_stbl_box(sample_entry, &self.video_chunks),
            unknown_boxes: Vec::new(),
        };
        Ok(MdiaBox {
            mdhd_box,
            hdlr_box,
            minf_box,
            unknown_boxes: Vec::new(),
        })
    }

    fn build_stbl_box(&self, sample_entry: &SampleEntry, chunks: &[Chunk]) -> StblBox {
        let stsd_box = StsdBox {
            entries: vec![sample_entry.clone()],
        };

        let stts_box = SttsBox::from_sample_deltas(
            chunks
                .iter()
                .flat_map(|c| c.samples.iter().map(|s| s.duration)),
        );

        let stsc_box = StscBox {
            entries: chunks
                .iter()
                .enumerate()
                .map(|(i, c)| StscEntry {
                    first_chunk: NonZeroU32::MIN.saturating_add(i as u32),
                    sample_per_chunk: c.samples.len() as u32,
                    sample_description_index: NonZeroU32::MIN,
                })
                .collect(),
        };

        let stsz_box = StszBox::Variable {
            entry_sizes: chunks
                .iter()
                .flat_map(|s| s.samples.iter().map(|s| s.size))
                .collect(),
        };

        let stco_or_co64_box = if self.file_size > u32::MAX as u64 {
            Either::B(Co64Box {
                chunk_offsets: chunks.iter().map(|c| c.offset).collect(),
            })
        } else {
            Either::A(StcoBox {
                chunk_offsets: chunks.iter().map(|c| c.offset as u32).collect(),
            })
        };

        let is_all_keyframe = chunks.iter().all(|c| c.samples.iter().all(|s| s.keyframe));
        let stss_box = if is_all_keyframe {
            None
        } else {
            Some(StssBox {
                sample_numbers: chunks
                    .iter()
                    .flat_map(|c| c.samples.iter())
                    .enumerate()
                    .filter_map(|(i, s)| {
                        s.keyframe
                            .then_some(NonZeroU32::MIN.saturating_add(i as u32))
                    })
                    .collect(),
            })
        };

        StblBox {
            stsd_box,
            stts_box,
            stsc_box,
            stsz_box,
            stco_or_co64_box,
            stss_box,
            unknown_boxes: Vec::new(),
        }
    }

    // 実際にメディアデータを書き込む前の MP4 ファイルの初期化処理
    fn init(&mut self, layout: &Layout) -> orfail::Result<()> {
        // ftyp ボックスを書きこむ
        self.write_ftyp_box().or_fail()?;

        // 最終的な moov ボックスを保持可能なサイズの free ボックスを書きこむ
        // (先頭付近に moov ボックスを配置することで、動画プレイヤーの再生開始までに掛かる時間を短縮できる)
        self.write_free_box(layout).or_fail()?;

        // 可変長の mdat ボックスのヘッダーを書きこむ
        self.mdat_box_offset = self.file_size;

        let mdat_box = MdatBox {
            is_variable_size: true,
            payload: Vec::new(),
        };
        mdat_box.encode(&mut self.file).or_fail()?;

        // [NOTE] 可変サイズの場合は `mdat_box.box_size()` は使えないので、固定値を加算する
        self.file_size += 8;

        Ok(())
    }

    fn write_ftyp_box(&mut self) -> orfail::Result<()> {
        // Hisui で扱う可能性があるコーデックを全て含んだ互換性ブランドを指定しておく。
        // （もし必要最小限に絞りたくなったら、実際にファイルに含まれるコーデックから動的に生成するようにする）
        let compatible_brands = vec![
            Brand::ISOM,
            Brand::ISO2,
            Brand::MP41,
            Brand::AVC1,
            Brand::AV01,
        ];

        let ftyp_box = FtypBox {
            major_brand: Brand::ISOM,
            minor_version: 0,
            compatible_brands,
        };
        ftyp_box.encode(&mut self.file).or_fail()?;
        self.file_size += ftyp_box.box_size().get();

        Ok(())
    }

    fn write_free_box(&mut self, layout: &Layout) -> orfail::Result<()> {
        self.moov_box_offset = self.file_size;

        // faststart 用にダミーの moov を事前に構築する (必要なサイズの計測用)
        // かなり余裕をみた計算方法になっているので、これで足りないことはまずないはず
        let moov_box = self.build_dummy_moov_box(layout);
        let max_moov_box_size = moov_box.box_size().get();
        self.stats.reserved_moov_box_size.set(max_moov_box_size);
        log::debug!("reserved moov box size: {max_moov_box_size}");

        // 初期化時点では free ボックスで領域だけ確保しておく
        let free_box = FreeBox {
            payload: vec![0; max_moov_box_size as usize],
        };
        free_box.encode(&mut self.file).or_fail()?;
        self.file_size += free_box.box_size().get();
        Ok(())
    }

    fn build_dummy_moov_box(&self, layout: &Layout) -> MoovBox {
        let mvhd_box = MvhdBox {
            // フィールドの値はなんでもいいのでテキトウに設定しておく
            creation_time: Mp4FileTime::default(),
            modification_time: Mp4FileTime::default(),
            timescale: NonZeroU32::MIN,
            duration: u64::MAX, // ここが 32 bit に収まるかどうかでサイズが変わるので大きい値を指定する
            rate: MvhdBox::DEFAULT_RATE,
            volume: MvhdBox::DEFAULT_VOLUME,
            matrix: MvhdBox::DEFAULT_MATRIX,
            next_track_id: 1,
        };

        let duration = layout.duration();
        let mut trak_boxes = Vec::new();
        if layout.has_audio() {
            let audio_sample_count =
                (duration.as_micros() / MIXED_AUDIO_DATA_DURATION.as_micros()) as usize;
            trak_boxes.push(self.build_dummy_trak_box(audio_sample_count));
        }
        if layout.has_video() {
            let video_sample_count = duration.as_secs() as usize
                * layout.frame_rate.numerator.get()
                / layout.frame_rate.denumerator.get();
            trak_boxes.push(self.build_dummy_trak_box(video_sample_count));
        }

        MoovBox {
            mvhd_box,
            trak_boxes,
            unknown_boxes: Vec::new(),
        }
    }

    fn build_dummy_trak_box(&self, sample_count: usize) -> TrakBox {
        let tkhd_box = TkhdBox {
            // フィールドの値はなんでもいいのでテキトウに設定しておく
            flag_track_enabled: true,
            flag_track_in_movie: true,
            flag_track_in_preview: false,
            flag_track_size_is_aspect_ratio: false,
            creation_time: Mp4FileTime::default(),
            modification_time: Mp4FileTime::default(),
            track_id: 1,
            duration: u64::MAX, // ここは 32 bit に収まるかどうかでサイズが変わる
            layer: TkhdBox::DEFAULT_LAYER,
            alternate_group: TkhdBox::DEFAULT_ALTERNATE_GROUP,
            volume: TkhdBox::DEFAULT_AUDIO_VOLUME,
            matrix: TkhdBox::DEFAULT_MATRIX,
            width: FixedPointNumber::default(),
            height: FixedPointNumber::default(),
        };
        TrakBox {
            tkhd_box,
            edts_box: None,
            mdia_box: self.build_dummy_mdia_box(sample_count),
            unknown_boxes: Vec::new(),
        }
    }

    fn build_dummy_mdia_box(&self, sample_count: usize) -> MdiaBox {
        let mdhd_box = MdhdBox {
            // フィールドの値はなんでもいいのでテキトウに設定しておく
            creation_time: Mp4FileTime::default(),
            modification_time: Mp4FileTime::default(),
            timescale: NonZeroU32::MIN,
            duration: u64::MAX, // ここは 32 bit に収まるかどうかでサイズが変わる
            language: MdhdBox::LANGUAGE_UNDEFINED,
        };
        let hdlr_box = HdlrBox {
            // 同上（テキトウな固定値でいい）
            handler_type: HdlrBox::HANDLER_TYPE_VIDE,
            name: Utf8String::EMPTY.into_null_terminated_bytes(),
        };
        let minf_box = MinfBox {
            // 同上（テキトウな固定値でいい）
            smhd_or_vmhd_box: Either::B(VmhdBox::default()),
            dinf_box: DinfBox::LOCAL_FILE,
            stbl_box: self.build_dummy_stbl_box(sample_count),
            unknown_boxes: Vec::new(),
        };
        MdiaBox {
            mdhd_box,
            hdlr_box,
            minf_box,
            unknown_boxes: Vec::new(),
        }
    }

    fn build_dummy_stbl_box(&self, sample_count: usize) -> StblBox {
        // Hisui では途中でエンコード情報が変わることはないので
        // サンプルエントリーは常に 1 つとなる
        let sample_entries = vec![SampleEntry::Unknown(UnknownBox {
            box_type: BoxType::Normal(*b"dumy"),
            box_size: BoxSize::U64(u64::MAX),

            // 多めを確保しておく (サンプルエントリーの中身が 4KB を超えることはまずない）
            payload: vec![0; 4096],
        })];
        let stsd_box = StsdBox {
            entries: sample_entries.clone(),
        };

        // 最悪ケースを想定して、全部のサンプルの尺が異なる、という扱いにしておく
        let stts_box = SttsBox::from_sample_deltas(0..sample_count as u32);

        // 最悪ケースを想定して、1つのチャンクに1つのサンプルしかない、という扱いにしておく
        let stsc_box = StscBox {
            entries: (0..sample_count as u32)
                .map(|i| StscEntry {
                    first_chunk: NonZeroU32::MIN.saturating_add(i),
                    sample_per_chunk: 1, // チャンク内のサンプル数は 1 固定
                    sample_description_index: NonZeroU32::MIN,
                })
                .collect(),
        };

        // 最悪ケースを想定して、全部のサンプルのサイズが異なる、という扱いにしておく
        let stsz_box = StszBox::Variable {
            entry_sizes: (0..sample_count as u32).collect(),
        };

        // 最悪ケースを想定して、MP4 ファイルのサイズが 4GB を越える、という扱いにしておく
        let co64_box = Co64Box {
            chunk_offsets: (0..sample_count as u64).collect(),
        };

        // 最悪ケースを想定して、全てが同期サンプル(キーフレーム)、という扱いにしておく
        //
        // なお、本来なら、このケースはボックスそのものが不要だが、ここでは、
        // 最大サイズ推定用にあえてボックスを残している
        let stss_box = StssBox {
            sample_numbers: (0..sample_count as u32)
                .map(|i| NonZeroU32::MIN.saturating_add(i))
                .collect(),
        };

        StblBox {
            stsd_box,
            stts_box,
            stsc_box,
            stsz_box,
            stco_or_co64_box: Either::B(co64_box),
            stss_box: Some(stss_box),
            unknown_boxes: Vec::new(),
        }
    }
}

#[derive(Debug)]
struct Chunk {
    offset: u64,
    samples: Vec<Sample>,
}

#[derive(Debug)]
struct Sample {
    keyframe: bool,
    size: u32,
    duration: u32,
}
