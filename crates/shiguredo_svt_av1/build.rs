use std::{
    fs::File,
    io::Read,
    path::{Path, PathBuf},
    process::Command,
};

use cmake::Config;
use sha2::Digest;

// 依存ライブラリの名前
const LIB_NAME: &str = "SVT-AV1";
const LINK_NAME: &str = "SvtAv1Enc";

fn main() {
    // Cargo.toml か build.rs が更新されたら、依存ライブラリを再ビルドする
    println!("cargo::rerun-if-changed=Cargo.toml");
    println!("cargo::rerun-if-changed=build.rs");

    // 各種変数やビルドディレクトリのセットアップ
    let out_dir = PathBuf::from(std::env::var_os("OUT_DIR").expect("infallible"));
    let out_build_dir = out_dir.join("build/");
    let output_metadata_path = out_dir.join("metadata.rs");
    let output_bindings_path = out_dir.join("bindings.rs");
    let _ = std::fs::remove_dir_all(&out_build_dir);
    std::fs::create_dir(&out_build_dir).expect("failed to create build directory");

    // 各種メタデータを書き込む
    let (git_url, version) = get_git_url_and_version();
    std::fs::write(
        output_metadata_path,
        format!(
            concat!(
                "pub const BUILD_METADATA_REPOSITORY: &str={:?};\n",
                "pub const BUILD_METADATA_VERSION: &str={:?};\n",
            ),
            git_url, version
        ),
    )
    .expect("failed to write metadata file");

    let src_dir = out_build_dir.join(format!("{LIB_NAME}-{version}"));
    let input_header_path = src_dir.join("Source/API/EbSvtAv1Enc.h");
    if std::env::var("DOCS_RS").is_ok() {
        // Docs.rs 向けのビルドでは curl ができないので build.rs の処理はスキップして、
        // 代わりに、ドキュメント生成時に最低限必要な定義だけをダミーで出力している。
        //
        // See also: https://docs.rs/about/builds
        std::fs::write(
            output_bindings_path,
            concat!(
                "pub struct EbErrorType;",
                "pub struct EbBufferHeaderType;",
                "pub struct EbSvtIOFormat;",
                "pub struct EbComponentType;",
            ),
        )
        .expect("write file error");
        return;
    }

    // 依存ライブラリを source URL から curl でダウンロードする
    download_external_lib(&out_build_dir, &version);

    // 依存ライブラリをビルドする
    let dst = Config::new(&src_dir)
        .define("BUILD_SHARED_LIBS", "OFF")
        .define("SVT_AV1_LTO", "OFF")
        .profile("Release")
        .build();

    // バインディングを生成する
    bindgen::Builder::default()
        .header(input_header_path.to_str().expect("invalid header path"))
        .generate()
        .expect("failed to generate bindings")
        .write_to_file(output_bindings_path)
        .expect("failed to write bindings");

    println!(
        "cargo::rustc-link-search=native={}",
        dst.join("lib").display()
    );
    println!("cargo::rustc-link-lib=static={LINK_NAME}");
}

// 外部ライブラリを source URL から curl でダウンロードして展開する
fn download_external_lib(build_dir: &Path, version: &str) {
    let source_url = get_source_url();
    let expected_sha256 = get_expected_sha256();

    // tar.gz ファイルをダウンロード
    let tar_gz_filename = format!("{LIB_NAME}-{version}.tar.gz");
    let tar_gz_path = build_dir.join(&tar_gz_filename);

    println!("Downloading {LIB_NAME} from {}", source_url);

    let success = Command::new("curl")
        .arg("-L")
        .arg("-o")
        .arg(&tar_gz_path)
        .arg(&source_url)
        .status()
        .is_ok_and(|status| status.success());

    if !success {
        panic!(
            "failed to download {LIB_NAME} from source URL: {}",
            source_url
        );
    }

    // ダウンロードしたファイルのハッシュを検証
    verify_sha256(&tar_gz_path, &expected_sha256).expect("SHA256 verification failed");

    // tar.gz を展開
    println!("Extracting {tar_gz_filename}");

    let success = Command::new("tar")
        .arg("-xzf")
        .arg(&tar_gz_path)
        .arg("-C")
        .arg(build_dir)
        .status()
        .is_ok_and(|status| status.success());

    if !success {
        panic!("failed to extract {LIB_NAME} archive");
    }

    // ダウンロードしたファイルを削除
    let _ = std::fs::remove_file(&tar_gz_path);

    println!("Successfully downloaded and extracted {LIB_NAME}");
}

