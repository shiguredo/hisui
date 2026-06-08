use std::io::Write;

use hisui::yuv::YuvReader;

// 16x16 の I420 フレーム 1 枚分のバイト数 (Y: 16*16, U/V: 8*8 ずつ)
const WIDTH: usize = 16;
const HEIGHT: usize = 16;
const Y_SIZE: usize = WIDTH * HEIGHT;
const CHROMA_SIZE: usize = (WIDTH / 2) * (HEIGHT / 2);
const FRAME_SIZE: usize = Y_SIZE + CHROMA_SIZE * 2;

// 指定したフィラー値で 1 フレーム分の I420 バイト列を作る
// Y / U / V でフィラー値を変えてプレーン分割の正しさを確認できるようにする
fn make_frame(y_fill: u8, u_fill: u8, v_fill: u8) -> Vec<u8> {
    let mut data = Vec::new();
    data.extend(std::iter::repeat_n(y_fill, Y_SIZE));
    data.extend(std::iter::repeat_n(u_fill, CHROMA_SIZE));
    data.extend(std::iter::repeat_n(v_fill, CHROMA_SIZE));
    data
}

fn write_temp_file(bytes: &[u8]) -> tempfile::NamedTempFile {
    let mut file = tempfile::NamedTempFile::new().expect("一時ファイルを作成できる");
    file.write_all(bytes).expect("一時ファイルへ書き込める");
    file.flush().expect("一時ファイルをフラッシュできる");
    file
}

#[test]
fn read_multiple_frames_split_into_planes() {
    let mut bytes = Vec::new();
    bytes.extend(make_frame(10, 20, 30));
    bytes.extend(make_frame(40, 50, 60));
    let file = write_temp_file(&bytes);

    let mut reader = YuvReader::new(file.path(), WIDTH, HEIGHT).expect("リーダーを生成できる");

    let frame0 = reader
        .read_frame()
        .expect("1 フレーム目を読める")
        .expect("1 フレーム目が存在する");
    assert_eq!(frame0.y(), [10u8; Y_SIZE]);
    assert_eq!(frame0.u(), [20u8; CHROMA_SIZE]);
    assert_eq!(frame0.v(), [30u8; CHROMA_SIZE]);

    let frame1 = reader
        .read_frame()
        .expect("2 フレーム目を読める")
        .expect("2 フレーム目が存在する");
    assert_eq!(frame1.y(), [40u8; Y_SIZE]);
    assert_eq!(frame1.u(), [50u8; CHROMA_SIZE]);
    assert_eq!(frame1.v(), [60u8; CHROMA_SIZE]);

    assert!(
        reader.read_frame().expect("終端を読める").is_none(),
        "全フレーム読み込み後は None になる"
    );
}

#[test]
fn empty_file_reaches_eof_on_first_read() {
    let file = write_temp_file(&[]);
    let mut reader = YuvReader::new(file.path(), WIDTH, HEIGHT).expect("リーダーを生成できる");
    assert!(
        reader.read_frame().expect("終端を読める").is_none(),
        "空ファイルは即座に None になる"
    );
}

#[test]
fn trailing_bytes_below_frame_boundary_cause_error() {
    let mut bytes = make_frame(1, 2, 3);
    // フレーム 1 枚分 + 端数を書き込む
    bytes.extend(std::iter::repeat_n(0u8, FRAME_SIZE / 2));
    let file = write_temp_file(&bytes);

    let mut reader = YuvReader::new(file.path(), WIDTH, HEIGHT).expect("リーダーを生成できる");
    reader
        .read_frame()
        .expect("1 フレーム目を読める")
        .expect("1 フレーム目が存在する");
    assert!(
        reader.read_frame().is_err(),
        "端数バイトが残っているとエラーになる"
    );
}

#[test]
fn files_with_different_frame_counts_yield_different_frame_counts() {
    // 参照側 2 フレーム、劣化側 1 フレームのように、フレーム数が一致しない場合に
    // 呼び出し側 (VMAF 評価) が不一致を検出できることを担保する
    let reference = write_temp_file(&[make_frame(1, 1, 1), make_frame(2, 2, 2)].concat());
    let distorted = write_temp_file(&make_frame(1, 1, 1));

    let count = |file: &tempfile::NamedTempFile| {
        let mut reader = YuvReader::new(file.path(), WIDTH, HEIGHT).expect("リーダーを生成できる");
        let mut n = 0;
        while reader.read_frame().expect("フレームを読める").is_some() {
            n += 1;
        }
        n
    };

    assert_eq!(count(&reference), 2);
    assert_eq!(count(&distorted), 1);
}