// ファイルの SHA256 ハッシュを計算して検証
fn verify_sha256(file_path: &Path, expected_hash: &str) -> Result<(), String> {
    use std::fmt::Write as FmtWrite;

    println!("Verifying SHA256 hash for {}", file_path.display());

    let mut file = File::open(file_path).map_err(|e| format!("failed to open file: {}", e))?;

    // SHA256 を計算
    let mut hasher = sha2::Sha256::new();
    let mut buffer = [0; 8192];
    loop {
        let n = file
            .read(&mut buffer)
            .map_err(|e| format!("failed to read file: {}", e))?;
        if n == 0 {
            break;
        }
        use sha2::Digest;
        hasher.update(&buffer[..n]);
    }

    let result = hasher.finalize();
    let mut calculated_hash = String::new();
    for byte in result {
        write!(&mut calculated_hash, "{:02x}", byte)
            .map_err(|e| format!("formatting error: {}", e))?;
    }

    if calculated_hash.eq_ignore_ascii_case(expected_hash) {
        println!("=> SHA256 hash verified: {}", calculated_hash);
        Ok(())
    } else {
        Err(format!(
            "SHA256 hash mismatch!\nExpected: {}\nCalculated: {}",
            expected_hash, calculated_hash
        ))
    }
}

// Cargo.toml から依存ライブラリの Git URL とバージョンタグを取得する
fn get_git_url_and_version() -> (String, String) {
    let cargo_toml: toml::Value =
        toml::from_str(include_str!("Cargo.toml")).expect("failed to parse Cargo.toml");
    if let Some((Some(git_url), Some(version))) = cargo_toml
        .get("package")
        .and_then(|v| v.get("metadata"))
        .and_then(|v| v.get("external-dependencies"))
        .and_then(|v| v.get(LIB_NAME))
        .map(|v| {
            (
                v.get("git").and_then(|s| s.as_str()),
                v.get("version").and_then(|s| s.as_str()),
            )
        })
    {
        (git_url.to_string(), version.to_string())
    } else {
        panic!(
            "Cargo.toml does not contain a valid [package.metadata.external-dependencies.{LIB_NAME}] table"
        );
    }
}

// Cargo.toml から source URL を取得する
fn get_source_url() -> String {
    let cargo_toml: toml::Value =
        toml::from_str(include_str!("Cargo.toml")).expect("failed to parse Cargo.toml");
    if let Some(source_url) = cargo_toml
        .get("package")
        .and_then(|v| v.get("metadata"))
        .and_then(|v| v.get("external-dependencies"))
        .and_then(|v| v.get(LIB_NAME))
        .and_then(|v| v.get("source"))
        .and_then(|s| s.as_str())
    {
        source_url.to_string()
    } else {
        panic!("Cargo.toml does not contain a valid source URL for {LIB_NAME}");
    }
}

// Cargo.toml から期待される SHA256 ハッシュを取得する
fn get_expected_sha256() -> String {
    let cargo_toml: toml::Value =
        toml::from_str(include_str!("Cargo.toml")).expect("failed to parse Cargo.toml");
    if let Some(sha256) = cargo_toml
        .get("package")
        .and_then(|v| v.get("metadata"))
        .and_then(|v| v.get("external-dependencies"))
        .and_then(|v| v.get(LIB_NAME))
        .and_then(|v| v.get("sha256"))
        .and_then(|s| s.as_str())
    {
        sha256.to_string()
    } else {
        panic!("Cargo.toml does not contain a valid SHA256 hash for {LIB_NAME}");
    }
}
